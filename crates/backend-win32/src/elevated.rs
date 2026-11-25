//! Windows of programs that run with an administrator's rights, which a
//! gevrek without them cannot manage.
//!
//! Windows lets a program move, resize, hide or give the keyboard to the
//! windows of programs with no more rights than its own. A window of a
//! program that runs elevated ("as administrator") refuses all of it. Taken
//! into a layout, such a window would get a tile it never goes to, and the
//! other windows would make room for nothing. So it is left alone
//! altogether: no window to the window manager, as if a rule ignored it.
//!
//! Where gevrek itself runs elevated every window is within its reach, and
//! none is left out for this.

use std::collections::HashMap;
use std::sync::Mutex;

/// Whether a window of a program is beyond this program's reach: the
/// other is elevated and this one is not. `None` for the other: Windows
/// would not say, which it does not for a program with more rights.
fn beyond_reach(own_elevated: bool, other_elevated: Option<bool>) -> bool {
    !own_elevated && other_elevated.unwrap_or(true)
}

/// What was found out about processes, by their numbers (a process keeps
/// its rights for as long as it lives; the list is emptied when it grows,
/// since a number is given out again after a process ends).
static KNOWN: Mutex<Option<HashMap<u32, (bool, std::time::Instant)>>> = Mutex::new(None);

/// How long what was found out of a process is believed: its number is
/// given to another program once it has ended, which may have other
/// rights.
const BELIEVED_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether a window belongs to a program this one cannot manage.
pub fn window_is_beyond_reach(window: isize) -> bool {
    !system::own_elevated() && window_is_elevated(window)
}

/// Whether a window belongs to a program that runs as an administrator
/// (or to one Windows does not say it of, which is one with more rights
/// than whoever asks). The same answer to the window manager and to its
/// helper: the two go by it to tell whose a key or a press is
/// (`hotkey.rs`).
pub fn window_is_elevated(window: isize) -> bool {
    let Some(process) = system::process_of(window) else {
        return false;
    };
    let mut known = KNOWN.lock().unwrap_or_else(|error| error.into_inner());
    let known = known.get_or_insert_with(HashMap::new);
    if let Some((answer, when)) = known.get(&process) {
        if when.elapsed() < BELIEVED_FOR {
            return *answer;
        }
    }
    if known.len() > 512 {
        known.clear();
    }
    let answer = beyond_reach(false, system::elevated(process));
    if answer {
        log::debug!("process {process} runs as an administrator");
    }
    known.insert(process, (answer, std::time::Instant::now()));
    answer
}

/// Whether this program runs as an administrator itself.
pub fn own_elevated() -> bool {
    system::own_elevated()
}

/// The name of the task that starts the helper for elevated windows
/// without Windows' question (`gevrek elevated install` registers it).
pub fn helper_task_name(instance: &str) -> String {
    let plain: String = instance
        .chars()
        .map(|letter| {
            if letter.is_ascii_alphanumeric() {
                letter
            } else {
                '-'
            }
        })
        .collect();
    format!("gevrek elevated helper ({plain})")
}

/// The folder of the machine's programs, as the system itself names it.
fn programs_folder() -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_ProgramFiles, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG,
    };
    // SAFETY: the text is the system's until it is freed below, and is
    // copied before that.
    unsafe {
        let named =
            SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let folder = named.to_string().ok();
        CoTaskMemFree(Some(named.0 as *const core::ffi::c_void));
        folder
            .filter(|folder| !folder.is_empty())
            .map(std::path::PathBuf::from)
    }
}

/// Where the helper lies that the task starts: a folder only an
/// administrator can write to. (gevrek itself is installed for one person,
/// in a folder that person's programs can write to: a task that started a
/// file from there as an administrator, without asking, would let any of
/// those programs put another file in its place and run as one.)
pub fn kept_helper() -> std::path::PathBuf {
    // Where the system says its programs are, and not where a variable
    // says so: whoever starts this program sets its variables, and the
    // folder "only an administrator can write to" was wherever they said.
    let programs = programs_folder()
        .or_else(|| {
            std::env::var_os("ProgramW6432")
                .or_else(|| std::env::var_os("ProgramFiles"))
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Program Files"));
    programs.join("gevrek").join("gevrek-elevated.exe")
}

/// The task as Windows' own tool reads it: run by hand only, for one
/// person, with the most rights that person has, and no end to its time.
fn helper_task_xml(person: &str, command: &std::path::Path, arguments: &str) -> String {
    let plain = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts gevrek's helper for the windows of programs that run as an administrator. Run by gevrek's window manager when it starts.</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{person}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        person = plain(person),
        command = plain(&command.to_string_lossy()),
        arguments = plain(arguments),
    )
}

/// Windows' tool for tasks, without a window of its own.
fn task_tool(arguments: &[&str]) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("schtasks.exe")
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "schtasks {}: {}",
        arguments.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// Register the task that starts the helper without a question, for an
/// instance of gevrek, the window manager it serves and the person it is
/// for. Done by the helper itself, started once with an administrator's
/// rights (`gevrek elevated install`): it puts a copy of itself where
/// only an administrator can write, and the task starts that copy.
pub fn install_helper_task(
    instance: &str,
    window_manager: &std::path::Path,
    person: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        own_elevated(),
        "only an administrator can register the task"
    );
    let kept = kept_helper();
    let own = std::env::current_exe()?;
    if own != kept {
        if let Some(folder) = kept.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // (The helper of the build before may still be ending, with the
        // window manager it served: Windows holds its file that long.)
        let mut tries = 0;
        loop {
            match std::fs::copy(&own, &kept) {
                Ok(_) => break,
                Err(error) if tries >= 20 => return Err(error.into()),
                Err(_) => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    let arguments = format!(
        "--instance \"{instance}\" --wm \"{}\"",
        window_manager.display()
    );
    let written = helper_task_xml(person, &kept, &arguments);
    // (As Windows' own tools write it: two bytes a letter, marked.)
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(written.encode_utf16().flat_map(u16::to_le_bytes));
    // Beside the helper, where only an administrator can write: Windows'
    // tool reads the file a moment after it was written, and in the
    // person's own temporary folder any of their programs could put
    // another task in its place meanwhile.
    let file = kept.with_file_name(format!("gevrek-elevated-task-{}.xml", std::process::id()));
    std::fs::write(&file, bytes)?;
    let made = task_tool(&[
        "/create",
        "/f",
        "/tn",
        &helper_task_name(instance),
        "/xml",
        &file.to_string_lossy(),
    ]);
    let _ = std::fs::remove_file(&file);
    made
}

/// Remove the task, and the helper's copy where no other task is left to
/// start it (the copy of a helper that runs stays: Windows holds its file).
pub fn remove_helper_task(instance: &str) -> anyhow::Result<()> {
    anyhow::ensure!(own_elevated(), "only an administrator can remove the task");
    let removed = task_tool(&["/delete", "/f", "/tn", &helper_task_name(instance)]);
    let _ = std::fs::remove_file(kept_helper());
    removed
}

/// Whether the task is registered.
pub fn helper_task_is_registered(instance: &str) -> bool {
    task_tool(&["/query", "/tn", &helper_task_name(instance)]).is_ok()
}

/// Run the task that starts the helper, where it is registered. Nothing
/// happens where it is not: no helper, and the windows of elevated
/// programs are left alone as before.
pub fn run_helper_task(instance: &str) {
    use std::os::windows::process::CommandExt;
    // (No window for the tool: 0x08000000 is CREATE_NO_WINDOW.)
    let started = std::process::Command::new("schtasks.exe")
        .args(["/run", "/tn", &helper_task_name(instance)])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn();
    match started {
        Ok(_) => log::debug!("asked for the helper for elevated windows by its task"),
        Err(error) => log::debug!("the helper's task could not be asked for: {error}"),
    }
}

mod system {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    /// The process a window is of.
    pub fn process_of(window: isize) -> Option<u32> {
        let mut process = 0u32;
        // SAFETY: a plain question; a window that is gone has no process.
        unsafe { GetWindowThreadProcessId(HWND(window as *mut _), Some(&mut process)) };
        (process != 0).then_some(process)
    }

    /// Whether the program behind a process handle runs elevated; `None`
    /// where Windows does not say.
    fn elevated_of(process: HANDLE) -> Option<bool> {
        // SAFETY: the token is closed below; the answer is a plain number.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut answer = TOKEN_ELEVATION::default();
            let mut length = 0u32;
            let asked = GetTokenInformation(
                token,
                TokenElevation,
                Some((&mut answer as *mut TOKEN_ELEVATION).cast()),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut length,
            );
            let _ = CloseHandle(token);
            asked.ok()?;
            Some(answer.TokenIsElevated != 0)
        }
    }

    /// Whether this program runs elevated (asked once).
    pub fn own_elevated() -> bool {
        static OWN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        // SAFETY: the handle of this process needs no closing.
        *OWN.get_or_init(|| elevated_of(unsafe { GetCurrentProcess() }).unwrap_or(false))
    }

    /// Whether another process runs elevated; `None` where Windows does
    /// not say (which it does not for one with more rights than ours).
    pub fn elevated(process: u32) -> Option<bool> {
        // SAFETY: the handle is closed below.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process).ok()?;
            let answer = elevated_of(handle);
            let _ = CloseHandle(handle);
            answer
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_of_an_elevated_program_is_beyond_a_gevrek_that_is_not() {
        assert!(beyond_reach(false, Some(true)));
        // Windows does not say: it does not for a program with more rights.
        assert!(beyond_reach(false, None));
        // An ordinary program's window is within reach.
        assert!(!beyond_reach(false, Some(false)));
        // An elevated gevrek reaches every window.
        assert!(!beyond_reach(true, Some(true)));
        assert!(!beyond_reach(true, None));
    }

    #[test]
    fn this_programs_own_windows_are_within_its_reach() {
        // (No window: nothing to leave alone.)
        assert!(!window_is_beyond_reach(0));
        assert_eq!(
            system::elevated(std::process::id()),
            Some(system::own_elevated())
        );
    }
}
