//! `desktop.outputs` on Windows: where each display is among the others,
//! its mode and how its picture is turned, set through Windows' own display
//! settings.
//!
//! Windows owns the displays; gevrek asks it, as the Settings window does.
//! What is asked for:
//!
//! - Only for a display that an entry of `desktop.outputs` names (by the
//!   name `gevrek state` shows, the device path, or `DISPLAY1`) and says
//!   something about beyond `enabled`: a position, a mode, a turned picture,
//!   or that it is the main one. A display no entry says more of is as
//!   Windows has it, and stays so.
//! - Positions are in the pixels of the desktop, as Windows counts them
//!   (not divided by a scale; a display's scale is set by itself, in
//!   `display_scale.rs`). The main
//!   display is at the corner: all positions are moved together so that it
//!   is.
//! - A mode the display does not offer is not asked for; every change is
//!   first tried with Windows' own test (nothing happens on the screen),
//!   and nothing is set unless all of them pass.
//! - What is set is written to Windows' settings too: it is how the
//!   displays are from then on, as after the Settings window.
//!
//! A change that could leave a person in front of a dark screen (a mode, a
//! turned picture) is on trial, as on Linux: see `Runtime::outputs_changed`.

use gevrek_model::outputs::{OutputProfile, OutputTransform};

/// A display as Windows has it now.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// Windows' name for it (`\\.\DISPLAY1`).
    pub device: String,
    /// The name `gevrek state` shows, and the device path.
    pub name: String,
    pub path: String,
    /// Who it is wherever it is plugged in (`MonitorInfo::stable_id`).
    pub stable_id: String,
    pub primary: bool,
    pub x: i32,
    pub y: i32,
    /// The size of the picture as it is seen (turned, when it is).
    pub width: u32,
    pub height: u32,
    pub hertz: u32,
    pub turned: OutputTransform,
    /// The modes it offers, as the display has them (not turned): width,
    /// height, hertz.
    pub modes: Vec<(u32, u32, u32)>,
}

/// What a display is to be.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub device: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hertz: u32,
    pub turned: OutputTransform,
    pub primary: bool,
}

/// An entry that says more of its display than whether it is used.
fn arranges(profile: &OutputProfile) -> bool {
    profile.enabled
        && (profile.position.is_some()
            || profile.mode.is_some()
            || profile.transform != OutputTransform::Normal
            || profile.primary)
}

pub(crate) fn means(profile: &OutputProfile, display: &Display) -> bool {
    let wanted = profile.name.trim();
    let short = display.device.trim_start_matches(r"\\.\");
    [&display.name, &display.path, &display.device]
        .into_iter()
        .any(|known| known.eq_ignore_ascii_case(wanted))
        || short.eq_ignore_ascii_case(wanted)
        || gevrek_engine::outputs::name_without_number(&display.name).eq_ignore_ascii_case(wanted)
}

fn sideways(turned: OutputTransform) -> bool {
    matches!(turned, OutputTransform::Left | OutputTransform::Right)
}

/// What has to change so that the displays are as `desktop.outputs` says:
/// the displays that are to be other than they are (none: nothing to do).
/// An error says which entry asks for what its display cannot do.
pub fn plan(profiles: &[OutputProfile], displays: &[Display]) -> Result<Vec<Wanted>, String> {
    let entry = |display: &Display| {
        profiles
            .iter()
            .find(|profile| arranges(profile) && means(profile, display))
    };
    if !displays.iter().any(|display| entry(display).is_some()) {
        return Ok(Vec::new());
    }
    let mut wanted = Vec::new();
    for display in displays {
        // As it is, unless its entry says otherwise.
        let (own_width, own_height) = if sideways(display.turned) {
            (display.height, display.width)
        } else {
            (display.width, display.height)
        };
        let mut now = Wanted {
            device: display.device.clone(),
            x: display.x,
            y: display.y,
            width: own_width,
            height: own_height,
            hertz: display.hertz,
            turned: display.turned,
            primary: display.primary,
        };
        if let Some(profile) = entry(display) {
            if let Some((x, y)) = profile.position {
                (now.x, now.y) = (x, y);
            }
            if let Some(mode) = &profile.mode {
                let at_size: Vec<u32> = display
                    .modes
                    .iter()
                    .filter(|(width, height, _)| (*width, *height) == (mode.width, mode.height))
                    .map(|(_, _, hertz)| *hertz)
                    .collect();
                let hertz = if mode.refresh > 0.0 {
                    at_size
                        .iter()
                        .copied()
                        .find(|hertz| (f64::from(*hertz) - mode.refresh).abs() < 0.6)
                } else {
                    at_size.iter().copied().max()
                };
                let Some(hertz) = hertz else {
                    return Err(format!(
                        "the display {} has no mode {}x{}{}",
                        display.name,
                        mode.width,
                        mode.height,
                        if mode.refresh > 0.0 {
                            format!(" at {} Hz", mode.refresh)
                        } else {
                            String::new()
                        }
                    ));
                };
                (now.width, now.height, now.hertz) = (mode.width, mode.height, hertz);
            }
            now.turned = profile.transform;
        }
        // (Kept as the display has it until here; from here as it is seen.)
        if sideways(now.turned) {
            (now.width, now.height) = (now.height, now.width);
        }
        wanted.push(now);
    }
    // The main display: the one an entry says, else the one that is.
    if let Some(main) = displays
        .iter()
        .position(|display| entry(display).is_some_and(|profile| profile.primary))
    {
        for (index, display) in wanted.iter_mut().enumerate() {
            display.primary = index == main;
        }
    }
    // It is at the corner; the others keep their places from it.
    if let Some((x, y)) = wanted
        .iter()
        .find(|display| display.primary)
        .map(|display| (display.x, display.y))
    {
        for display in &mut wanted {
            display.x -= x;
            display.y -= y;
        }
    }
    Ok(wanted
        .into_iter()
        .zip(displays)
        .filter(|(wanted, display)| {
            (wanted.x, wanted.y, wanted.width, wanted.height)
                != (display.x, display.y, display.width, display.height)
                || wanted.hertz != display.hertz
                || wanted.turned != display.turned
                || wanted.primary != display.primary
        })
        .map(|(wanted, _)| wanted)
        .collect())
}

/// The displays' settings as they were, to put them back.
#[derive(Clone)]
pub struct Saved(Vec<(String, windows::Win32::Graphics::Gdi::DEVMODEW, bool)>);

/// Whether `desktop.outputs` asks anything of how a display that is
/// there is arranged (its place, its mode, its turn, which is the main
/// one). When it asks nothing any more, the displays are given back the
/// settings they had before gevrek changed them.
pub fn arranges_any(profiles: &[OutputProfile], displays: &[Display]) -> bool {
    displays.iter().any(|display| {
        profiles
            .iter()
            .any(|profile| arranges(profile) && means(profile, display))
    })
}

pub use system::{displays, modes_of, restore, set, test};

mod system {
    use super::{Display, Saved, Wanted};
    use gevrek_model::outputs::OutputTransform;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::POINTL;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplaySettingsExW, CDS_NORESET, CDS_SET_PRIMARY, CDS_TEST,
        CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE, DISP_CHANGE_SUCCESSFUL, DMDO_180,
        DMDO_270, DMDO_90, DMDO_DEFAULT, DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION,
        DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS,
        ENUM_DISPLAY_SETTINGS_MODE,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn settings_of(device: &str, which: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
        let name = wide(device);
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: the name ends with a zero; the structure says its size.
        unsafe { EnumDisplaySettingsExW(
                PCWSTR(name.as_ptr()),
                which,
                &mut mode,
                windows::Win32::Graphics::Gdi::ENUM_DISPLAY_SETTINGS_FLAGS(0),
            ) }
            .as_bool()
            .then_some(mode)
    }

    /// The modes a display offers: width, height, hertz, each once.
    pub fn modes_of(device: &str) -> Vec<(u32, u32, u32)> {
        let mut modes: Vec<(u32, u32, u32)> = (0..)
            .map_while(|index| settings_of(device, ENUM_DISPLAY_SETTINGS_MODE(index)))
            .filter(|mode| mode.dmBitsPerPel >= 24)
            .map(|mode| (mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency))
            .collect();
        modes.sort_unstable();
        modes.dedup();
        modes
    }

    /// The displays of the desktop as Windows has them now, each with the
    /// name and path the window manager knows it by.
    pub fn displays() -> Vec<Display> {
        // (All of them: one that is switched off in `desktop.outputs` is
        // still part of Windows' arrangement.)
        let known = crate::dwm::monitor::all_displays();
        known
            .into_iter()
            .filter_map(|(device, info)| {
                let now = settings_of(&device, ENUM_CURRENT_SETTINGS)?;
                // SAFETY: a display's settings hold these two.
                let (at, turned) = unsafe {
                    (
                        now.Anonymous1.Anonymous2.dmPosition,
                        now.Anonymous1.Anonymous2.dmDisplayOrientation,
                    )
                };
                Some(Display {
                    modes: modes_of(&device),
                    device,
                    name: info.name,
                    path: info.device_path,
                    stable_id: info.stable_id,
                    primary: info.is_primary,
                    x: at.x,
                    y: at.y,
                    width: now.dmPelsWidth,
                    height: now.dmPelsHeight,
                    hertz: now.dmDisplayFrequency,
                    turned: match turned {
                        DMDO_90 => OutputTransform::Left,
                        DMDO_180 => OutputTransform::UpsideDown,
                        DMDO_270 => OutputTransform::Right,
                        _ => OutputTransform::Normal,
                    },
                })
            })
            .collect()
    }

    fn asked(wanted: &Wanted) -> Option<DEVMODEW> {
        let mut mode = settings_of(&wanted.device, ENUM_CURRENT_SETTINGS)?;
        mode.dmFields = DM_POSITION
            | DM_PELSWIDTH
            | DM_PELSHEIGHT
            | DM_DISPLAYFREQUENCY
            | DM_DISPLAYORIENTATION;
        mode.dmPelsWidth = wanted.width;
        mode.dmPelsHeight = wanted.height;
        mode.dmDisplayFrequency = wanted.hertz;
        mode.Anonymous1.Anonymous2.dmPosition = POINTL {
            x: wanted.x,
            y: wanted.y,
        };
        // (Windows counts the turns the other way round than the model
        // names them: its 90 is the picture turned to the left.)
        mode.Anonymous1.Anonymous2.dmDisplayOrientation = match wanted.turned {
            OutputTransform::Normal => DMDO_DEFAULT,
            OutputTransform::Left => DMDO_90,
            OutputTransform::UpsideDown => DMDO_180,
            OutputTransform::Right => DMDO_270,
        };
        Some(mode)
    }

    fn change(device: &str, mode: &DEVMODEW, flags: CDS_TYPE) -> DISP_CHANGE {
        let name = wide(device);
        // SAFETY: the name ends with a zero; Windows only reads the mode.
        unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(mode), None, flags, None) }
    }

    fn why(answer: DISP_CHANGE) -> String {
        match answer.0 {
            -2 => "the display cannot show this mode".to_string(),
            -1 => "the display's driver failed".to_string(),
            -3 => "the settings could not be written".to_string(),
            -5 => "the settings are not valid together".to_string(),
            1 => "Windows wants to be started again for it".to_string(),
            other => format!("Windows said {other}"),
        }
    }

    /// Ask Windows whether it would take these changes; nothing happens on
    /// the screen. An error names the display and what Windows said.
    pub fn test(wanted: &[Wanted]) -> Result<(), String> {
        for display in wanted {
            let mode = asked(display)
                .ok_or_else(|| format!("{} is not a display of the desktop", display.device))?;
            let answer = change(&display.device, &mode, CDS_TEST);
            if answer != DISP_CHANGE_SUCCESSFUL {
                return Err(format!(
                    "{}: {}x{} at {} Hz at ({}, {}): {}",
                    display.device,
                    display.width,
                    display.height,
                    display.hertz,
                    display.x,
                    display.y,
                    why(answer)
                ));
            }
        }
        Ok(())
    }

    /// Make the displays what is wanted, all at once. Tried first
    /// ([`test`]): unless Windows would take all of it, nothing is set.
    /// What comes back puts the displays back as they were.
    pub fn set(wanted: &[Wanted]) -> Result<Saved, String> {
        test(wanted)?;
        let saved = Saved(
            wanted
                .iter()
                .filter_map(|display| {
                    let was = settings_of(&display.device, ENUM_CURRENT_SETTINGS)?;
                    let primary = crate::dwm::monitor::all_displays()
                        .into_iter()
                        .any(|(device, info)| device == display.device && info.is_primary);
                    Some((display.device.clone(), was, primary))
                })
                .collect(),
        );
        for display in wanted {
            let Some(mode) = asked(display) else {
                continue;
            };
            let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
            if display.primary {
                flags |= CDS_SET_PRIMARY;
            }
            let answer = change(&display.device, &mode, flags);
            if answer != DISP_CHANGE_SUCCESSFUL {
                // What was written so far is taken back.
                restore(&saved);
                return Err(format!("{}: {}", display.device, why(answer)));
            }
        }
        carry_out()?;
        Ok(saved)
    }

    /// What was written for the single displays becomes what is shown.
    fn carry_out() -> Result<(), String> {
        // SAFETY: no display and no mode: "apply what was written".
        let answer = unsafe { ChangeDisplaySettingsExW(None, None, None, CDS_TYPE(0), None) };
        if answer == DISP_CHANGE_SUCCESSFUL {
            Ok(())
        } else {
            Err(why(answer))
        }
    }

    /// Put the displays back as they were before [`set`].
    pub fn restore(saved: &Saved) -> bool {
        let mut all = true;
        for (device, mode, primary) in &saved.0 {
            let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
            if *primary {
                flags |= CDS_SET_PRIMARY;
            }
            all &= change(device, mode, flags) == DISP_CHANGE_SUCCESSFUL;
        }
        all && carry_out().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::outputs::OutputMode;

    fn two() -> Vec<Display> {
        let modes = vec![(1920, 1080, 60), (2560, 1440, 60), (2560, 1440, 144)];
        vec![
            Display {
                device: r"\\.\DISPLAY1".to_string(),
                name: "Monitor ACM2401".to_string(),
                path: r"\\?\DISPLAY#ACM2401#one".to_string(),
                stable_id: String::new(),
                primary: true,
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                hertz: 144,
                turned: OutputTransform::Normal,
                modes: modes.clone(),
            },
            Display {
                device: r"\\.\DISPLAY2".to_string(),
                name: "Monitor ACM2702".to_string(),
                path: r"\\?\DISPLAY#ACM2702#two".to_string(),
                stable_id: String::new(),
                primary: false,
                x: 2560,
                y: 0,
                width: 2560,
                height: 1440,
                hertz: 60,
                turned: OutputTransform::Normal,
                modes,
            },
        ]
    }

    fn entry(name: &str) -> OutputProfile {
        OutputProfile {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_configuration_asks_something_of_the_displays_or_nothing() {
        // Nothing: no entry, one that only switches a display off or on,
        // one for a display that is not there.
        assert!(!arranges_any(&[], &two()));
        let mut off = entry("Monitor ACM2702");
        off.enabled = false;
        assert!(!arranges_any(&[entry("Monitor ACM2401"), off], &two()));
        let mut elsewhere = entry("HDMI-A-1");
        elsewhere.position = Some((0, 0));
        assert!(!arranges_any(&[elsewhere], &two()));
        // Something: a mode, even the one the display has (it is asked
        // for, and the display's own settings are not given back).
        let mut same = entry("DISPLAY2");
        same.mode = Some(OutputMode {
            width: 2560,
            height: 1440,
            refresh: 60.0,
        });
        assert!(arranges_any(&[same], &two()));
    }

    #[test]
    fn displays_no_entry_says_more_of_are_left_as_windows_has_them() {
        assert_eq!(plan(&[], &two()), Ok(Vec::new()));
        // Named, and only switched on or off: Windows' own business.
        let mut off = entry("Monitor ACM2702");
        off.enabled = false;
        assert_eq!(plan(&[entry("Monitor ACM2401"), off], &two()), Ok(Vec::new()));
        // Said as it is: nothing to do.
        let mut same = entry("DISPLAY2");
        same.position = Some((2560, 0));
        same.mode = Some(OutputMode {
            width: 2560,
            height: 1440,
            refresh: 60.0,
        });
        assert_eq!(plan(&[same], &two()), Ok(Vec::new()));
        // A name no display has.
        let mut other = entry("HDMI-A-1");
        other.position = Some((0, 0));
        assert_eq!(plan(&[other], &two()), Ok(Vec::new()));
    }

    #[test]
    fn a_position_a_mode_and_a_turn_are_asked_for_the_display_named() {
        let mut second = entry("monitor acm2702");
        second.position = Some((-1440, 0));
        second.mode = Some(OutputMode {
            width: 2560,
            height: 1440,
            refresh: 0.0,
        });
        second.transform = OutputTransform::Right;
        let wanted = plan(&[second], &two()).expect("a plan");
        // Only the second display changes: to the left of the first, at
        // the highest rate of that size, standing on its side.
        assert_eq!(
            wanted,
            vec![Wanted {
                device: r"\\.\DISPLAY2".to_string(),
                x: -1440,
                y: 0,
                width: 1440,
                height: 2560,
                hertz: 144,
                turned: OutputTransform::Right,
                primary: false,
            }]
        );
    }

    #[test]
    fn the_main_display_is_at_the_corner() {
        // The second is to be the main one: it goes to the corner and the
        // first keeps its place from it.
        let mut second = entry(r"\\?\display#acm2702#two");
        second.primary = true;
        let wanted = plan(&[second], &two()).expect("a plan");
        let places: Vec<(i32, i32, bool)> = wanted
            .iter()
            .map(|display| (display.x, display.y, display.primary))
            .collect();
        assert_eq!(places, [(-2560, 0, false), (0, 0, true)]);
    }

    #[test]
    fn a_mode_the_display_does_not_offer_is_said_and_nothing_is_asked() {
        let mut first = entry("Monitor ACM2401");
        first.mode = Some(OutputMode {
            width: 1234,
            height: 777,
            refresh: 0.0,
        });
        let refused = plan(&[first.clone()], &two()).expect_err("no such mode");
        assert!(refused.contains("1234x777"), "{refused}");
        // A rate the size does not have.
        first.mode = Some(OutputMode {
            width: 1920,
            height: 1080,
            refresh: 144.0,
        });
        assert!(plan(&[first.clone()], &two()).is_err());
        // One that is near enough to the display's own is that one.
        first.mode = Some(OutputMode {
            width: 2560,
            height: 1440,
            refresh: 59.951,
        });
        let wanted = plan(&[first], &two()).expect("a plan");
        assert_eq!(wanted[0].hertz, 60);
    }
}
