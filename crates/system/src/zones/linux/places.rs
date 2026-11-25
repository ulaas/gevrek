//! The folders the freedesktop specifications name: the person's home
//! and the base directories (configuration, data, cache, state), and the
//! desktop folder among the user directories.
//!
//! Everything else here is told its folders through a `Places`, and asks
//! the environment for none itself: a test builds one under a folder of
//! its own and never touches the real home.

use std::path::{Path, PathBuf};

/// Where things are for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (`~/.config`).
    pub config_home: PathBuf,
    /// `$XDG_CONFIG_DIRS` (`/etc/xdg`), the most important first.
    pub config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME` (`~/.local/share`).
    pub data_home: PathBuf,
    /// `$XDG_DATA_DIRS`, the most important first, without the home one.
    pub data_dirs: Vec<PathBuf>,
    /// `$XDG_CACHE_HOME` (`~/.cache`).
    pub cache_home: PathBuf,
    /// `$XDG_STATE_HOME` (`~/.local/state`).
    pub state_home: PathBuf,
    /// The names this desktop has in `XDG_CURRENT_DESKTOP`, in small
    /// letters: whose `<name>-mimeapps.list` is read before the plain one.
    pub desktops: Vec<String>,
}

impl Places {
    /// The places of the session this program runs in, by the Base
    /// Directory specification: a variable that is not set, is empty or is
    /// not an absolute path counts as not there.
    pub fn of_session() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let named = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let list = |name: &str, otherwise: &str| -> Vec<PathBuf> {
            let said = std::env::var(name).unwrap_or_default();
            let said = if said.is_empty() {
                otherwise.to_string()
            } else {
                said
            };
            said.split(':')
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .collect()
        };
        let mut desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|name| !name.is_empty())
            .map(str::to_lowercase)
            .collect();
        if desktops.is_empty() {
            desktops.push(crate::apps::DESKTOP.to_string());
        }
        Self {
            config_home: named("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            config_dirs: list("XDG_CONFIG_DIRS", "/etc/xdg"),
            data_home: named("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            data_dirs: list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            cache_home: named("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")),
            state_home: named("XDG_STATE_HOME").unwrap_or_else(|| home.join(".local/state")),
            desktops,
            home,
        }
    }

    /// Places that are all below `root`, laid out as a home is: for a
    /// test, and for trying things without touching the person's own.
    pub fn under(root: &Path) -> Self {
        let home = root.join("home");
        Self {
            config_home: home.join(".config"),
            config_dirs: vec![root.join("etc/xdg")],
            data_home: home.join(".local/share"),
            data_dirs: vec![root.join("usr/local/share"), root.join("usr/share")],
            cache_home: home.join(".cache"),
            state_home: home.join(".local/state"),
            desktops: vec![crate::apps::DESKTOP.to_string()],
            home,
        }
    }

    /// Every data directory, the person's own first.
    pub fn all_data_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.data_home.clone()];
        for dir in &self.data_dirs {
            if !dirs.contains(dir) {
                dirs.push(dir.clone());
            }
        }
        dirs
    }

    /// The desktop folder: `XDG_DESKTOP_DIR` of `user-dirs.dirs`, and
    /// `~/Desktop` where the file is not there or does not say.
    pub fn desktop(&self) -> PathBuf {
        std::fs::read_to_string(self.config_home.join("user-dirs.dirs"))
            .ok()
            .and_then(|text| user_dir(&text, "XDG_DESKTOP_DIR", &self.home))
            .unwrap_or_else(|| self.home.join("Desktop"))
    }

    /// The folder downloads go to: `XDG_DOWNLOAD_DIR` of
    /// `user-dirs.dirs`, and `~/Downloads` where the file is not there or
    /// does not say.
    pub fn downloads(&self) -> PathBuf {
        std::fs::read_to_string(self.config_home.join("user-dirs.dirs"))
            .ok()
            .and_then(|text| user_dir(&text, "XDG_DOWNLOAD_DIR", &self.home))
            .unwrap_or_else(|| self.home.join("Downloads"))
    }

    /// Where gevrek keeps what it remembers of launchers the person
    /// allowed to be started.
    pub fn trusted_launchers(&self) -> PathBuf {
        self.state_home.join("gevrek/zones/trusted-launchers")
    }
}

/// One of the user directories as `user-dirs.dirs` gives it. The file is
/// written to be read by a shell, but only two forms are allowed, and
/// only those are read: `XDG_X_DIR="$HOME/relative"` and
/// `XDG_X_DIR="/absolute"`. Anything else (another variable, no quotes, a
/// relative path) is not an answer. The last line that says it counts, as
/// it would for a shell.
pub fn user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let Some(quoted) = value
            .trim()
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        else {
            continue;
        };
        // Inside the quotes a backslash takes the next letter as it is.
        let mut plain = String::with_capacity(quoted.len());
        let mut letters = quoted.chars();
        while let Some(letter) = letters.next() {
            match letter {
                '\\' => plain.extend(letters.next()),
                other => plain.push(other),
            }
        }
        let path = if let Some(relative) = plain.strip_prefix("$HOME") {
            if !relative.is_empty() && !relative.starts_with('/') {
                // `$HOMEWORK` is another variable.
                continue;
            }
            home.join(relative.trim_start_matches('/'))
        } else if plain.starts_with('/') {
            PathBuf::from(plain)
        } else {
            continue;
        };
        found = Some(path);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_directory_is_read_in_its_two_forms_only() {
        let home = Path::new("/home/ada");
        let read = |text: &str| user_dir(text, "XDG_DESKTOP_DIR", home);
        assert_eq!(
            read("XDG_DESKTOP_DIR=\"$HOME/Masaüstü\"\n"),
            Some(PathBuf::from("/home/ada/Masaüstü"))
        );
        assert_eq!(
            read("# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"/mnt/desk top\"\n"),
            Some(PathBuf::from("/mnt/desk top"))
        );
        // The home itself: desktops are switched off this way.
        assert_eq!(
            read("XDG_DESKTOP_DIR=\"$HOME/\""),
            Some(PathBuf::from("/home/ada"))
        );
        assert_eq!(
            read("XDG_DESKTOP_DIR=\"$HOME/a \\\"quoted\\\" name\""),
            Some(PathBuf::from("/home/ada/a \"quoted\" name"))
        );
        // The last one counts.
        assert_eq!(
            read("XDG_DESKTOP_DIR=\"$HOME/One\"\nXDG_DESKTOP_DIR=\"$HOME/Two\"\n"),
            Some(PathBuf::from("/home/ada/Two"))
        );
        for not_an_answer in [
            "XDG_DESKTOP_DIR=$HOME/Desktop",
            "XDG_DESKTOP_DIR=\"Desktop\"",
            "XDG_DESKTOP_DIR=\"$HOMEWORK/Desktop\"",
            "XDG_DESKTOP_DIR=\"$OTHER/Desktop\"",
            "#XDG_DESKTOP_DIR=\"$HOME/Desktop\"",
            "",
        ] {
            assert_eq!(read(not_an_answer), None, "{not_an_answer}");
        }
    }

    #[test]
    fn the_desktop_is_what_the_file_says_and_else_the_usual_folder() {
        let root = crate::zones::linux::testing::folder("places");
        let places = Places::under(&root);
        assert_eq!(places.desktop(), places.home.join("Desktop"));
        std::fs::create_dir_all(&places.config_home).expect("a folder");
        std::fs::write(
            places.config_home.join("user-dirs.dirs"),
            "XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n",
        )
        .expect("written");
        assert_eq!(places.desktop(), places.home.join("Schreibtisch"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
