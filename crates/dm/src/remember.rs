//! What the login remembers of the person who logged in last, so that it
//! looks and types as their desktop does: the wallpaper, the accent, and
//! the keyboard's layouts.
//!
//! The greeter has nobody's configuration and may not read anybody's
//! home. So the worker, when a person has logged in, reads that person's
//! configuration and wallpaper *as that person* (a child with their
//! rights does the reading: root never opens a file because a person's
//! configuration names it), and puts copies into a folder of the
//! greeter's own. The service hands them to the greeter the next time it
//! starts one.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// The folder in the greeter's home the copies are kept in.
pub const FOLDER: &str = "remembered";
/// A configuration larger than this is not read.
const CONFIG_MOST: u64 = 8 * 1024 * 1024;
/// A wallpaper larger than this is not copied.
const PICTURE_MOST: u64 = 64 * 1024 * 1024;
/// The kinds of picture the login can show.
const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// What a configuration says that the login takes over.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Taken {
    /// `XKB_DEFAULT_*`, as the greeter's compositor is to have them.
    pub keyboard: Vec<(String, String)>,
    /// The wallpaper's file, with `~` already made the person's home.
    pub wallpaper: Option<PathBuf>,
    /// The accent as `rrggbb`, or `wallpaper` where it is the picture's.
    pub accent: String,
}

/// What this configuration says for the login.
pub fn taken(config: &gevrek_model::Config, home: &Path) -> Taken {
    let desktop = &config.desktop;
    let keys = &desktop.input.keyboard;
    let (layouts, variants) = keys.xkb_layouts();
    let mut keyboard = Vec::new();
    if !layouts.trim_matches(',').is_empty() {
        keyboard.push(("XKB_DEFAULT_LAYOUT".to_string(), layouts));
        if !variants.trim_matches(',').is_empty() {
            keyboard.push(("XKB_DEFAULT_VARIANT".to_string(), variants));
        }
        if !keys.model.trim().is_empty() {
            keyboard.push((
                "XKB_DEFAULT_MODEL".to_string(),
                keys.model.trim().to_string(),
            ));
        }
        // (With the keys that switch the layout, as in the session.)
        let options = keys.xkb_options().join(",");
        if !options.is_empty() {
            keyboard.push(("XKB_DEFAULT_OPTIONS".to_string(), options));
        }
    }
    // The picture of every display that has none of its own. (`~` is the
    // person's home, not that of whoever reads this.)
    let image = desktop.wallpaper.image.trim();
    let wallpaper = (!image.is_empty()).then(|| match image.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(image),
    });
    let accent = if gevrek_model::theme::follows_wallpaper(&desktop.appearance.theme) {
        "wallpaper".to_string()
    } else {
        let (red, green, blue) = desktop.appearance.active_border_color;
        format!("{red:02x}{green:02x}{blue:02x}")
    };
    Taken {
        keyboard,
        wallpaper,
        accent,
    }
}

/// How long a file of a person's is waited for.
const READ_FOR: std::time::Duration = std::time::Duration::from_secs(4);

/// Who a file is read as.
pub struct Reader<'a> {
    pub name: &'a str,
    pub uid: u32,
    pub gid: u32,
}

/// Read a file as that person does: a child with their rights reads it,
/// and no more than `most` bytes of it are taken. Nothing where it is not
/// theirs to read, is no ordinary file, or is larger, and nothing where
/// it does not come within `READ_FOR`. (It was waited for without an end:
/// a configuration that was a pipe nobody wrote to, or lay on a mount
/// that did not answer, kept the worker from ever ending, and with it
/// every later login.)
pub fn read_as(reader: &Reader, file: &Path, most: u64) -> Option<Vec<u8>> {
    let name = std::ffi::CString::new(reader.name).ok()?;
    let (uid, gid) = (reader.uid, reader.gid);
    let mut command = std::process::Command::new("/bin/cat");
    command
        .arg("--")
        .arg(file)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    // SAFETY: only calls that are safe between fork and exec; the groups
    // first, then the identity (see the worker's own child).
    unsafe {
        command.pre_exec(move || {
            if libc::initgroups(name.as_ptr(), gid) != 0
                || libc::setgid(gid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let out = child.stdout.take()?;
    // Read on a thread of its own, so that it can be given up on: the
    // child killed, its end of the pipe closes and the thread ends.
    let (done, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = out.take(most + 1).read_to_end(&mut bytes);
        let _ = done.send((read.is_ok(), bytes));
    });
    let answer = read.recv_timeout(READ_FOR);
    // (Whatever it was still writing: it is not waited for with a full pipe.)
    let _ = child.kill();
    let ended = child.wait().ok()?;
    let Ok((read, bytes)) = answer else {
        log::warn!("{} was not read in time", file.display());
        return None;
    };
    let whole = read && bytes.len() as u64 <= most;
    // Killed by us after everything was read is fine; an error of its
    // own (no such file, not allowed) is not.
    (whole && !bytes.is_empty() && ended.code().is_none_or(|code| code == 0)).then_some(bytes)
}

/// The ending of a picture's name, where the login can show such a picture.
pub fn picture_ending(file: &Path) -> Option<String> {
    let ending = file.extension()?.to_str()?.to_lowercase();
    PICTURES.contains(&ending.as_str()).then_some(ending)
}

/// Whether a path is a folder itself, and no link to one.
fn is_a_folder_of_its_own(folder: &Path) -> bool {
    std::fs::symlink_metadata(folder).is_ok_and(|folder| folder.is_dir())
}

/// Keep what the person's configuration says for the login, in the
/// greeter's folder (which is the greeter's: `owner`). What cannot be
/// read is left as it was kept before.
pub fn keep(reader: &Reader, home: &Path, greeter_home: &Path, owner: (u32, u32)) {
    let Some(written) = read_as(
        reader,
        &home.join(".config").join("gevrek").join("config.json"),
        CONFIG_MOST,
    ) else {
        log::debug!(
            "the login keeps what it had of {}: their configuration was not read",
            reader.name
        );
        return;
    };
    let config = match serde_json::from_slice::<gevrek_model::Config>(&written) {
        Ok(config) => config,
        Err(error) => {
            log::debug!(
                "the login keeps what it had of {}: their configuration is not one: {error}",
                reader.name
            );
            return;
        }
    };
    let taken = taken(&config, home);
    let folder = greeter_home.join(FOLDER);
    if let Err(error) = std::fs::create_dir_all(&folder) {
        log::debug!(
            "the login keeps nothing of {}: {}: {error}",
            reader.name,
            folder.display()
        );
        return;
    }
    // The folder itself is to be a folder, and no link to one: it lies in
    // the greeter's account's home, and what is written into it is written
    // as root. (Only the files' own names were looked at: a link put where
    // the folder is would have had root make the greeter's files wherever
    // it pointed.)
    if !is_a_folder_of_its_own(&folder) {
        log::warn!(
            "the login keeps nothing of {}: {} is no folder of its own",
            reader.name,
            folder.display()
        );
        return;
    }
    let put = |name: &str, bytes: &[u8]| {
        let file = folder.join(name);
        // (Written beside and moved over: the greeter never reads half a file.)
        let beside = folder.join(format!(".{name}.new"));
        // The folder is the greeter's account's, and this runs as root:
        // a link put where the file is written would have root write
        // through it, and hand what it points at to the greeter. So what
        // lies there is taken away (the link, not what it points at), the
        // file is made anew and only if nothing is there, and it is the
        // file that was made that changes hands, not whatever has its
        // name by then.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(&beside);
        let made = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o644)
            .open(&beside);
        let mut made = match made {
            Ok(made) => made,
            Err(error) => {
                log::debug!("{} was not written: {error}", beside.display());
                return;
            }
        };
        match made
            .write_all(bytes)
            .and_then(|()| std::os::unix::fs::fchown(&made, Some(owner.0), Some(owner.1)))
        {
            Ok(()) => {
                drop(made);
                if let Err(error) = std::fs::rename(&beside, &file) {
                    log::debug!("{} was not put in place: {error}", file.display());
                }
            }
            Err(error) => {
                drop(made);
                log::debug!("{} was not written: {error}", beside.display());
                let _ = std::fs::remove_file(&beside);
            }
        }
    };
    let keyboard: String = taken
        .keyboard
        .iter()
        .map(|(name, value)| format!("{name}={value}\n"))
        .collect();
    put("keyboard", keyboard.as_bytes());
    put("accent", taken.accent.as_bytes());
    // The wallpaper: the one picture kept is this person's, or none.
    let picture = taken.wallpaper.as_deref().and_then(|file| {
        let ending = picture_ending(file)?;
        Some((ending, read_as(reader, file, PICTURE_MOST)?))
    });
    for ending in PICTURES {
        let _ = std::fs::remove_file(folder.join(format!("wallpaper.{ending}")));
    }
    if let Some((ending, bytes)) = picture {
        put(&format!("wallpaper.{ending}"), &bytes);
    }
    log::debug!(
        "the login keeps the keyboard, the accent and {} of {}",
        if picture_kept(&folder) {
            "the wallpaper"
        } else {
            "no wallpaper"
        },
        reader.name
    );
    // (The folder itself, not what a link in its place points at.)
    let _ = std::os::unix::fs::lchown(&folder, Some(owner.0), Some(owner.1));
}

/// Whether a wallpaper is kept in the folder.
fn picture_kept(folder: &Path) -> bool {
    PICTURES
        .iter()
        .any(|ending| folder.join(format!("wallpaper.{ending}")).exists())
}

/// What was kept, as the environment of the greeter: the keyboard (in
/// place of the system's own layout), the wallpaper and the accent for
/// the login's screen. Empty where nobody has logged in yet.
pub fn environment(greeter_home: &Path) -> Vec<(String, String)> {
    let folder = greeter_home.join(FOLDER);
    let mut environment =
        keyboard_kept(&std::fs::read_to_string(folder.join("keyboard")).unwrap_or_default());
    if let Some(picture) = PICTURES
        .iter()
        .map(|ending| folder.join(format!("wallpaper.{ending}")))
        .find(|file| file.is_file())
    {
        environment.push((
            "GEVREK_LOCK_WALLPAPER".to_string(),
            picture.display().to_string(),
        ));
    }
    let accent = std::fs::read_to_string(folder.join("accent")).unwrap_or_default();
    let accent = accent.trim();
    if accent == "wallpaper"
        || (accent.len() == 6 && accent.chars().all(|letter| letter.is_ascii_hexdigit()))
    {
        environment.push(("GEVREK_LOCK_ACCENT".to_string(), accent.to_string()));
    }
    environment
}

/// The keyboard as it was kept: only the names that are the keyboard's,
/// and only values a layout's name can be made of.
pub fn keyboard_kept(kept: &str) -> Vec<(String, String)> {
    kept.lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(name, value)| {
            matches!(
                *name,
                "XKB_DEFAULT_LAYOUT"
                    | "XKB_DEFAULT_VARIANT"
                    | "XKB_DEFAULT_MODEL"
                    | "XKB_DEFAULT_OPTIONS"
            ) && !value.is_empty()
                && value.len() <= 256
                && value.chars().all(|letter| {
                    letter.is_ascii_alphanumeric()
                        || matches!(letter, ',' | '_' | '-' | ':' | '+' | '(' | ')' | '.')
                })
        })
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_takes_the_keyboard_the_wallpaper_and_the_accent() {
        let config: gevrek_model::Config = serde_json::from_value(serde_json::json!({
            "desktop": {
                "input": {"keyboard": {"layouts": [{"layout": "us"}, {"layout": "tr", "variant": "f"}]}},
                "wallpaper": {"image": "~/Pictures/hills.jpg"},
                "appearance": {"active_border_color": [0, 170, 255]}
            }
        }))
        .expect("a configuration");
        let taken = taken(&config, Path::new("/home/ada"));
        assert_eq!(
            taken.keyboard,
            vec![
                ("XKB_DEFAULT_LAYOUT".to_string(), "us,tr".to_string()),
                ("XKB_DEFAULT_VARIANT".to_string(), ",f".to_string()),
                // (Several layouts: with the keys that switch them.)
                (
                    "XKB_DEFAULT_OPTIONS".to_string(),
                    "grp:alt_shift_toggle".to_string()
                ),
            ]
        );
        assert_eq!(
            taken.wallpaper.as_deref(),
            Some(Path::new("/home/ada/Pictures/hills.jpg"))
        );
        assert_eq!(taken.accent, "00aaff");
        // Nothing named: nothing taken, and the system's layout stays.
        let plain = super::taken(&gevrek_model::Config::default(), Path::new("/home/ada"));
        assert!(plain.keyboard.is_empty());
        assert_eq!(plain.wallpaper, None);
    }

    #[test]
    fn what_was_kept_is_read_back_carefully() {
        let kept = "XKB_DEFAULT_LAYOUT=us,tr\nXKB_DEFAULT_OPTIONS=grp:alt_shift_toggle\nPATH=/tmp/evil\nXKB_DEFAULT_MODEL=$(reboot)\nXKB_DEFAULT_VARIANT=\n";
        assert_eq!(
            keyboard_kept(kept),
            vec![
                ("XKB_DEFAULT_LAYOUT".to_string(), "us,tr".to_string()),
                (
                    "XKB_DEFAULT_OPTIONS".to_string(),
                    "grp:alt_shift_toggle".to_string()
                ),
            ]
        );
        assert_eq!(
            picture_ending(Path::new("/a/b.JPG")).as_deref(),
            Some("jpg")
        );
        assert_eq!(picture_ending(Path::new("/etc/shadow")), None);
        assert_eq!(picture_ending(Path::new("/a/b.svg")), None);
    }

    #[test]
    fn nothing_is_written_through_a_folder_that_is_a_link() {
        let root = std::env::temp_dir().join(format!("gevrek-dm-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(&real).expect("a folder");
        assert!(is_a_folder_of_its_own(&real));
        // A link to a folder is not one, wherever it points; nor is what
        // is not there.
        let link = root.join("remembered");
        std::os::unix::fs::symlink(&real, &link).expect("a link");
        assert!(!is_a_folder_of_its_own(&link));
        assert!(!is_a_folder_of_its_own(&root.join("none")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_read_with_the_readers_own_rights_and_not_beyond_a_size() {
        // (As whoever runs the tests: the rights are their own.)
        let folder =
            std::env::temp_dir().join(format!("gevrek-dm-remember-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let file = folder.join("words");
        std::fs::write(&file, b"0123456789").expect("a file");
        // SAFETY: asks who runs this.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        let name =
            users::get_user_by_uid(uid).map(|user| user.name().to_string_lossy().to_string());
        let Some(name) = name else { return };
        let reader = Reader {
            name: &name,
            uid,
            gid,
        };
        // (Without root, setting one's own groups again is refused: the
        // reading itself is what is looked at here, as root in the machine.)
        if uid == 0 {
            assert_eq!(
                read_as(&reader, &file, 100).as_deref(),
                Some(&b"0123456789"[..])
            );
            assert_eq!(read_as(&reader, &file, 4), None, "larger than allowed");
            assert_eq!(read_as(&reader, &folder.join("none"), 100), None);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
