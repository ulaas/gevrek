//! Where gevrek keeps its files, on every system, for every program.
//!
//! | | Linux | Windows |
//! |---|---|---|
//! | configuration | `$XDG_CONFIG_HOME/gevrek` | `%APPDATA%\\gevrek` |
//! | state (backups, history, session) | `$XDG_STATE_HOME/gevrek` | `%LOCALAPPDATA%\\gevrek\\state` |
//! | cache (icons, pictures) | `$XDG_CACHE_HOME/gevrek` | `%LOCALAPPDATA%\\gevrek\\cache` |
//! | runtime (sockets, pid files) | `$XDG_RUNTIME_DIR/gevrek` | `%LOCALAPPDATA%\\gevrek\\run` |
//!
//! `GEVREK_INSTANCE` names another instance than the session's own (a
//! nested one for development, a sandbox): it gets its own folders, so two
//! gevreks on one machine never share a file. `GEVREK_CONFIG_DIR`,
//! `GEVREK_STATE_DIR`, `GEVREK_CACHE_DIR` and `GEVREK_RUNTIME_DIR` set a
//! folder outright (tests use them).
//!
//! Nothing here creates a folder: the code that writes does.

use std::path::PathBuf;

/// The instance the session itself runs as.
pub const MAIN_INSTANCE: &str = "main";

/// The name of this instance: `GEVREK_INSTANCE`, or "main". Only letters,
/// digits, `-` and `_` are kept, so the name is safe in a file name.
pub fn instance() -> String {
    instance_from(std::env::var("GEVREK_INSTANCE").ok().as_deref())
}

fn instance_from(value: Option<&str>) -> String {
    let name: String = value
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(48)
        .collect();
    if name.is_empty() {
        MAIN_INSTANCE.to_string()
    } else {
        name
    }
}

/// The name of the panel's surfaces (`gevrek-shell`) of an instance:
/// `gevrek-panel`, with the instance after a dot when it is not the main
/// one. On Windows the name is the end of the surfaces' window class, and
/// the window manager looks for it to know that its own panel runs: a
/// second gevrek on the machine (the desktop checks) must not take the
/// first one's panel for its own.
pub fn panel_surface(instance: &str) -> String {
    if instance == MAIN_INSTANCE {
        "gevrek-panel".to_string()
    } else {
        format!("gevrek-panel.{instance}")
    }
}

/// `base/gevrek`, or `base/gevrek/instances/<name>` for another instance
/// than the main one.
fn scoped(base: PathBuf, instance: &str) -> PathBuf {
    let root = base.join("gevrek");
    if instance == MAIN_INSTANCE {
        root
    } else {
        root.join("instances").join(instance)
    }
}

fn overridden(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn fallback() -> PathBuf {
    std::env::temp_dir()
}

/// The folder of the configuration.
pub fn config_dir() -> PathBuf {
    overridden("GEVREK_CONFIG_DIR")
        .unwrap_or_else(|| scoped(dirs::config_dir().unwrap_or_else(fallback), &instance()))
}

/// The configuration file programs use when none is named with `--config`.
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// What should outlive a restart and is not configuration: backups of the
/// configuration, history, the saved session.
pub fn state_dir() -> PathBuf {
    overridden("GEVREK_STATE_DIR").unwrap_or_else(|| {
        // `dirs::state_dir` is Linux only; elsewhere a folder beside the
        // local data.
        match dirs::state_dir() {
            Some(base) => scoped(base, &instance()),
            None => {
                scoped(dirs::data_local_dir().unwrap_or_else(fallback), &instance()).join("state")
            }
        }
    })
}

/// What can be made again: icons, scaled pictures.
pub fn cache_dir() -> PathBuf {
    overridden("GEVREK_CACHE_DIR").unwrap_or_else(|| {
        if cfg!(windows) {
            scoped(dirs::data_local_dir().unwrap_or_else(fallback), &instance()).join("cache")
        } else {
            scoped(dirs::cache_dir().unwrap_or_else(fallback), &instance())
        }
    })
}

/// What lives only while the session does: sockets, pid files. All
/// instances share it; their files carry the instance's name.
pub fn runtime_dir() -> PathBuf {
    overridden("GEVREK_RUNTIME_DIR").unwrap_or_else(|| match dirs::runtime_dir() {
        Some(base) => base.join("gevrek"),
        None => dirs::data_local_dir()
            .unwrap_or_else(fallback)
            .join("gevrek")
            .join("run"),
    })
}

/// The file a running program of this instance is known by:
/// `<runtime>/<instance>.<component>.<pid>.claim`.
fn claim_file(instance: &str, component: &str, pid: u32) -> PathBuf {
    runtime_dir().join(format!("{instance}.{component}.{pid}.claim"))
}

/// Say that this process is `component` (`wm`, `editor`, `overlay`) of this
/// instance. Programs are told apart by their file name otherwise, and
/// two instances on one machine (the gevrek in use and one being worked
/// on) run programs of the same name: `gevrek stop` of the one must not
/// end the other's. The file is empty; it is the name that says it.
/// Failing to write it is not an error (the program runs all the same).
pub fn claim_process(component: &str) {
    let file = claim_file(&instance(), component, std::process::id());
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(file, b"");
}

/// The processes that said they are `component`, as (instance, pid, the
/// file that says so). A process that ended without removing its file is
/// still listed: whoever reads this checks that the process is there.
pub fn claimed_processes(component: &str) -> Vec<(String, u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(runtime_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (instance, pid) = parse_claim(&name, component)?;
            Some((instance, pid, entry.path()))
        })
        .collect()
}

/// `dev.wm.4242.claim` for the component `wm`: `("dev", 4242)`.
fn parse_claim(file_name: &str, component: &str) -> Option<(String, u32)> {
    let mut parts = file_name.split('.');
    let instance = parts.next()?;
    let found = parts.next()?;
    let pid = parts.next()?.parse().ok()?;
    (found == component && parts.next() == Some("claim") && parts.next().is_none())
        .then(|| (instance.to_string(), pid))
}

/// The configuration file to use: the one named, else the standard one.
/// A `config.json` in the working directory was the default before there
/// was a standard place: it is still used when it is there and the standard
/// file is not, and the second value says so, for a notice.
///
/// Not when the working directory is the user's home: that is where a
/// login session and a program started from a menu run, and a
/// `config.json` lying there is some other program's, which gevrek would
/// read as its own and later write over.
pub fn resolve_config(named: Option<PathBuf>) -> (PathBuf, bool) {
    if let Some(path) = named {
        return (path, false);
    }
    let standard = config_file();
    let legacy = PathBuf::from("config.json");
    let here = std::env::current_dir().ok();
    if !standard.exists()
        && instance() == MAIN_INSTANCE
        && working_directory_counts(here.as_deref(), dirs::home_dir().as_deref())
        && legacy.is_file()
    {
        return (legacy, true);
    }
    (standard, false)
}

/// Whether a `config.json` in the working directory may be gevrek's: the
/// directory is known and is not the home directory.
fn working_directory_counts(
    here: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> bool {
    let Some(here) = here else {
        return false;
    };
    let same = |one: &std::path::Path, other: &std::path::Path| {
        one == other
            || matches!((one.canonicalize(), other.canonicalize()), (Ok(one), Ok(other)) if one == other)
    };
    !home.is_some_and(|home| same(here, home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_instance_has_a_panel_name_of_its_own() {
        assert_eq!(panel_surface(MAIN_INSTANCE), "gevrek-panel");
        assert_eq!(panel_surface("checks"), "gevrek-panel.checks");
        assert_ne!(panel_surface("a"), panel_surface("b"));
    }

    #[test]
    fn a_config_json_in_the_home_directory_is_not_ours() {
        let home = PathBuf::from("/home/someone");
        assert!(!working_directory_counts(Some(&home), Some(&home)));
        assert!(working_directory_counts(
            Some(&home.join("source/gevrek")),
            Some(&home)
        ));
        // Nowhere known: nothing is taken from there.
        assert!(!working_directory_counts(None, Some(&home)));
        // No home known: the working directory is as good as any.
        assert!(working_directory_counts(Some(&home), None));
    }

    #[test]
    fn a_claim_file_names_its_instance_component_and_process() {
        assert_eq!(
            parse_claim("dev.wm.4242.claim", "wm"),
            Some(("dev".to_string(), 4242))
        );
        assert_eq!(
            parse_claim("sandbox-7_a.overlay.9.claim", "overlay"),
            Some(("sandbox-7_a".to_string(), 9))
        );
        // Another component, a pid file, a log, something else entirely.
        assert_eq!(parse_claim("dev.wm.4242.claim", "editor"), None);
        assert_eq!(parse_claim("main.wm.pid", "wm"), None);
        assert_eq!(parse_claim("main.wm.log", "wm"), None);
        assert_eq!(parse_claim("main.wm.x.claim", "wm"), None);
        assert_eq!(parse_claim("main.wm.1.claim.tmp", "wm"), None);
    }

    #[test]
    fn an_instance_name_is_safe_for_a_file_name() {
        assert_eq!(instance_from(None), "main");
        assert_eq!(instance_from(Some("")), "main");
        assert_eq!(instance_from(Some("dev")), "dev");
        assert_eq!(instance_from(Some("sandbox-4242")), "sandbox-4242");
        assert_eq!(instance_from(Some("../../etc")), "etc");
        assert_eq!(instance_from(Some("a b/c")), "abc");
    }

    #[test]
    fn another_instance_has_its_own_folders() {
        let base = PathBuf::from("/base");
        assert_eq!(scoped(base.clone(), "main"), PathBuf::from("/base/gevrek"));
        assert_eq!(
            scoped(base, "dev"),
            PathBuf::from("/base/gevrek/instances/dev")
        );
    }
}
