//! Writing files so that a crash or a full disk never leaves half of one,
//! and keeping what was there before.

use std::io::Write;
use std::path::{Path, PathBuf};

/// How many backups of the configuration are kept.
const KEEP: usize = 100;

/// Write `bytes` to `path` so that the file is either the old one or the
/// new one, never a part: a temporary file in the same folder is written,
/// flushed to the disk and renamed over the target. A symbolic link is
/// followed, so a configuration kept elsewhere (dotfiles) stays a link.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let folder = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&folder)?;
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    // (A name of its own for each writing: two of one program at the
    // same moment, on two of its threads, would else write into one file
    // and rename each other's half.)
    static WRITINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let writing = WRITINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = folder.join(format!(
        ".{name}.gevrek-{}-{writing}",
        std::process::id()
    ));
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(existing) = std::fs::metadata(&target) {
            let _ = std::fs::set_permissions(&temporary, existing.permissions());
        }
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The folder of the backups.
pub fn backup_dir() -> PathBuf {
    crate::paths::state_dir().join("backups")
}

/// One kept version of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// Where the copy is.
    pub path: PathBuf,
    /// When it was made, in seconds since 1970.
    pub taken_at: u64,
    /// The file it is a copy of.
    pub origin: PathBuf,
}

/// Keep `content` (what `origin` held until now) as a backup, and drop the
/// oldest ones beyond the newest hundred.
pub fn backup(origin: &Path, content: &str) -> std::io::Result<Backup> {
    backup_in(&backup_dir(), origin, content)
}

fn backup_in(folder: &Path, origin: &Path, content: &str) -> std::io::Result<Backup> {
    std::fs::create_dir_all(folder)?;
    let taken_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let name = origin
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.json".to_string());
    // Two saves in one second get different names.
    let mut path = folder.join(format!("{taken_at}-{name}"));
    let mut counter = 1;
    while path.exists() {
        path = folder.join(format!("{taken_at}.{counter}-{name}"));
        counter += 1;
    }
    std::fs::write(&path, content)?;
    let absolute = std::fs::canonicalize(origin).unwrap_or_else(|_| origin.to_path_buf());
    std::fs::write(sidecar(&path), absolute.to_string_lossy().as_bytes())?;
    let mut all = list_in(folder);
    for old in all.drain(KEEP.min(all.len())..) {
        let _ = std::fs::remove_file(sidecar(&old.path));
        let _ = std::fs::remove_file(&old.path);
    }
    Ok(Backup {
        path,
        taken_at,
        origin: absolute,
    })
}

fn sidecar(backup: &Path) -> PathBuf {
    let mut name = backup.as_os_str().to_os_string();
    name.push(".origin");
    PathBuf::from(name)
}

/// The backups, newest first.
pub fn list_backups() -> Vec<Backup> {
    list_in(&backup_dir())
}

fn list_in(folder: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut backups: Vec<(String, Backup)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name.ends_with(".origin") {
                return None;
            }
            let stamp = name.split(['-', '.']).next()?;
            let taken_at = stamp.parse::<u64>().ok()?;
            let origin = std::fs::read_to_string(sidecar(&path))
                .map(PathBuf::from)
                .unwrap_or_default();
            Some((
                name,
                Backup {
                    path,
                    taken_at,
                    origin,
                },
            ))
        })
        .collect();
    // Newest first; within one second by name, which carries the counter.
    backups.sort_by(|a, b| b.1.taken_at.cmp(&a.1.taken_at).then_with(|| b.0.cmp(&a.0)));
    backups.into_iter().map(|(_, backup)| backup).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-files-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_nothing_behind() {
        let folder = scratch("atomic");
        let path = folder.join("config.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_atomic_write_keeps_a_symbolic_link_a_link() {
        let folder = scratch("link");
        let real = folder.join("real.json");
        let link = folder.join("config.json");
        std::fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn backups_are_listed_newest_first_and_know_their_origin() {
        let folder = scratch("backups");
        let origin = folder.join("config.json");
        std::fs::write(&origin, "current").unwrap();
        let store = folder.join("store");
        let first = backup_in(&store, &origin, "first").unwrap();
        let second = backup_in(&store, &origin, "second").unwrap();
        assert_ne!(first.path, second.path);
        let listed = list_in(&store);
        assert_eq!(listed.len(), 2);
        assert_eq!(std::fs::read_to_string(&listed[0].path).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(&listed[1].path).unwrap(), "first");
        assert_eq!(listed[0].origin, std::fs::canonicalize(&origin).unwrap());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
