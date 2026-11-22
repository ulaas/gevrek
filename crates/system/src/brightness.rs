//! How bright the display is, where the machine can set that (a laptop's
//! own display: its backlight).
//!
//! Linux names its backlights under `/sys/class/backlight`, each with the
//! brightness it has and the most it can have. Reading is anybody's;
//! setting is root's, or logind's for the user at the machine
//! (`SetBrightness` of the session), which is how it is set here. A
//! desktop machine's displays have no backlight of that kind: there is
//! nothing to set, and that is said.

use std::path::Path;
#[cfg(not(any(windows, target_os = "macos")))]
use std::path::PathBuf;

/// How bright a display is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brightness {
    pub now: u32,
    pub most: u32,
}

impl Brightness {
    /// In hundredths of the most, rounded.
    pub fn percent(&self) -> u32 {
        if self.most == 0 {
            return 0;
        }
        ((u64::from(self.now.min(self.most)) * 100 + u64::from(self.most) / 2)
            / u64::from(self.most)) as u32
    }

    /// The brightness after a step of `by_percent` hundredths of the
    /// most: never beyond the most, and never darker than one hundredth
    /// (a display that is off cannot be read to switch it on again). A
    /// step moves by at least one where it can.
    pub fn stepped(&self, by_percent: i32) -> u32 {
        if self.most == 0 {
            return 0;
        }
        let least = (self.most / 100).max(1);
        let step = (i64::from(self.most) * i64::from(by_percent.unsigned_abs()) / 100).max(1);
        let wanted = if by_percent >= 0 {
            i64::from(self.now) + step
        } else {
            i64::from(self.now) - step
        };
        wanted.clamp(i64::from(least), i64::from(self.most)) as u32
    }
}

/// Where the system lists its backlights (`GEVREK_BACKLIGHT_DIR` names
/// another place: the checks have a folder of their own).
#[cfg(not(any(windows, target_os = "macos")))]
fn backlights() -> PathBuf {
    std::env::var_os("GEVREK_BACKLIGHT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/sys/class/backlight"))
}

fn number(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The backlight to set, of those in `folder`: one the machine's
/// firmware gives before one of the platform before a driver's own (the
/// order the kernel's documentation asks for), by name among equals.
pub fn device_in(folder: &Path) -> Option<(String, Brightness)> {
    let rank = |kind: &str| match kind.trim() {
        "firmware" => 0,
        "platform" => 1,
        "raw" => 2,
        _ => 3,
    };
    let mut found: Vec<(i32, String, Brightness)> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let most = number(&path.join("max_brightness")).filter(|most| *most > 0)?;
            let now = number(&path.join("brightness"))?;
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            Some((
                rank(&kind),
                entry.file_name().to_string_lossy().into_owned(),
                Brightness { now, most },
            ))
        })
        .collect();
    found.sort_by(|one, other| (one.0, &one.1).cmp(&(other.0, &other.1)));
    found
        .into_iter()
        .next()
        .map(|(_, name, brightness)| (name, brightness))
}

/// How bright the machine's own display is; `None`: it has none whose
/// brightness can be set.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn read() -> Option<Brightness> {
    device_in(&backlights()).map(|(_, brightness)| brightness)
}

/// The built-in display's brightness, through DisplayServices (the
/// system's own, loaded when it is first asked: it is not in the public
/// frameworks). `None` on a Mac without a built-in display, or where it is
/// not there.
#[cfg(target_os = "macos")]
mod mac {
    use super::Brightness;
    use std::ffi::{c_void, CString};
    use std::sync::OnceLock;

    type Get = unsafe extern "C" fn(u32, *mut f32) -> i32;
    type Set = unsafe extern "C" fn(u32, f32) -> i32;

    extern "C" {
        fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const i8) -> *mut c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(max: u32, out: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayIsBuiltin(display: u32) -> u32;
    }

    fn calls() -> Option<(Get, Set)> {
        static CALLS: OnceLock<Option<(usize, usize)>> = OnceLock::new();
        let (get, set) = (*CALLS.get_or_init(|| {
            let path = CString::new("/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices").ok()?;
            // SAFETY: the framework loaded once and kept; its two calls
            // looked up by name.
            unsafe {
                let handle = dlopen(path.as_ptr(), 1);
                if handle.is_null() {
                    return None;
                }
                let get = dlsym(handle, c"DisplayServicesGetBrightness".as_ptr());
                let set = dlsym(handle, c"DisplayServicesSetBrightness".as_ptr());
                (!get.is_null() && !set.is_null()).then_some((get as usize, set as usize))
            }
        }))?;
        // SAFETY: the calls' signatures, as every tool that uses them has
        // them.
        Some(unsafe { (std::mem::transmute::<usize, Get>(get), std::mem::transmute::<usize, Set>(set)) })
    }

    fn built_in() -> Option<u32> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        // SAFETY: room for 16 ids.
        unsafe {
            CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count);
            ids[..count as usize].iter().copied().find(|id| CGDisplayIsBuiltin(*id) != 0)
        }
    }

    pub fn read() -> Option<Brightness> {
        let (get, _) = calls()?;
        let display = built_in()?;
        let mut level = 0f32;
        // SAFETY: a value written by the call.
        (unsafe { get(display, &mut level) } == 0).then(|| Brightness {
            now: (level * 100.0).round() as u32,
            most: 100,
        })
    }

    pub fn change(by_percent: i32) -> Option<Brightness> {
        let (_, set) = calls()?;
        let display = built_in()?;
        let now = read()?;
        let wanted = (now.now as i32 + by_percent).clamp(1, 100);
        // SAFETY: a plain call.
        unsafe { set(display, wanted as f32 / 100.0) };
        read()
    }
}

#[cfg(target_os = "macos")]
pub use mac::{change, read};

/// How bright the machine's own display is, as Windows' display driver
/// says (a laptop's panel; a desktop machine's displays have nothing of
/// the kind, and `None` says so).
#[cfg(windows)]
pub fn read() -> Option<Brightness> {
    let panel = lcd::Panel::open()?;
    panel.now().map(|now| Brightness {
        now: u32::from(now),
        most: 100,
    })
}

/// Make the display brighter or darker by so many hundredths, to one of
/// the levels its driver offers. The answer is how bright it is
/// afterwards; `None`: there is nothing to set, or it could not be set.
#[cfg(windows)]
pub fn change(by_percent: i32) -> Option<Brightness> {
    let panel = lcd::Panel::open()?;
    let now = panel.now()?;
    let wanted = level_after(&panel.levels(), now, by_percent);
    if wanted != now && !panel.set(wanted) {
        log::warn!("the brightness was not set");
        return None;
    }
    Some(Brightness {
        now: u32::from(wanted),
        most: 100,
    })
}

/// The level a display is at after a step of `by_percent` from `now`,
/// among the levels its driver offers (hundredths of the most): the one
/// nearest to where the step leads, and another than `now` where there is
/// one in that direction; never the darkest when that is nothing at all
/// (a display that is off cannot be read to switch it on again). A driver
/// that names no levels takes every hundredth.
pub fn level_after(levels: &[u8], now: u8, by_percent: i32) -> u8 {
    let target = (i32::from(now) + by_percent).clamp(1, 100);
    let brighter = by_percent >= 0;
    let all: Vec<u8> = (1..=100).collect();
    let offered: &[u8] = if levels.is_empty() { &all } else { levels };
    offered
        .iter()
        .copied()
        .filter(|level| *level > 0 && *level <= 100)
        .filter(|level| if brighter { *level > now } else { *level < now })
        .min_by_key(|level| (i32::from(*level) - target).abs())
        .unwrap_or(now)
}

#[cfg(windows)]
mod lcd {
    //! The display driver's own word on the panel's backlight
    //! (`\\.\LCD`): the levels it offers, the one in use, and setting one.

    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::IO::DeviceIoControl;

    const QUERY_SUPPORTED: u32 = 0x0023_2494;
    const QUERY: u32 = 0x0023_2498;
    const SET: u32 = 0x0023_249C;
    /// Reading and writing (`GENERIC_READ | GENERIC_WRITE`).
    const READ_WRITE: u32 = 0xC000_0000;
    /// The level counts on the mains and on the battery alike.
    const BOTH: u8 = 3;

    pub struct Panel(HANDLE);

    impl Panel {
        /// The machine's panel; `None`: it has none (a desktop machine).
        pub fn open() -> Option<Self> {
            // SAFETY: a device of the system is opened by its name.
            let handle = unsafe {
                CreateFileW(
                    w!("\\\\.\\LCD"),
                    READ_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            }
            .ok()?;
            Some(Self(handle))
        }

        fn ask(&self, what: u32, given: Option<&[u8]>, answer: &mut [u8]) -> Option<usize> {
            let mut written = 0u32;
            // SAFETY: both buffers are as long as is said of them.
            unsafe {
                DeviceIoControl(
                    self.0,
                    what,
                    given.map(|given| given.as_ptr().cast()),
                    given.map_or(0, |given| given.len() as u32),
                    Some(answer.as_mut_ptr().cast()),
                    answer.len() as u32,
                    Some(&mut written),
                    None,
                )
            }
            .ok()?;
            Some(written as usize)
        }

        /// The levels the driver offers, in hundredths of the most.
        pub fn levels(&self) -> Vec<u8> {
            let mut answer = [0u8; 256];
            match self.ask(QUERY_SUPPORTED, None, &mut answer) {
                Some(count) => answer[..count.min(answer.len())].to_vec(),
                None => Vec::new(),
            }
        }

        /// The level in use: the one for the mains while the machine is
        /// on them, else the one for the battery.
        pub fn now(&self) -> Option<u8> {
            // (Which of the two counts, the level on the mains, the level
            // on the battery.)
            let mut answer = [0u8; 3];
            self.ask(QUERY, None, &mut answer).filter(|count| *count >= 3)?;
            let mut status = SYSTEM_POWER_STATUS::default();
            // SAFETY: the call fills in the structure it is given.
            let on_mains =
                unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 1;
            Some(if on_mains { answer[1] } else { answer[2] })
        }

        pub fn set(&self, level: u8) -> bool {
            let mut nothing = [0u8; 0];
            self.ask(SET, Some(&[BOTH, level, level]), &mut nothing)
                .is_some()
        }
    }

    impl Drop for Panel {
        fn drop(&mut self) {
            // SAFETY: the handle is this value's own.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Make the display brighter or darker by so many hundredths. The answer
/// is how bright it is afterwards; `None`: there is nothing to set, or it
/// could not be set.
#[cfg(target_os = "linux")]
pub fn change(by_percent: i32) -> Option<Brightness> {
    // One change at a time: two that read the same brightness would be
    // one step between them.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let folder = backlights();
    let (name, before) = device_in(&folder)?;
    let wanted = before.stepped(by_percent);
    if wanted != before.now {
        let file = folder.join(&name).join("brightness");
        // The file itself where it may be written (a rule of the system
        // that gives it to the user, or a folder of the checks'), else
        // through logind, which sets it for the user at the machine.
        if let Err(error) = std::fs::write(&file, wanted.to_string()) {
            log::debug!("{} was not written ({error}): logind is asked", file.display());
            let asked = std::process::Command::new("busctl")
                .args([
                    "call",
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                    "SetBrightness",
                    "ssu",
                    "backlight",
                    &name,
                    &wanted.to_string(),
                ])
                .output();
            match asked {
                Ok(output) if output.status.success() => {
                    log::debug!("logind set the brightness of {name} to {wanted}");
                }
                Ok(output) => {
                    log::warn!(
                        "the brightness was not set: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    return None;
                }
                Err(error) => {
                    log::warn!("the brightness was not set (busctl): {error}");
                    return None;
                }
            }
        }
    }
    device_in(&folder)
        .map(|(_, brightness)| brightness)
        // (Read back at once it may still say the old value.)
        .map(|brightness| Brightness {
            now: if brightness.now == before.now {
                wanted
            } else {
                brightness.now
            },
            ..brightness
        })
}

/// Not set by gevrek here: the system's own keys do it.
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub fn change(_by_percent: i32) -> Option<Brightness> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_stays_between_one_hundredth_and_the_most() {
        let half = Brightness {
            now: 500,
            most: 1000,
        };
        assert_eq!(half.percent(), 50);
        assert_eq!(half.stepped(5), 550);
        assert_eq!(half.stepped(-5), 450);
        assert_eq!(
            Brightness {
                now: 980,
                most: 1000
            }
            .stepped(5),
            1000
        );
        // Never dark altogether.
        assert_eq!(
            Brightness {
                now: 20,
                most: 1000
            }
            .stepped(-5),
            10
        );
        assert_eq!(
            Brightness {
                now: 10,
                most: 1000
            }
            .stepped(-5),
            10
        );
        // A backlight of few steps moves by one at least.
        let coarse = Brightness { now: 3, most: 7 };
        assert_eq!(coarse.stepped(5), 4);
        assert_eq!(coarse.stepped(-5), 2);
        assert_eq!(Brightness { now: 1, most: 7 }.stepped(-5), 1);
        assert_eq!(Brightness { now: 0, most: 0 }.stepped(5), 0);
        assert_eq!(Brightness { now: 7, most: 7 }.percent(), 100);
        assert_eq!(Brightness { now: 1, most: 3 }.percent(), 33);
    }

    #[test]
    fn a_step_lands_on_a_level_the_driver_offers() {
        let levels = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(level_after(&levels, 50, 5), 60);
        assert_eq!(level_after(&levels, 50, -5), 40);
        assert_eq!(level_after(&levels, 50, 20), 70);
        // At the ends it stays; and never dark altogether.
        assert_eq!(level_after(&levels, 100, 5), 100);
        assert_eq!(level_after(&levels, 10, -5), 10);
        // A driver that names no levels takes every hundredth.
        assert_eq!(level_after(&[], 50, 5), 55);
        assert_eq!(level_after(&[], 3, -5), 1);
        assert_eq!(level_after(&[], 1, -5), 1);
    }

    #[test]
    fn the_firmwares_backlight_is_taken_before_a_drivers() {
        let folder = std::env::temp_dir().join(format!("gevrek-backlight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let make = |name: &str, kind: &str, now: &str, most: &str| {
            let device = folder.join(name);
            std::fs::create_dir_all(&device).expect("a folder");
            std::fs::write(device.join("type"), kind).expect("a file");
            std::fs::write(device.join("brightness"), now).expect("a file");
            std::fs::write(device.join("max_brightness"), most).expect("a file");
        };
        assert_eq!(device_in(&folder), None);
        make("intel_backlight", "raw\n", "300\n", "1000\n");
        make("acpi_video0", "firmware\n", "5\n", "10\n");
        make("broken", "firmware\n", "x\n", "10\n");
        make("nothing", "firmware\n", "0\n", "0\n");
        let (name, brightness) = device_in(&folder).expect("a backlight");
        assert_eq!(name, "acpi_video0");
        assert_eq!(brightness, Brightness { now: 5, most: 10 });
        let _ = std::fs::remove_dir_all(&folder);
    }
}
