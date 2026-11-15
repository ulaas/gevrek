//! When the system itself switches the displays off and sends the machine
//! to sleep after nobody used it.
//!
//! On Windows these times are the system's, not gevrek's: they are in the
//! power plan in use (Settings, System, Power), Windows keeps to them
//! whatever runs, and `desktop.session`'s times are not applied there.
//! They are only read here ([`read`]), never written, so that gevrek can
//! say what holds on this machine. What is read is turned into words
//! ([`IdleTimes::lines`]) without asking anything of Windows, and that is
//! tested on every system.
//!
//! On Linux gevrek is the session and keeps the times of its own
//! configuration, so there is nothing to read.

/// One setting of the power plan as it is on mains and on battery. `None`
/// where the system would not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OnPower {
    pub mains: Option<u32>,
    pub battery: Option<u32>,
}

/// The settings of a power plan that are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Seconds without input until the displays are switched off; 0: never.
    DisplaysOff,
    /// Seconds without input until the machine sleeps; 0: never.
    Sleep,
    /// Whether a password is asked for when the machine wakes: 0 or 1.
    PasswordOnWake,
}

/// Which of a setting's two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Mains,
    Battery,
}

/// The system's times, in seconds as the power plan holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdleTimes {
    pub displays_off: OnPower,
    pub sleep: OnPower,
    /// 1: a password is asked for on waking; 0: none is.
    pub password_on_wake: OnPower,
}

impl IdleTimes {
    /// The times as `value` gives them, setting by setting.
    pub fn read_by(value: impl Fn(Setting, Power) -> Option<u32>) -> Self {
        let both = |setting| OnPower {
            mains: value(setting, Power::Mains),
            battery: value(setting, Power::Battery),
        };
        Self {
            displays_off: both(Setting::DisplaysOff),
            sleep: both(Setting::Sleep),
            password_on_wake: both(Setting::PasswordOnWake),
        }
    }

    /// Whether the system said anything at all.
    pub fn is_known(&self) -> bool {
        [self.displays_off, self.sleep, self.password_on_wake]
            .iter()
            .any(|one| one.mains.is_some() || one.battery.is_some())
    }

    /// The times in words, a line each, for a person to read. What the
    /// system would not say of the password is left out; a time it would
    /// not say is "not known".
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Displays off: {}", both(self.displays_off, after)),
            format!("Sleep: {}", both(self.sleep, after)),
        ];
        if self.password_on_wake != OnPower::default() {
            lines.push(format!(
                "Password on wake: {}",
                both(self.password_on_wake, asked)
            ));
        }
        lines
    }
}

/// A setting's two values in words: once when they are the same.
fn both(values: OnPower, words: fn(Option<u32>) -> String) -> String {
    let (mains, battery) = (words(values.mains), words(values.battery));
    if mains == battery {
        mains
    } else {
        format!("{mains} on mains, {battery} on battery")
    }
}

/// A time in words: whole minutes where it is, seconds where it is not.
fn after(seconds: Option<u32>) -> String {
    let count = |number: u32, unit: &str| {
        let plural = if number == 1 { "" } else { "s" };
        format!("after {number} {unit}{plural}")
    };
    match seconds {
        None => "not known".to_string(),
        Some(0) => "never".to_string(),
        Some(seconds) if seconds % 60 == 0 => count(seconds / 60, "minute"),
        Some(seconds) => count(seconds, "second"),
    }
}

fn asked(value: Option<u32>) -> String {
    match value {
        None => "not known",
        Some(0) => "not asked for",
        Some(_) => "asked for",
    }
    .to_string()
}

/// The power plan's own names for a setting: its group and itself.
#[cfg(any(windows, test))]
fn guids(setting: Setting) -> (u128, u128) {
    // `GUID_VIDEO_SUBGROUP`, `GUID_SLEEP_SUBGROUP` and `NO_SUBGROUP_GUID`.
    const VIDEO: u128 = 0x7516b95f_f776_4464_8c53_06167f40cc99;
    const SLEEP: u128 = 0x238c9fa8_0aad_41ed_83f4_97be242c8f20;
    const NONE: u128 = 0xfea3413e_7e05_4911_9a71_700331f1c294;
    match setting {
        // `GUID_VIDEO_POWERDOWN_TIMEOUT` (powercfg: VIDEOIDLE).
        Setting::DisplaysOff => (VIDEO, 0x3c0bc021_c8a8_4e07_a973_6b14cbcb2b7e),
        // `GUID_STANDBY_TIMEOUT` (powercfg: STANDBYIDLE).
        Setting::Sleep => (SLEEP, 0x29f6c1db_86da_48c5_9fdb_f2b67b1f44da),
        // `GUID_LOCK_CONSOLE_ON_WAKE` (powercfg: CONSOLELOCK).
        Setting::PasswordOnWake => (NONE, 0x0e796bdb_100d_47d6_a2d5_f7d2daa51f51),
    }
}

/// The times of the power plan in use. Read only: nothing of the plan is
/// changed. Elsewhere than on Windows there is nothing to read.
#[cfg(not(windows))]
pub fn read() -> Option<IdleTimes> {
    None
}

/// The times of the power plan in use. Read only: nothing of the plan is
/// changed. `None` when Windows names no plan or says nothing of it.
#[cfg(windows)]
pub fn read() -> Option<IdleTimes> {
    use windows::core::GUID;
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
    use windows::Win32::System::Power::{
        PowerGetActiveScheme, PowerReadACValueIndex, PowerReadDCValueIndex,
    };
    use windows::Win32::System::Registry::HKEY;
    let mut named: *mut GUID = std::ptr::null_mut();
    // SAFETY: the call writes one pointer, to a name it allocated; it is
    // copied and given back below.
    let scheme = unsafe {
        if PowerGetActiveScheme(HKEY::default(), &mut named) != ERROR_SUCCESS || named.is_null() {
            return None;
        }
        let scheme = *named;
        let _ = LocalFree(HLOCAL(named.cast()));
        scheme
    };
    let times = IdleTimes::read_by(|setting, power| {
        let (group, itself) = guids(setting);
        let (group, itself) = (GUID::from_u128(group), GUID::from_u128(itself));
        let mut value = 0u32;
        // SAFETY: three names that live through the call, and one number
        // for it to write.
        let read = unsafe {
            match power {
                Power::Mains => {
                    PowerReadACValueIndex(
                        HKEY::default(),
                        Some(&scheme),
                        Some(&group),
                        Some(&itself),
                        &mut value,
                    )
                    .0
                }
                Power::Battery => PowerReadDCValueIndex(
                    HKEY::default(),
                    Some(&scheme),
                    Some(&group),
                    Some(&itself),
                    &mut value,
                ),
            }
        };
        (read == ERROR_SUCCESS.0).then_some(value)
    });
    times.is_known().then_some(times)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(displays: (u32, u32), sleep: (u32, u32)) -> IdleTimes {
        IdleTimes::read_by(|setting, power| {
            let (mains, battery) = match setting {
                Setting::DisplaysOff => displays,
                Setting::Sleep => sleep,
                Setting::PasswordOnWake => return None,
            };
            Some(match power {
                Power::Mains => mains,
                Power::Battery => battery,
            })
        })
    }

    #[test]
    fn the_times_are_said_in_minutes_and_zero_is_never() {
        assert_eq!(
            times((600, 300), (1800, 0)).lines(),
            [
                "Displays off: after 10 minutes on mains, after 5 minutes on battery",
                "Sleep: after 30 minutes on mains, never on battery",
            ]
        );
    }

    #[test]
    fn a_time_that_is_the_same_on_mains_and_battery_is_said_once() {
        assert_eq!(
            times((60, 60), (0, 0)).lines(),
            ["Displays off: after 1 minute", "Sleep: never"]
        );
        // What is no whole minute is said in seconds.
        assert_eq!(
            times((90, 1), (0, 0)).lines()[0],
            "Displays off: after 90 seconds on mains, after 1 second on battery"
        );
    }

    #[test]
    fn each_value_is_read_for_its_own_setting_and_power() {
        let read = IdleTimes::read_by(|setting, power| {
            Some(match (setting, power) {
                (Setting::DisplaysOff, Power::Mains) => 1,
                (Setting::DisplaysOff, Power::Battery) => 2,
                (Setting::Sleep, Power::Mains) => 3,
                (Setting::Sleep, Power::Battery) => 4,
                (Setting::PasswordOnWake, Power::Mains) => 5,
                (Setting::PasswordOnWake, Power::Battery) => 6,
            })
        });
        let pair = |one: OnPower| (one.mains, one.battery);
        assert_eq!(pair(read.displays_off), (Some(1), Some(2)));
        assert_eq!(pair(read.sleep), (Some(3), Some(4)));
        assert_eq!(pair(read.password_on_wake), (Some(5), Some(6)));
    }

    #[test]
    fn the_password_is_spoken_of_only_when_the_system_says_something_of_it() {
        let mut read = times((600, 600), (0, 0));
        assert_eq!(read.lines().len(), 2);
        read.password_on_wake = OnPower {
            mains: Some(1),
            battery: Some(1),
        };
        assert_eq!(read.lines()[2], "Password on wake: asked for");
        read.password_on_wake.battery = Some(0);
        assert_eq!(
            read.lines()[2],
            "Password on wake: asked for on mains, not asked for on battery"
        );
    }

    #[test]
    fn what_the_system_would_not_say_is_not_known_and_nothing_at_all_is_nothing() {
        let nothing = IdleTimes::read_by(|_, _| None);
        assert!(!nothing.is_known());
        let half = IdleTimes::read_by(|setting, power| {
            (setting == Setting::DisplaysOff && power == Power::Mains).then_some(120)
        });
        assert!(half.is_known());
        assert_eq!(
            half.lines(),
            [
                "Displays off: after 2 minutes on mains, not known on battery",
                "Sleep: not known",
            ]
        );
    }

    #[test]
    fn each_setting_has_a_name_of_its_own_in_the_power_plan() {
        let all = [
            Setting::DisplaysOff,
            Setting::Sleep,
            Setting::PasswordOnWake,
        ];
        for (index, one) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(guids(*one), guids(*other));
            }
        }
        // powercfg's VIDEOIDLE in SUB_VIDEO, by Windows' own numbers.
        assert_eq!(
            format!("{:032x}", guids(Setting::DisplaysOff).1),
            "3c0bc021c8a84e07a9736b14cbcb2b7e"
        );
    }
}
