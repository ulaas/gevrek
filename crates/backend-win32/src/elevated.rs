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
static KNOWN: Mutex<Option<HashMap<u32, bool>>> = Mutex::new(None);

/// Whether a window belongs to a program this one cannot manage.
pub fn window_is_beyond_reach(window: isize) -> bool {
    let Some(process) = system::process_of(window) else {
        return false;
    };
    let mut known = KNOWN.lock().unwrap_or_else(|error| error.into_inner());
    let known = known.get_or_insert_with(HashMap::new);
    if let Some(answer) = known.get(&process) {
        return *answer;
    }
    if known.len() > 512 {
        known.clear();
    }
    let answer = beyond_reach(system::own_elevated(), system::elevated(process));
    if answer {
        log::info!(
            "A window of process {process} is left alone: the program runs as an administrator and gevrek does not"
        );
    }
    known.insert(process, answer);
    answer
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
