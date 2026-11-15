//! The sandbox: the draft, running, in a window of its own.
//!
//! A second gevrek is started inside the session as a nested window, as
//! its own instance with its own control socket and a private copy of the
//! configuration in the window. Every valid change is written to that
//! copy, which the nested gevrek watches and takes up; the desktop you
//! are sitting at is not touched until you apply.
//!
//! Linux only: there gevrek is the compositor and can run inside another.
//! On Windows the window manager arranges the real windows of the one
//! desktop, and there is nothing to nest.

use gevrek_model::Config;
use std::path::PathBuf;
use std::process::Child;

pub struct Sandbox {
    child: Child,
    /// The private copy the nested gevrek runs.
    config: PathBuf,
    instance: String,
    /// The generation of the document that was written last.
    written: u64,
}

/// Why no sandbox can be started here, if none can.
pub fn unavailable() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return Some("The sandbox is a nested gevrek, which only Linux has.".to_string());
    }
    match compositor() {
        Some(_) => None,
        None => Some("gevrek-wl was not found beside this program or on the PATH.".to_string()),
    }
}

/// The nested compositor's program: beside this one, else on the PATH.
fn compositor() -> Option<PathBuf> {
    let name = "gevrek-wl";
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
    {
        if beside.is_file() {
            return Some(beside);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|folder| folder.join(name))
            .find(|path| path.is_file())
    })
}

/// What the nested gevrek runs: the sample windows, one a little after
/// the other so that they come in their order, and nothing after they
/// are closed. The studio's program is named by the environment
/// (`GEVREK_STUDIO`), not written into the text.
pub fn samples_script() -> String {
    let mut script = String::new();
    for name in crate::sample::NAMES {
        script.push_str(&format!("\"$GEVREK_STUDIO\" --sample {name} & sleep 0.4; "));
    }
    script.push_str("wait");
    script
}

impl Sandbox {
    /// Start a nested gevrek that runs `config`.
    pub fn start(config: &Config, generation: u64) -> Result<Self, String> {
        let program = compositor().ok_or("gevrek-wl was not found")?;
        let instance = format!("sandbox-{}", std::process::id());
        let folder = gevrek_model::paths::runtime_dir().join(&instance);
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
        let path = folder.join("config.json");
        write(config, &path)?;
        let mut command = std::process::Command::new(program);
        command
            .args([
                "--backend",
                "winit",
                "--size",
                "1280x800",
                "--no-xwayland",
                "--config",
            ])
            .arg(&path)
            .env("GEVREK_INSTANCE", &instance)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Something to arrange: a few sample windows (this program
        // itself, as each of them), so that the draft's layout is seen at
        // once. Where this program cannot be found again, a terminal to
        // start programs from.
        match std::env::current_exe() {
            Ok(studio) => {
                command.arg("--").arg("sh").arg("-c").arg(samples_script());
                command.env("GEVREK_STUDIO", studio);
            }
            Err(_) => {
                command
                    .arg("--")
                    .arg(gevrek_model::examples::default_terminal());
            }
        }
        let child = command
            .spawn()
            .map_err(|error| format!("gevrek-wl did not start: {error}"))?;
        Ok(Self {
            child,
            config: path,
            instance,
            written: generation,
        })
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Whether the nested gevrek still runs (its window may be closed).
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Give the nested gevrek the configuration as it is now, if it
    /// changed since it was last given.
    pub fn mirror(&mut self, config: &Config, generation: u64) {
        if generation == self.written {
            return;
        }
        self.written = generation;
        if let Err(error) = write(config, &self.config) {
            log::warn!("the sandbox was not given the change: {error}");
        }
    }

    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config);
    }
}

/// The sandbox's copy never keeps the file watcher off and never starts
/// the session's own programs a second time.
fn write(config: &Config, path: &std::path::Path) -> Result<(), String> {
    let mut copy = config.clone();
    copy.desktop.watch_configuration = true;
    copy.desktop.startup.clear();
    let text = serde_json::to_string_pretty(&copy).map_err(|error| error.to_string())?;
    gevrek_model::files::write_atomic(path, text.as_bytes())
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_copy_watches_its_file_and_starts_nothing() {
        let mut config = gevrek_model::examples::starting_configuration();
        config.desktop.watch_configuration = false;
        config.desktop.startup = serde_json::from_value(
            serde_json::json!([{"name": "mail", "launch": {"program": "thunderbird"}}]),
        )
        .unwrap_or_default();
        let folder =
            std::env::temp_dir().join(format!("gevrek-studio-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let path = folder.join("config.json");
        write(&config, &path).expect("written");
        let read = Config::load(&path).expect("read back");
        assert!(read.desktop.watch_configuration);
        assert!(read.desktop.startup.is_empty());
        assert_eq!(read.desktop.bindings.len(), config.desktop.bindings.len());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&folder);
    }
}

#[cfg(test)]
mod sample_tests {
    #[test]
    fn the_sandbox_opens_every_sample_by_the_studio_named_in_the_environment() {
        let script = super::samples_script();
        for name in crate::sample::NAMES {
            assert!(
                script.contains(&format!("\"$GEVREK_STUDIO\" --sample {name} &")),
                "{script}"
            );
        }
        assert!(script.ends_with("wait"));
    }
}
