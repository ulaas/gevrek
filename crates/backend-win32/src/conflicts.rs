//! Other programs on this desktop that do what gevrek is set to do, with
//! the same keys. Both then act on one input, and neither result is what
//! was meant. gevrek does not take the other program's keys away or stop
//! it: it says so (`Notice`), and the person decides.
//!
//! Known so far:
//!
//! - **PowerToys FancyZones**: by default a window dragged with Shift held
//!   is dropped into one of its zones, which is exactly gevrek's default
//!   for its own zones (`desktop.snap_zones.activation_modifier`).
//!
//! And one thing about gevrek itself that changes what its keys do:
//!
//! - **Started as an administrator**: what gevrek starts (the launcher, a
//!   key bound to a program, the startup programs) then runs as an
//!   administrator too, which is seldom meant.

use gevrek_engine::overlay_data::Notice;
use gevrek_model::settings::DesktopConfig;
use gevrek_model::snap_zones::SnapZoneModifier;

/// What there is to say about the desktop as it is now.
pub fn notices(desktop: &DesktopConfig) -> Vec<Notice> {
    let zones = &desktop.snap_zones.behavior;
    // Asked only when it could matter: the list of processes is not free.
    let shift_zones = zones.enabled && zones.activation_modifier == Some(SnapZoneModifier::Shift);
    let fancy_zones = shift_zones && process_runs("PowerToys.FancyZones.exe");
    fancyzones_notice(
        shift_zones,
        fancy_zones,
        fancy_zones && fancyzones_uses_shift(fancyzones_settings().as_deref()),
    )
    .into_iter()
    .chain(administrator_notice(runs_as_administrator()))
    .collect()
}

/// The notice about gevrek having an administrator's rights.
fn administrator_notice(administrator: bool) -> Option<Notice> {
    administrator.then(|| Notice {
        id: "runs-as-administrator".to_string(),
        title: "gevrek runs as an administrator".to_string(),
        text: "Every program gevrek starts (from the launcher, a key, the startup list) runs as \
               an administrator too. Unless that is meant, stop gevrek and start it from a \
               terminal or a shortcut that is not \"Run as administrator\"."
            .to_string(),
    })
}

/// Whether this process has an administrator's rights now (it was started
/// so, or the machine asks nobody).
fn runs_as_administrator() -> bool {
    // SAFETY: a plain question about this process.
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool()
}

/// The notice about FancyZones, when gevrek's zones are dropped into with
/// Shift, FancyZones runs, and it uses Shift for the same.
fn fancyzones_notice(shift_zones: bool, running: bool, uses_shift: bool) -> Option<Notice> {
    (shift_zones && running && uses_shift).then(|| Notice {
        id: "fancyzones-shift-drag".to_string(),
        title: "PowerToys FancyZones also uses Shift while dragging".to_string(),
        text: "A window dragged with Shift held is meant for one of gevrek's snap zones, and \
               FancyZones takes it for one of its own. Give one of the two another key \
               (gevrek: Snap zones, activation key; FancyZones: \"Hold Shift key to activate \
               zones while dragging\"), or switch FancyZones off."
            .to_string(),
    })
}

/// Whether FancyZones drops a window into a zone while Shift is held: its
/// setting `fancyzones_shiftDrag`. It is on unless the settings say
/// otherwise (that is FancyZones' default, and a file that cannot be read
/// says nothing).
fn fancyzones_uses_shift(settings: Option<&str>) -> bool {
    settings
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|settings| {
            settings
                .pointer("/properties/fancyzones_shiftDrag/value")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(true)
}

/// FancyZones' settings: `%LOCALAPPDATA%\Microsoft\PowerToys\FancyZones\settings.json`.
fn fancyzones_settings() -> Option<String> {
    let file = dirs::data_local_dir()?
        .join("Microsoft")
        .join("PowerToys")
        .join("FancyZones")
        .join("settings.json");
    std::fs::read_to_string(file).ok()
}

/// Whether a program of this file name runs (in any session of this user).
fn process_runs(program: &str) -> bool {
    !processes_named(program).is_empty()
}

/// Whether this instance's shell (`gevrek-shell`) runs: a process that
/// said it is this instance's shell (`gevrek_model::paths::claim_process`)
/// and still runs that program. Asked when the list of shortcuts is
/// wanted: the shell shows it where it runs.
pub fn shell_runs() -> bool {
    let claims: Vec<(String, u32)> = gevrek_model::paths::claimed_processes("shell")
        .into_iter()
        .map(|(instance, pid, _)| (instance, pid))
        .collect();
    // Asked only when someone claims to be it: the list is not free.
    let instance = gevrek_model::paths::instance();
    claims.iter().any(|(owner, _)| *owner == instance)
        && claimed_and_running(&claims, &instance, &processes_named("gevrek-shell.exe"))
}

/// Whether one of the processes that claim to be `instance`'s is among
/// those that run the program. (A claim outlives a process that was
/// ended, and its number may be another program's by now.)
fn claimed_and_running(claims: &[(String, u32)], instance: &str, running: &[u32]) -> bool {
    claims
        .iter()
        .any(|(owner, pid)| owner == instance && running.contains(pid))
}

/// The processes that run a program of this file name, by their numbers.
fn processes_named(program: &str) -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: the snapshot handle is closed below; the entry is a plain
    // structure the two calls fill in.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = Vec::new();
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|character| *character == 0)
                    .unwrap_or(entry.szExeFile.len());
                if String::from_utf16_lossy(&entry.szExeFile[..length])
                    .eq_ignore_ascii_case(program)
                {
                    found.push(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_administrators_rights_are_spoken_of() {
        assert!(administrator_notice(false).is_none());
        assert_eq!(
            administrator_notice(true).map(|notice| notice.id),
            Some("runs-as-administrator".to_string())
        );
    }

    #[test]
    fn fancyzones_is_spoken_of_only_when_both_use_shift_and_it_runs() {
        assert!(fancyzones_notice(true, true, true).is_some());
        // gevrek's zones are off, or on another key.
        assert!(fancyzones_notice(false, true, true).is_none());
        // FancyZones does not run.
        assert!(fancyzones_notice(true, false, true).is_none());
        // FancyZones is on another key.
        assert!(fancyzones_notice(true, true, false).is_none());
    }

    #[test]
    fn the_shell_runs_when_a_process_that_claims_to_be_ours_still_is_it() {
        let claims = [("main".to_string(), 10), ("checks".to_string(), 20)];
        assert!(claimed_and_running(&claims, "checks", &[20, 30]));
        // The other instance's shell is not ours.
        assert!(!claimed_and_running(&claims, "checks", &[10]));
        // A claim left behind by a shell that ended.
        assert!(!claimed_and_running(&claims, "checks", &[]));
        assert!(!claimed_and_running(&[], "main", &[10]));
    }

    #[test]
    fn fancyzones_shift_is_read_from_its_settings() {
        let on = r#"{"properties":{"fancyzones_shiftDrag":{"value":true},"other":{"value":1}}}"#;
        let off = r#"{"properties":{"fancyzones_shiftDrag":{"value":false}}}"#;
        assert!(fancyzones_uses_shift(Some(on)));
        assert!(!fancyzones_uses_shift(Some(off)));
        // Its default, when the settings say nothing or cannot be read.
        assert!(fancyzones_uses_shift(Some(r#"{"properties":{}}"#)));
        assert!(fancyzones_uses_shift(Some("not json")));
        assert!(fancyzones_uses_shift(None));
    }

    #[test]
    fn nothing_is_said_of_fancyzones_when_the_zones_are_not_on_shift() {
        // Of FancyZones only: what else `notices` says is of the machine
        // the test runs on. (Where the tests run as an administrator, as
        // on a GitHub runner, that is said, and rightly.)
        let of_fancyzones = |desktop: &DesktopConfig| {
            notices(desktop)
                .iter()
                .any(|notice| notice.id == "fancyzones-shift-drag")
        };
        let mut desktop = DesktopConfig::default();
        desktop.snap_zones.behavior.enabled = false;
        assert!(!of_fancyzones(&desktop));
        desktop.snap_zones.behavior.enabled = true;
        desktop.snap_zones.behavior.activation_modifier = Some(SnapZoneModifier::Alt);
        assert!(!of_fancyzones(&desktop));
    }

    #[test]
    fn what_is_said_of_the_machine_is_what_is_so_there() {
        let mut desktop = DesktopConfig::default();
        desktop.snap_zones.behavior.enabled = false;
        let said: Vec<String> = notices(&desktop)
            .into_iter()
            .map(|notice| notice.id)
            .collect();
        let expected: Vec<String> = administrator_notice(runs_as_administrator())
            .into_iter()
            .map(|notice| notice.id)
            .collect();
        assert_eq!(said, expected);
    }
}
