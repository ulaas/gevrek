//! `desktop.outputs[].scale` on Windows: how large the system draws what
//! is on a display (100 %, 125 %, 150 % ...), set as the Settings window
//! sets it.
//!
//! Windows has no documented way for a program to set a display's scale.
//! What is used here is what the Settings window itself uses: two values
//! of `DisplayConfigGetDeviceInfo` and `DisplayConfigSetDeviceInfo` that
//! Windows' headers do not name (-3 to ask, -4 to set). They are known
//! from other programs that do the same and work on Windows 10 and 11;
//! a later Windows may answer them otherwise. Should it refuse, nothing
//! is changed and the log says so.
//!
//! Windows counts a display's scale in steps from the one it recommends
//! for that display: so many steps below it down to 100 %, so many above
//! it up to the largest it offers there.
//!
//! - A scale is set only for a display that an entry of `desktop.outputs`
//!   names and gives a scale, and only one the display offers.
//! - What is set is how the display is from then on, as after the
//!   Settings window. A display whose entry no longer gives a scale gets
//!   back the one it had before gevrek changed it (as long as the window
//!   manager that changed it still runs).

use crate::display_settings::Display;
use gevrek_model::outputs::OutputProfile;
use std::collections::HashMap;

/// The scales Windows knows, in per cent, in its own order.
pub const STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

/// A display's scale as Windows has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scale {
    /// In per cent.
    pub current: u32,
    /// What Windows recommends for this display.
    pub recommended: u32,
    /// What it offers there, the smallest first.
    pub offered: Vec<u32>,
}

impl Scale {
    /// From what Windows answers: the steps from the recommended scale
    /// down to the smallest (nought or less), to the one in use, and up
    /// to the largest. `None` for an answer that makes no sense.
    pub fn read(smallest: i32, current: i32, largest: i32) -> Option<Scale> {
        let recommended = usize::try_from(smallest.checked_neg()?).ok()?;
        if recommended >= STEPS.len() || largest < 0 {
            return None;
        }
        let last = recommended
            .saturating_add(largest as usize)
            .min(STEPS.len() - 1);
        // (A scale typed in by hand in the Settings window lies outside
        // the steps: the nearest end of them then.)
        let now = (recommended as i64 + i64::from(current)).clamp(0, last as i64) as usize;
        Some(Scale {
            current: STEPS[now],
            recommended: STEPS[recommended],
            offered: STEPS[..=last].to_vec(),
        })
    }

    /// The steps from the recommended scale to `percent`, as Windows is
    /// told; `None` where the display does not offer it.
    pub fn steps_to(&self, percent: u32) -> Option<i32> {
        let wanted = self.offered.iter().position(|step| *step == percent)?;
        let recommended = STEPS.iter().position(|step| *step == self.recommended)?;
        Some(wanted as i32 - recommended as i32)
    }
}

/// A scale to be set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Windows' name for the display (`\\.\DISPLAY1`).
    pub device: String,
    /// The name it is known by, for the log.
    pub name: String,
    /// In per cent.
    pub to: u32,
    /// The steps from the recommended scale.
    pub steps: i32,
    /// It is given back the scale it had before gevrek changed it.
    pub back: bool,
}

/// A scale of the configuration (1, 1.25, 1.5 ...) in per cent.
fn percent(scale: f64) -> u32 {
    (scale * 100.0).round().clamp(0.0, 10_000.0) as u32
}

/// What has to change so that the displays have the scales
/// `desktop.outputs` gives them. `before`: the scale each display had
/// before gevrek changed it (by Windows' name for it); a display that is
/// there and whose entry gives none any more gets that back. An error
/// says which entry asks for a scale its display does not offer; nothing
/// is to be changed then.
pub fn plan(
    profiles: &[OutputProfile],
    displays: &[(Display, Scale)],
    before: &HashMap<String, u32>,
) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    for (display, scale) in displays {
        let asked = profiles
            .iter()
            .filter(|profile| profile.enabled && crate::display_settings::means(profile, display))
            .find_map(|profile| profile.scale);
        let (to, back) = match (asked, before.get(&display.device)) {
            (Some(asked), _) => (percent(asked), false),
            (None, Some(before)) => (*before, true),
            (None, None) => continue,
        };
        let Some(steps) = scale.steps_to(to) else {
            if back {
                // (What it had is not offered any more: left as it is.)
                continue;
            }
            return Err(format!(
                "the display {} has no scale of {to} % (it offers {})",
                display.name,
                scale
                    .offered
                    .iter()
                    .map(|step| format!("{step} %"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        if to != scale.current || back {
            changes.push(Change {
                device: display.device.clone(),
                name: display.name.clone(),
                to,
                steps,
                back,
            });
        }
    }
    Ok(changes)
}

pub use system::{now, set};

mod system {
    use super::Scale;
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes,
        QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    use windows::Win32::Foundation::ERROR_SUCCESS;

    /// The two values Windows' headers do not name.
    const ASK_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-3);
    const SET_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-4);

    /// What Windows answers when asked for a display's scale: the steps
    /// from the recommended one to the smallest, the one in use, and the
    /// largest.
    #[repr(C)]
    #[derive(Default)]
    struct Asked {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
        smallest: i32,
        current: i32,
        largest: i32,
    }

    /// What Windows is told to set a display's scale: the steps from the
    /// recommended one.
    #[repr(C)]
    #[derive(Default)]
    struct Told {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
        steps: i32,
    }

    /// The header that names the display `device` (`\\.\DISPLAY1`) to the
    /// display configuration: its adapter and its number there.
    fn header_of(device: &str) -> Option<DISPLAYCONFIG_DEVICE_INFO_HEADER> {
        let (mut paths_there, mut modes_there) = (0u32, 0u32);
        // SAFETY: the two counts are ours.
        if unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_there, &mut modes_there)
        } != ERROR_SUCCESS
        {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_there as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_there as usize];
        // SAFETY: the lists are as long as the counts say; the system
        // says how many of each it filled.
        if unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut paths_there,
                paths.as_mut_ptr(),
                &mut modes_there,
                modes.as_mut_ptr(),
                None,
            )
        } != ERROR_SUCCESS
        {
            return None;
        }
        paths.truncate(paths_there as usize);
        paths.iter().find_map(|path| {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            // SAFETY: the structure is ours and says what it is and how
            // large.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
                return None;
            }
            let name = String::from_utf16_lossy(&source.viewGdiDeviceName);
            name.trim_end_matches('\0')
                .eq_ignore_ascii_case(device)
                .then_some(source.header)
        })
    }

    /// The scale of the display `device` as Windows has it; `None` where
    /// Windows does not say (the display is not there, or this Windows
    /// does not answer the question).
    pub fn now(device: &str) -> Option<Scale> {
        let mut asked = Asked {
            header: header_of(device)?,
            ..Default::default()
        };
        asked.header.r#type = ASK_SCALE;
        asked.header.size = std::mem::size_of::<Asked>() as u32;
        // SAFETY: the structure is ours, begins with the header and says
        // how large it is; the system fills what follows the header.
        if unsafe { DisplayConfigGetDeviceInfo(&mut asked.header) } != 0 {
            return None;
        }
        Scale::read(asked.smallest, asked.current, asked.largest)
    }

    /// Set the scale of the display `device`: so many steps from the one
    /// Windows recommends for it.
    pub fn set(device: &str, steps: i32) -> Result<(), String> {
        let mut told = Told {
            header: header_of(device).ok_or("Windows does not know the display")?,
            steps,
        };
        told.header.r#type = SET_SCALE;
        told.header.size = std::mem::size_of::<Told>() as u32;
        // SAFETY: the structure is ours, begins with the header and says
        // how large it is; the system only reads it.
        match unsafe { DisplayConfigSetDeviceInfo(&told.header) } {
            0 => Ok(()),
            code => Err(format!("Windows refused it (error {code})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::outputs::OutputTransform;

    fn display(device: &str, name: &str) -> Display {
        Display {
            device: device.to_string(),
            name: name.to_string(),
            path: format!(r"\\?\DISPLAY#{name}"),
            stable_id: String::new(),
            primary: false,
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            hertz: 60,
            turned: OutputTransform::Normal,
            modes: Vec::new(),
        }
    }

    fn entry(name: &str, scale: Option<f64>) -> OutputProfile {
        serde_json::from_value(serde_json::json!({"name": name, "scale": scale})).expect("an entry")
    }

    /// What Windows says of the displays of the machine the test runs
    /// on: asked only, nothing is set.
    ///
    ///     cargo test -p gevrek-win32 this_machine -- --ignored --nocapture
    #[test]
    #[ignore = "asks the machine's own displays; prints what they say"]
    fn the_displays_of_this_machine_say_their_scale() {
        for display in crate::display_settings::displays() {
            println!(
                "{} ({}): {:?}",
                display.name,
                display.device,
                now(&display.device)
            );
        }
    }

    #[test]
    fn windows_answer_is_read_as_a_scale() {
        // Recommended 150 % (two steps above 100), in use 125 %, up to 225 %.
        let scale = Scale::read(-2, -1, 3).unwrap();
        assert_eq!(scale.recommended, 150);
        assert_eq!(scale.current, 125);
        assert_eq!(scale.offered, [100, 125, 150, 175, 200, 225]);
        assert_eq!(scale.steps_to(100), Some(-2));
        assert_eq!(scale.steps_to(150), Some(0));
        assert_eq!(scale.steps_to(225), Some(3));
        assert_eq!(scale.steps_to(250), None, "not offered there");
        assert_eq!(scale.steps_to(130), None, "no step at all");

        // A display that offers 100 % only.
        let only = Scale::read(0, 0, 0).unwrap();
        assert_eq!((only.current, only.recommended), (100, 100));
        assert_eq!(only.offered, [100]);
        // A scale set by hand lies outside the steps: the nearest end.
        assert_eq!(Scale::read(-1, 9, 2).unwrap().current, 175);
        assert_eq!(Scale::read(-1, -9, 2).unwrap().current, 100);
        // The largest Windows knows is the last, whatever it says.
        assert_eq!(Scale::read(-10, 0, 9).unwrap().offered.last(), Some(&500));
        // Answers that make no sense.
        assert_eq!(Scale::read(1, 0, 2), None);
        assert_eq!(Scale::read(-12, 0, 0), None);
        assert_eq!(Scale::read(0, 0, -1), None);
        assert_eq!(Scale::read(i32::MIN, 0, 0), None);
    }

    #[test]
    fn a_scale_is_set_where_an_entry_gives_one_the_display_offers() {
        let displays = vec![
            (
                display(r"\\.\DISPLAY1", "Left"),
                Scale::read(-1, 0, 3).unwrap(),
            ),
            (
                display(r"\\.\DISPLAY2", "Right"),
                Scale::read(0, 0, 1).unwrap(),
            ),
        ];
        let nothing = HashMap::new();
        // Nothing is asked: nothing changes.
        assert_eq!(plan(&[], &displays, &nothing), Ok(Vec::new()));
        assert_eq!(
            plan(&[entry("Left", None)], &displays, &nothing),
            Ok(Vec::new())
        );
        // The scale it has already: nothing changes.
        assert_eq!(
            plan(&[entry("Left", Some(1.25))], &displays, &nothing),
            Ok(Vec::new())
        );

        // Another one: that display alone, counted from its recommended.
        let changes = plan(&[entry("left", Some(2.0))], &displays, &nothing).unwrap();
        assert_eq!(
            changes,
            [Change {
                device: r"\\.\DISPLAY1".to_string(),
                name: "Left".to_string(),
                to: 200,
                steps: 3,
                back: false,
            }]
        );
        // By Windows' own name for it too.
        let changes = plan(&[entry("DISPLAY2", Some(1.25))], &displays, &nothing).unwrap();
        assert_eq!(
            (changes[0].name.as_str(), changes[0].to, changes[0].steps),
            ("Right", 125, 1)
        );

        // One the display does not offer: said, and nothing is changed.
        let refused = plan(
            &[entry("Left", Some(2.0)), entry("Right", Some(2.0))],
            &displays,
            &nothing,
        )
        .unwrap_err();
        assert_eq!(
            refused,
            "the display Right has no scale of 200 % (it offers 100 %, 125 %)"
        );

        // An entry that is switched off says nothing.
        let mut off = entry("Left", Some(2.0));
        off.enabled = false;
        assert_eq!(plan(&[off], &displays, &nothing), Ok(Vec::new()));
    }

    #[test]
    fn a_display_whose_entry_gives_no_scale_any_more_gets_its_own_back() {
        // gevrek made it 200 %; it had 125 %.
        let displays = vec![(
            display(r"\\.\DISPLAY1", "Left"),
            Scale::read(-1, 3, 3).unwrap(),
        )];
        let before = HashMap::from([(r"\\.\DISPLAY1".to_string(), 125)]);
        // Still asked for: stays.
        assert_eq!(
            plan(&[entry("Left", Some(2.0))], &displays, &before),
            Ok(Vec::new())
        );
        // No longer: back to what it had.
        for profiles in [vec![], vec![entry("Left", None)]] {
            let changes = plan(&profiles, &displays, &before).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(
                (changes[0].to, changes[0].steps, changes[0].back),
                (125, 0, true)
            );
        }
        // What it had is not offered any more: left as it is.
        let before = HashMap::from([(r"\\.\DISPLAY1".to_string(), 500)]);
        assert_eq!(plan(&[], &displays, &before), Ok(Vec::new()));
    }
}
