//! How much every gevrek program writes to its log.
//!
//! Each program has a level of its own for everything (`warn` for the
//! shell, `info` for the window manager, …). Over that, gevrek's own
//! debug lines (every module whose path begins with `gevrek`) are written
//! too where [`debug_lines`] says so:
//!
//! 1. `RUST_LOG`, where it is set, says alone: nothing is added to it.
//! 2. The setting `desktop.debug_log` of the configuration: `on` / `off`.
//! 3. The build: `GEVREK_DEBUG_LOG=1` (or `on`) when it was built writes
//!    them, `0` (or `off`) does not.
//! 4. The version: before 1.0 (and in a pre-release such as `1.2.0-rc.1`)
//!    they are written; from 1.0 on, not.
//!
//! Where they are written, a panic is written to the log too, with where
//! it happened and the stack that led to it.
//!
//! What a debug line is for: what happened at the edge of a part (a
//! request came, an answer went, a program connected or left, a file was
//! read or written, a state changed), not what is done each frame or for
//! each move of the pointer: those are `trace`.

use gevrek_model::settings::DebugLog;
use log::LevelFilter;
use std::path::{Path, PathBuf};

/// What the build said with `GEVREK_DEBUG_LOG`: `Some(true)` on,
/// `Some(false)` off, `None` not said.
pub fn build_says() -> Option<bool> {
    said(option_env!("GEVREK_DEBUG_LOG"))
}

fn said(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "yes" | "true" => Some(true),
        "0" | "off" | "no" | "false" => Some(false),
        _ => None,
    }
}

/// Whether this version comes before a release that is meant to last:
/// before 1.0, or a pre-release.
pub fn before_release() -> bool {
    is_before_release(
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_PRE"),
    )
}

fn is_before_release(major: &str, pre: &str) -> bool {
    major == "0" || !pre.is_empty()
}

/// Why gevrek's debug lines are written or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Because {
    /// `RUST_LOG` says, and nothing is added to it.
    RustLog,
    Setting,
    Build,
    Version,
}

/// Whether gevrek's own debug lines are written, and why. `rust_log`:
/// whether `RUST_LOG` is set.
pub fn debug_lines(
    rust_log: bool,
    setting: DebugLog,
    build: Option<bool>,
    before_release: bool,
) -> (bool, Because) {
    if rust_log {
        return (false, Because::RustLog);
    }
    match setting {
        DebugLog::On => (true, Because::Setting),
        DebugLog::Off => (false, Because::Setting),
        DebugLog::Auto => match build {
            Some(on) => (on, Because::Build),
            None => (before_release, Because::Version),
        },
    }
}

/// `desktop.debug_log` of the configuration file at `file`; `auto` where
/// there is no file, or it does not say (or says what is not understood:
/// the program says what is wrong with its file when it reads it).
pub fn setting_of(file: &Path) -> DebugLog {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| setting_in(&text))
        .unwrap_or_default()
}

fn setting_in(text: &str) -> Option<DebugLog> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::from_value(value.pointer("/desktop/debug_log")?.clone()).ok()
}

/// The start of a program's log.
pub struct Logging {
    program: &'static str,
    level: LevelFilter,
    config: Option<PathBuf>,
    read_setting: bool,
    only_where_asked: bool,
    quiet: Vec<(&'static str, LevelFilter)>,
}

impl Logging {
    /// For `program` (its name, as the log says it), which writes what
    /// is at `level` or above of everything.
    pub fn new(program: &'static str, level: LevelFilter) -> Self {
        Self {
            program,
            level,
            config: None,
            read_setting: true,
            only_where_asked: false,
            quiet: Vec::new(),
        }
    }

    /// The configuration is not to be read at all (as with the window
    /// manager's `--safe`): the setting counts as `auto`.
    pub fn without_setting(mut self) -> Self {
        self.read_setting = false;
        self
    }

    /// The configuration file the setting is read from, where it is
    /// another than the standard one (`--config`).
    pub fn config(mut self, file: Option<PathBuf>) -> Self {
        self.config = file;
        self
    }

    /// A command a person (or a script) runs: its debug lines are written
    /// only where the setting asks for them, not for the build's or the
    /// version's sake, so that its answer is not lost among them (and a
    /// script that reads what it says reads that alone).
    pub fn only_where_asked(mut self) -> Self {
        self.only_where_asked = true;
        self
    }

    /// A library that tells too much: only what is at `level` or above of
    /// it, unless `RUST_LOG` names it.
    pub fn quiet(mut self, module: &'static str, level: LevelFilter) -> Self {
        self.quiet.push((module, level));
        self
    }

    /// Start the log. Returns whether gevrek's debug lines are written.
    pub fn init(self) -> bool {
        let rust_log = std::env::var("RUST_LOG")
            .ok()
            .filter(|asked| !asked.trim().is_empty());
        let file = self
            .config
            .clone()
            .unwrap_or_else(gevrek_model::paths::config_file);
        let setting = if rust_log.is_some() || !self.read_setting {
            DebugLog::Auto
        } else {
            setting_of(&file)
        };
        let (mut on, because) =
            debug_lines(rust_log.is_some(), setting, build_says(), before_release());
        if self.only_where_asked && because != Because::Setting {
            on = false;
        }
        let mut logger = env_logger::Builder::from_env(env_logger::Env::default());
        if rust_log.is_none() {
            logger.filter_level(self.level);
            if on {
                logger.filter_module("gevrek", LevelFilter::Debug);
            }
        }
        for (module, level) in &self.quiet {
            if !rust_log
                .as_deref()
                .is_some_and(|asked| asked.contains(module))
            {
                logger.filter_module(module, *level);
            }
        }
        // (A second start, as in a check that starts it again, keeps the
        // first.)
        if logger.try_init().is_err() {
            return on;
        }
        if on || log::log_enabled!(log::Level::Debug) {
            install_panic_hook();
        }
        let why = match because {
            Because::RustLog => "as RUST_LOG says",
            Because::Setting => "as desktop.debug_log says",
            Because::Build => "as the build says (GEVREK_DEBUG_LOG)",
            Because::Version => "before 1.0",
        };
        log::debug!(
            "{} {} starts; gevrek's debug lines are {} ({why})",
            self.program,
            gevrek_build::version(),
            if on { "written" } else { "not written" },
        );
        on
    }
}

/// A panic is written to the log as well, with the stack that led to it
/// (the panic's own words still go where they went).
fn install_panic_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let words = panic
            .payload()
            .downcast_ref::<&str>()
            .map(|words| words.to_string())
            .or_else(|| panic.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(not words)".to_string());
        let place = panic
            .location()
            .map(|at| at.to_string())
            .unwrap_or_else(|| "?".to_string());
        log::error!(
            "the thread {name} panicked at {place}: {words}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        before(panic);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_1_0_the_debug_lines_are_written_and_from_1_0_on_not() {
        assert_eq!(
            debug_lines(false, DebugLog::Auto, None, true),
            (true, Because::Version)
        );
        assert_eq!(
            debug_lines(false, DebugLog::Auto, None, false),
            (false, Because::Version)
        );
        assert!(is_before_release("0", ""));
        assert!(is_before_release("1", "rc.1"));
        assert!(!is_before_release("1", ""));
        assert!(!is_before_release("2", ""));
    }

    #[test]
    fn the_build_says_before_the_version_and_the_setting_before_the_build() {
        assert_eq!(
            debug_lines(false, DebugLog::Auto, Some(true), false),
            (true, Because::Build)
        );
        assert_eq!(
            debug_lines(false, DebugLog::Auto, Some(false), true),
            (false, Because::Build)
        );
        assert_eq!(
            debug_lines(false, DebugLog::On, Some(false), false),
            (true, Because::Setting)
        );
        assert_eq!(
            debug_lines(false, DebugLog::Off, Some(true), true),
            (false, Because::Setting)
        );
    }

    #[test]
    fn rust_log_says_alone() {
        assert_eq!(
            debug_lines(true, DebugLog::On, Some(true), true),
            (false, Because::RustLog)
        );
    }

    #[test]
    fn the_build_says_in_a_few_words() {
        assert_eq!(said(Some("1")), Some(true));
        assert_eq!(said(Some(" ON ")), Some(true));
        assert_eq!(said(Some("off")), Some(false));
        assert_eq!(said(Some("0")), Some(false));
        assert_eq!(said(Some("maybe")), None);
        assert_eq!(said(None), None);
    }

    #[test]
    fn the_setting_is_read_from_the_file_and_is_auto_where_it_is_not_said() {
        assert_eq!(
            setting_in(r#"{"desktop": {"debug_log": "on"}}"#),
            Some(DebugLog::On)
        );
        assert_eq!(
            setting_in(r#"{"desktop": {"debug_log": "off"}}"#),
            Some(DebugLog::Off)
        );
        assert_eq!(setting_in(r#"{"desktop": {}}"#), None);
        assert_eq!(setting_in(r#"{"desktop": {"debug_log": "loud"}}"#), None);
        assert_eq!(setting_in("not json"), None);
        assert_eq!(
            setting_of(Path::new("/nowhere/at/all/config.json")),
            DebugLog::Auto
        );
    }

    #[test]
    fn the_setting_is_read_from_what_the_model_writes() {
        let mut config = gevrek_model::config::Config::new();
        let written = serde_json::to_string(&config).unwrap();
        assert_eq!(setting_in(&written), Some(DebugLog::Auto));
        config.desktop.debug_log = DebugLog::On;
        let written = serde_json::to_string(&config).unwrap();
        assert_eq!(setting_in(&written), Some(DebugLog::On));
    }
}
