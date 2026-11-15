//! The window that lists the current shortcuts.
//!
//! It is the settings program started with `--shortcuts` (gevrek-studio):
//! a small window that asks the running WM for its configuration and shows
//! the key assignments.
//! The WM only starts and ends that program; the same command does both, so
//! one key opens the list and closes it again.

use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};

static ADDRESS: OnceLock<String> = OnceLock::new();
static WINDOW: Mutex<Option<Child>> = Mutex::new(None);

/// Tell where this WM's server listens (`ip:port`), for the program started
/// by [`toggle`]. Called once by the WM's `main`.
pub fn set_server_address(address: impl Into<String>) {
    let _ = ADDRESS.set(address.into());
}

/// Whether the window this program opened is still open.
pub fn is_open() -> bool {
    let mut window = WINDOW
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    window
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
}

/// Open the list of shortcuts, or close it when it is open.
pub fn toggle() -> anyhow::Result<()> {
    let mut window = WINDOW
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(mut child) = window.take() {
        // Still open: close it. (`try_wait` is `Ok(None)` while it runs.)
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
    }
    let program = editor_program()?;
    let mut command = Command::new(&program);
    command.arg("--shortcuts");
    if let Some(address) = ADDRESS.get() {
        command.arg("--address").arg(address);
    }
    #[cfg(windows)]
    {
        // No console window for it.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| anyhow::anyhow!("Could not start {}: {error}", program.display()))?;
    *window = Some(child);
    Ok(())
}

/// The settings program next to this program; failing that, the one on
/// the `PATH`.
fn editor_program() -> anyhow::Result<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "gevrek-studio.exe"
    } else {
        "gevrek-studio"
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|folder| folder.join(name)))
        .filter(|path| path.exists());
    Ok(beside.unwrap_or_else(|| std::path::PathBuf::from(name)))
}
