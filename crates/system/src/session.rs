//! Ending the session, the machine's sleep, restart and switching off,
//! and switching the displays off.
//!
//! On Windows these are calls of the system itself; which call an end
//! maps to ([`call_for`]) asks nothing of Windows and is tested on every
//! system. Carrying one out ([`carry_out`]) is there on Windows only, and
//! is the one way to the system's calls: with [`DRY_RUN`] set in the
//! environment it carries nothing out, whoever asks.
//!
//! On Linux the shell asks the login service on the system's bus instead
//! (`gevrek-shell`'s `power`), and the window manager locks and switches
//! the displays off itself, so there is nothing here for it.

/// A way to leave, for good or for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The session stays, behind the system's lock screen.
    Lock,
    /// The session ends; the machine runs on.
    SignOut,
    Sleep,
    Restart,
    ShutDown,
    /// The displays show nothing and save power until the next key or
    /// movement; everything runs on.
    DisplaysOff,
}

impl End {
    pub const ALL: [End; 6] = [
        End::Lock,
        End::SignOut,
        End::Sleep,
        End::Restart,
        End::ShutDown,
        End::DisplaysOff,
    ];
}

/// Set (to anything but nothing) in a program's environment, that program
/// carries out nothing of what is here: [`carry_out`] says so and leaves
/// it. For the desktop checks, which must never lock, end or darken the
/// session they run in.
pub const DRY_RUN: &str = "GEVREK_POWER_DRY_RUN";

/// What came of asking for an end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The system was asked, and did not refuse.
    CarriedOut,
    /// [`DRY_RUN`] is set: the system was not asked.
    DryRun,
}

/// The system's call for an end, with what is said to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// `LockWorkStation`.
    Lock,
    /// `ExitWindowsEx` with these flags. With `privilege`, the right to
    /// shut the machine down has to be switched on for this program
    /// first (every user at the machine has it, switched off).
    Exit { flags: u32, privilege: bool },
    /// `SetSuspendState`: to sleep, not to disk, and without forcing the
    /// programs that ask for a moment first.
    Suspend,
    /// The system command `SC_MONITORPOWER` (`WM_SYSCOMMAND`) with this
    /// state of the displays.
    MonitorPower { state: isize },
}

/// `SC_MONITORPOWER`'s states, by Windows' own numbers: the displays
/// switched off. (-1 is on and 1 is low power; neither is asked for here.)
pub const MONITOR_OFF: isize = 2;

/// `ExitWindowsEx`'s flags, by Windows' own numbers.
pub const EXIT_SIGN_OUT: u32 = 0x0;
pub const EXIT_SHUT_DOWN: u32 = 0x1;
pub const EXIT_RESTART: u32 = 0x2;
pub const EXIT_POWER_OFF: u32 = 0x8;
/// Shut down as the Start menu does: so that the next start is a fast
/// one, where the machine is set up for that.
pub const EXIT_HYBRID: u32 = 0x0040_0000;
/// Never said here: programs with unsaved work are asked, and can hold
/// the end back, as they can when the Start menu asks.
pub const EXIT_FORCE: u32 = 0x4;
pub const EXIT_FORCE_IF_HUNG: u32 = 0x10;

/// Which call of the system carries an end out.
pub fn call_for(end: End) -> Call {
    match end {
        End::Lock => Call::Lock,
        End::SignOut => Call::Exit {
            flags: EXIT_SIGN_OUT,
            privilege: false,
        },
        End::Sleep => Call::Suspend,
        End::Restart => Call::Exit {
            flags: EXIT_RESTART,
            privilege: true,
        },
        End::ShutDown => Call::Exit {
            flags: EXIT_SHUT_DOWN | EXIT_POWER_OFF | EXIT_HYBRID,
            privilege: true,
        },
        End::DisplaysOff => Call::MonitorPower { state: MONITOR_OFF },
    }
}

/// Whether the environment's value of [`DRY_RUN`] says to carry nothing
/// out: any value but an empty one.
fn says_dry_run(switch: Option<&std::ffi::OsStr>) -> bool {
    switch.is_some_and(|value| !value.is_empty())
}

/// Whether this program carries nothing out ([`DRY_RUN`] is set), for a
/// program to say so when it starts.
pub fn is_dry_run() -> bool {
    says_dry_run(std::env::var_os(DRY_RUN).as_deref())
}

/// Carry an end out by `system`, unless the environment's value of
/// [`DRY_RUN`] (`switch`) says not to. Every call of the system goes
/// through here, so that nothing gets past the switch.
#[cfg(any(windows, test))]
fn carry_out_by(
    end: End,
    switch: Option<&std::ffi::OsStr>,
    system: impl FnOnce(Call) -> Result<(), String>,
) -> Result<Outcome, String> {
    if says_dry_run(switch) {
        return Ok(Outcome::DryRun);
    }
    system(call_for(end)).map(|()| Outcome::CarriedOut)
}

/// Carry an end out, unless [`DRY_RUN`] is set. An error says in words
/// why the system refused.
#[cfg(windows)]
pub fn carry_out(end: End) -> Result<Outcome, String> {
    carry_out_by(end, std::env::var_os(DRY_RUN).as_deref(), ask_windows)
}

/// Carry an end out on a Mac, unless [`DRY_RUN`] is set: the system's own
/// commands where it has them (`pmset`), else asking the login window, as
/// the Apple menu does.
#[cfg(target_os = "macos")]
pub fn carry_out(end: End) -> Result<Outcome, String> {
    if says_dry_run(std::env::var_os(DRY_RUN).as_deref()) {
        return Ok(Outcome::DryRun);
    }
    let (program, arguments): (&str, Vec<&str>) = match end {
        // The displays sleep; with "require password" (the system's
        // default), the session is locked behind them.
        End::Lock | End::DisplaysOff => ("/usr/bin/pmset", vec!["displaysleepnow"]),
        End::Sleep => ("/usr/bin/pmset", vec!["sleepnow"]),
        End::SignOut => ("/usr/bin/osascript", vec!["-e", "tell application \"loginwindow\" to «event aevtrlgo»"]),
        End::Restart => ("/usr/bin/osascript", vec!["-e", "tell application \"loginwindow\" to «event aevtrrst»"]),
        End::ShutDown => ("/usr/bin/osascript", vec!["-e", "tell application \"loginwindow\" to «event aevtrsdn»"]),
    };
    let output = std::process::Command::new(program)
        .args(&arguments)
        .output()
        .map_err(|error| format!("{program} did not start: {error}"))?;
    if output.status.success() {
        Ok(Outcome::CarriedOut)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// How long after it was asked for the displays are switched off: the
/// letting go of the very keys that asked would switch them on again.
#[cfg(windows)]
const DISPLAYS_OFF_DELAY: std::time::Duration = std::time::Duration::from_millis(700);

/// The system's call itself. Only [`carry_out`] comes here.
#[cfg(windows)]
fn ask_windows(call: Call) -> Result<(), String> {
    use windows::Win32::System::Power::SetSuspendState;
    use windows::Win32::System::Shutdown::{
        ExitWindowsEx, LockWorkStation, EXIT_WINDOWS_FLAGS, SHTDN_REASON_FLAG_PLANNED,
        SHTDN_REASON_MAJOR_OTHER,
    };
    match call {
        // SAFETY: a call without arguments.
        Call::Lock => unsafe { LockWorkStation() }
            .map_err(|error| format!("The session could not be locked: {error}")),
        Call::Exit { flags, privilege } => {
            if privilege {
                allow_shutdown()?;
            }
            // SAFETY: flags and a reason, both numbers.
            unsafe {
                ExitWindowsEx(
                    EXIT_WINDOWS_FLAGS(flags),
                    SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
                )
            }
            .map_err(|error| format!("Windows refused: {error}"))
        }
        Call::Suspend => {
            // The call returns when the machine is awake again: not on
            // the thread that draws.
            std::thread::Builder::new()
                .name("gevrek-sleep".to_string())
                .spawn(|| {
                    // SAFETY: three flags; see `Call::Suspend`.
                    let slept = unsafe { SetSuspendState(false, false, false) };
                    if !slept.as_bool() {
                        log::warn!("the machine did not go to sleep");
                    }
                })
                .map(|_| ())
                .map_err(|error| format!("The machine could not be sent to sleep: {error}"))
        }
        Call::MonitorPower { state } => std::thread::Builder::new()
            .name("gevrek-displays-off".to_string())
            .spawn(move || {
                use windows::Win32::Foundation::{LPARAM, WPARAM};
                use windows::Win32::UI::WindowsAndMessaging::{
                    PostMessageW, HWND_BROADCAST, SC_MONITORPOWER, WM_SYSCOMMAND,
                };
                std::thread::sleep(DISPLAYS_OFF_DELAY);
                // Posted, not sent: sending waits for every window's
                // answer, and one program that does not answer would
                // keep this one waiting with it.
                // SAFETY: a message of three numbers, to no window of ours.
                let posted = unsafe {
                    PostMessageW(
                        HWND_BROADCAST,
                        WM_SYSCOMMAND,
                        WPARAM(SC_MONITORPOWER as usize),
                        LPARAM(state),
                    )
                };
                if let Err(error) = posted {
                    log::warn!("the displays were not switched off: {error}");
                }
            })
            .map(|_| ())
            .map_err(|error| format!("The displays could not be switched off: {error}")),
    }
}

/// Switch on this program's right to shut the machine down.
#[cfg(windows)]
fn allow_shutdown() -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_SUCCESS, HANDLE, LUID};
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let refused = |error: windows::core::Error| {
        format!("This program may not shut the machine down: {error}")
    };
    // SAFETY: the token is this process's own and is closed below; the
    // structures are filled in by the calls or handed over whole.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(refused)?;
        let mut right = LUID::default();
        let adjusted = LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut right).and_then(|()| {
            let wanted = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: right,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            AdjustTokenPrivileges(token, false, Some(&wanted), 0, None, None)?;
            // It says yes also when it switched nothing on.
            let last = GetLastError();
            if last == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(windows::core::Error::from(last))
            }
        });
        let _ = CloseHandle(token);
        adjusted.map_err(refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_end_is_the_systems_call_for_it() {
        assert_eq!(call_for(End::Lock), Call::Lock);
        assert_eq!(call_for(End::Sleep), Call::Suspend);
        // Signing out is every user's to do; the machine's ends need the
        // right to shut it down switched on.
        assert_eq!(
            call_for(End::SignOut),
            Call::Exit {
                flags: 0,
                privilege: false
            }
        );
        assert_eq!(
            call_for(End::Restart),
            Call::Exit {
                flags: 0x2,
                privilege: true
            }
        );
        assert_eq!(
            call_for(End::ShutDown),
            Call::Exit {
                flags: 0x0040_0009,
                privilege: true
            }
        );
    }

    #[test]
    fn no_end_forces_programs_to_close_or_does_what_another_does() {
        let ends = End::ALL;
        for end in ends {
            if let Call::Exit { flags, .. } = call_for(end) {
                assert_eq!(flags & (EXIT_FORCE | EXIT_FORCE_IF_HUNG), 0, "{end:?}");
                // Restarting and switching off are never asked for at once.
                assert!(
                    flags & EXIT_RESTART == 0 || flags & (EXIT_SHUT_DOWN | EXIT_POWER_OFF) == 0,
                    "{end:?}"
                );
            }
        }
        for (index, one) in ends.iter().enumerate() {
            for other in &ends[index + 1..] {
                assert_ne!(call_for(*one), call_for(*other), "{one:?} and {other:?}");
            }
        }
    }

    #[test]
    fn switching_the_displays_off_is_the_system_command_for_it_and_only_off() {
        // Windows' numbers: 2 is off; -1 would switch them on.
        assert_eq!(call_for(End::DisplaysOff), Call::MonitorPower { state: 2 });
        for end in End::ALL {
            if let Call::MonitorPower { state } = call_for(end) {
                assert_eq!(end, End::DisplaysOff);
                assert_eq!(state, MONITOR_OFF);
            }
        }
    }

    #[test]
    fn with_the_switch_set_nothing_is_carried_out_whatever_is_asked() {
        use std::ffi::OsStr;
        for end in End::ALL {
            for value in ["1", "0", "no", " "] {
                let came = carry_out_by(end, Some(OsStr::new(value)), |call| {
                    panic!(
                        "{end:?} reached the system as {call:?} with the switch set to {value:?}"
                    )
                });
                assert_eq!(came, Ok(Outcome::DryRun), "{end:?}");
            }
        }
    }

    #[test]
    fn without_the_switch_each_end_reaches_the_system_once_as_its_own_call() {
        use std::ffi::OsStr;
        // Not set, and set to nothing, are the same: not set.
        for switch in [None, Some(OsStr::new(""))] {
            for end in End::ALL {
                let mut asked = Vec::new();
                let came = carry_out_by(end, switch, |call| {
                    asked.push(call);
                    Ok(())
                });
                assert_eq!(came, Ok(Outcome::CarriedOut));
                assert_eq!(asked, [call_for(end)]);
            }
        }
        // What the system refuses is said, not taken for done.
        let refused = carry_out_by(End::Lock, None, |_| Err("no".to_string()));
        assert_eq!(refused, Err("no".to_string()));
    }
}
