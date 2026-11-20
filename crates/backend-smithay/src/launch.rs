//! Starting programs for the user (startup programs, launch nodes,
//! scratchpads, workspace hooks).

use gevrek_model::settings::LaunchProfile;
use std::process::{Command, Stdio};

/// Start the program and return its process id. It lives on its own: in a
/// process group of its own, so a signal meant for gevrek (Ctrl+C in the
/// terminal it was started from) is not also sent to it; and it is waited
/// for when it ends, so that no dead process stays behind in the table.
pub fn spawn(launch: &LaunchProfile) -> anyhow::Result<u32> {
    spawn_then(launch, || {})
}

/// As [`spawn`]; `ended` is called (on a thread of its own) when the
/// program has ended.
pub fn spawn_then(
    launch: &LaunchProfile,
    ended: impl FnOnce() + Send + 'static,
) -> anyhow::Result<u32> {
    if launch.program.trim().is_empty() {
        anyhow::bail!("no program to launch");
    }
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .envs(&launch.environment)
        .stdin(Stdio::null());
    if let Some(directory) = &launch.working_directory {
        command.current_dir(directory);
    }
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("could not start {}: {error}", launch.program))?;
    let pid = child.id();
    // Nothing waits for the result; the thread only collects it.
    let reaper = std::thread::Builder::new()
        .name("gevrek-reap".to_string())
        .spawn(move || {
            match child.wait() {
                Ok(status) => log::debug!("the program of pid {pid} ended ({status})"),
                Err(error) => log::debug!("the end of pid {pid} could not be waited for: {error}"),
            }
            ended();
        });
    if let Err(error) = reaper {
        log::debug!("no thread to wait for {pid}: {error}");
    }
    Ok(pid)
}
