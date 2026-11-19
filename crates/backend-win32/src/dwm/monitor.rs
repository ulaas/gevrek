//! Windows side of the monitor model: enumeration, HMONITOR conversions,
//! and the OS lookups. The model itself (`gevrek_engine::outputs::Outputs`,
//! re-exported as `MonitorManager`) is backend-neutral.

use anyhow::Result;
use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

pub use gevrek_engine::outputs::{
    MonitorChanges, MonitorHandle, MonitorInfo, Outputs as MonitorManager, StaleWindow,
    WorkspaceSettings,
};

/// HMONITOR conversions. Windows-only, so they live here.
pub trait HmonitorExt {
    fn to_hmonitor(self) -> HMONITOR;
    fn from_hmonitor(hmonitor: HMONITOR) -> Self;
}

impl HmonitorExt for MonitorHandle {
    fn to_hmonitor(self) -> HMONITOR {
        HMONITOR(self.0 as *mut core::ffi::c_void)
    }

    fn from_hmonitor(hmonitor: HMONITOR) -> Self {
        MonitorHandle(hmonitor.0 as isize)
    }
}

/// The displays the configuration switches off (`desktop.outputs`, an entry
/// with `enabled: false`), by the names they are given there.
static UNMANAGED_NAMES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
/// The displays the configuration names to be used (an entry that is not
/// switched off): a display that is no screen is used when it is among
/// them.
static WANTED_NAMES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
/// The handles those displays have at the moment (since the last
/// enumeration).
static UNMANAGED_HANDLES: std::sync::RwLock<Vec<isize>> = std::sync::RwLock::new(Vec::new());

/// The taskbar is hidden by the window manager (`crate::taskbar`): its
/// strip on each display is room for windows.
static TASKBAR_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Say whether the taskbar is hidden. Returns whether that is news (the
/// monitors then have to be looked at again).
pub fn set_taskbar_hidden(hidden: bool) -> bool {
    TASKBAR_HIDDEN.swap(hidden, std::sync::atomic::Ordering::Relaxed) != hidden
}

/// Each display's room for windows as it is while the taskbar is hidden.
fn give_taskbar_strips(all: &mut [MonitorInfo]) {
    if !TASKBAR_HIDDEN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let strips = crate::taskbar::strips();
    for info in all {
        let (x, y, width, height) = crate::taskbar::work_with_strips(
            (info.x, info.y, info.width, info.height),
            (info.work_x, info.work_y, info.work_width, info.work_height),
            &strips,
        );
        (info.work_x, info.work_y, info.work_width, info.work_height) = (x, y, width, height);
    }
}

/// Take the displays that are switched off from the configuration. On
/// Windows the display stays part of the desktop (that is the system's to
/// decide); gevrek leaves it alone: it is no monitor to the window manager,
/// has no workspaces, and the windows on it are not managed.
///
/// Returns whether the set changed (the monitors then have to be
/// enumerated again).
pub fn set_unmanaged_outputs(outputs: &[gevrek_model::outputs::OutputProfile]) -> bool {
    let names_where = |enabled: bool| {
        let mut names: Vec<String> = outputs
            .iter()
            .filter(|output| output.enabled == enabled)
            .map(|output| output.name.trim().to_lowercase())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    };
    let mut changed = false;
    for (kept, names) in [
        (&UNMANAGED_NAMES, names_where(false)),
        (&WANTED_NAMES, names_where(true)),
    ] {
        let mut current = kept.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current != names {
            *current = names;
            changed = true;
        }
    }
    changed
}

/// The displays of the desktop that are no screens, by Windows' own
/// names for them (`\\.\DISPLAY3`): those a program makes up (a
/// headset's desktop, a display for a film sent elsewhere, a screen
/// that is not there). Windows says of each display how it is connected;
/// these are the ones connected to nothing. A display on a dock that
/// goes through USB is a screen and is not among them.
fn displays_that_are_no_screens() -> Vec<String> {
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    };
    active_paths()
        .iter()
        .filter(|path| {
            path.targetInfo.outputTechnology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL
        })
        .filter_map(|path| {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            // SAFETY: the structure is ours and says what it is and how
            // large.
            (unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } == 0).then(|| {
                String::from_utf16_lossy(&source.viewGdiDeviceName)
                    .trim_end_matches('\0')
                    .to_string()
            })
        })
        .collect()
}

// Who a display is wherever it is plugged in is read where the shell can
// read it too (it needs the same ids for what it keeps per display).
pub use gevrek_system::display_ids::{edid_of, ids_by_makers_and_models};
use gevrek_system::display_ids::active_paths;

/// Give every display the id it has wherever it is plugged in
/// (`MonitorInfo::stable_id`). All displays of the desktop at once: twins
/// are told apart among all of them.
fn give_stable_ids(all: &mut [MonitorInfo]) {
    let places: Vec<(String, i32, i32, isize)> = all
        .iter()
        .map(|info| (info.device_path.clone(), info.x, info.y, info.handle.0))
        .collect();
    for (info, id) in all.iter_mut().zip(gevrek_system::display_ids::stable_ids(&places)) {
        info.stable_id = id;
    }
}

/// Windows' own name for a display (`\\.\DISPLAY1`).
fn system_name(handle: MonitorHandle) -> Option<String> {
    let mut asked = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the structure is ours and says how large it is.
    unsafe { GetMonitorInfoW(handle.to_hmonitor(), &mut asked.monitorInfo) }
        .as_bool()
        .then(|| {
            String::from_utf16_lossy(&asked.szDevice)
                .trim_end_matches('\0')
                .to_string()
        })
}

/// Whether a window is on a display gevrek leaves alone.
pub fn is_on_unmanaged_display(hwnd: HWND) -> bool {
    let handles = UNMANAGED_HANDLES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if handles.is_empty() {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    handles.contains(&(monitor.0 as isize))
}

/// Whether an entry of `desktop.outputs` means this display: by the name
/// `gevrek state` shows for it, or by its device path.
fn is_named(info: &MonitorInfo, name: &str) -> bool {
    info.is_called(name) || info.device_path.eq_ignore_ascii_case(name)
}

/// The displays gevrek uses and the ones it leaves alone: those the
/// configuration switches off (`names`), and those that are no screens
/// (`no_screens`) unless the configuration names them to be used
/// (`wanted`). Never all of them are left alone: a window manager
/// without a monitor manages nothing, so a configuration that switches
/// every display off is not followed, and a desktop of nothing but
/// displays that are no screens (a machine reached from afar) is used.
fn split_unmanaged(
    all: Vec<MonitorInfo>,
    names: &[String],
    wanted: &[String],
    no_screens: &[MonitorHandle],
) -> (Vec<MonitorInfo>, Vec<MonitorInfo>) {
    if names.is_empty() && no_screens.is_empty() {
        return (all, Vec::new());
    }
    let named = |info: &MonitorInfo, names: &[String]| names.iter().any(|name| is_named(info, name));
    let (unmanaged, managed): (Vec<_>, Vec<_>) = all.into_iter().partition(|info| {
        named(info, names) || (no_screens.contains(&info.handle) && !named(info, wanted))
    });
    if managed.is_empty() {
        log::warn!(
            "Every display is switched off in desktop.outputs or is no screen; all of them are used"
        );
        return (unmanaged, Vec::new());
    }
    (managed, unmanaged)
}

/// Every display of the desktop, those gevrek leaves alone among them,
/// each with Windows' own name for it (`\\.\DISPLAY1`): what the display
/// settings are asked and set by.
pub fn all_displays() -> Vec<(String, MonitorInfo)> {
    let mut monitors: HashMap<MonitorHandle, MonitorInfo> = HashMap::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    let mut all: Vec<MonitorInfo> = monitors.into_values().collect();
    gevrek_engine::outputs::number_same_names(&mut all);
    give_stable_ids(&mut all);
    give_taskbar_strips(&mut all);
    all.into_iter()
        .filter_map(|info| {
            let mut asked = MONITORINFOEXW {
                monitorInfo: MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: the structure is ours and says how large it is.
            unsafe { GetMonitorInfoW(info.handle.to_hmonitor(), &mut asked.monitorInfo) }
                .as_bool()
                .then(|| {
                    (
                        String::from_utf16_lossy(&asked.szDevice)
                            .trim_end_matches('\0')
                            .to_string(),
                        info,
                    )
                })
        })
        .collect()
}

/// Currently attached, active displays that gevrek uses.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors: HashMap<MonitorHandle, MonitorInfo> = HashMap::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    let names = UNMANAGED_NAMES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut all: Vec<MonitorInfo> = monitors.into_values().collect();
    // Two displays of one model are told apart by a number.
    gevrek_engine::outputs::number_same_names(&mut all);
    give_stable_ids(&mut all);
    give_taskbar_strips(&mut all);
    let wanted = WANTED_NAMES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let made_up = displays_that_are_no_screens();
    let no_screens: Vec<MonitorHandle> = if made_up.is_empty() {
        Vec::new()
    } else {
        all.iter()
            .filter(|info| {
                system_name(info.handle)
                    .is_some_and(|name| made_up.iter().any(|other| other.eq_ignore_ascii_case(&name)))
            })
            .map(|info| info.handle)
            .collect()
    };
    let (managed, unmanaged) = split_unmanaged(all, &names, &wanted, &no_screens);
    for info in &unmanaged {
        if no_screens.contains(&info.handle) {
            log::info!(
                "Display '{}' is no screen (a program makes it up): left alone unless \
                 desktop.outputs names it",
                info.name
            );
        } else {
            log::info!(
                "Display '{}' is switched off in desktop.outputs: left alone",
                info.name
            );
        }
    }
    *UNMANAGED_HANDLES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        unmanaged.iter().map(|info| info.handle.0).collect();
    managed
}

/// The monitor model operations that need Windows. Same names as before the
/// engine split, so call sites only import this trait.
pub trait MonitorManagerWin32 {
    /// Refresh monitor information only (without creating workspaces).
    fn refresh_monitors_only(&mut self) -> Result<()>;
    /// Refresh monitors (without auto-creating workspaces).
    fn refresh(&mut self) -> Result<()>;
    /// Re-enumerate displays and apply the differences.
    fn reconcile_monitors(&mut self) -> Result<MonitorChanges>;
    fn get_monitor_at_point(&self, x: i32, y: i32) -> Option<MonitorInfo>;
    fn get_monitor_for_window(&self, hwnd: HWND) -> Option<MonitorInfo>;
    fn get_monitor_at_cursor(&self) -> Option<MonitorInfo>;
}

impl MonitorManagerWin32 for MonitorManager {
    fn refresh_monitors_only(&mut self) -> Result<()> {
        self.set_monitors_only(enumerate_monitors());
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        self.replace_monitors(enumerate_monitors());
        Ok(())
    }

    fn reconcile_monitors(&mut self) -> Result<MonitorChanges> {
        Ok(self.reconcile_with(enumerate_monitors()))
    }

    fn get_monitor_at_point(&self, x: i32, y: i32) -> Option<MonitorInfo> {
        let hmonitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
        (!hmonitor.is_invalid())
            .then(|| {
                self.get_monitor(MonitorHandle::from_hmonitor(hmonitor))
                    .cloned()
            })
            .flatten()
    }

    fn get_monitor_for_window(&self, hwnd: HWND) -> Option<MonitorInfo> {
        let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        (!hmonitor.is_invalid())
            .then(|| {
                self.get_monitor(MonitorHandle::from_hmonitor(hmonitor))
                    .cloned()
            })
            .flatten()
    }

    fn get_monitor_at_cursor(&self) -> Option<MonitorInfo> {
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut point) }.ok()?;
        self.get_monitor_at_point(point.x, point.y)
    }
}

unsafe extern "system" fn enum_monitors_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};

    let monitors = &mut *(lparam.0 as *mut HashMap<MonitorHandle, MonitorInfo>);

    let mut monitor_info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    if GetMonitorInfoW(hmonitor, &mut monitor_info.monitorInfo).as_bool() {
        let handle = MonitorHandle::from_hmonitor(hmonitor);

        // Get device name (e.g., "\\.\DISPLAY1")
        let device_name = String::from_utf16_lossy(&monitor_info.szDevice)
            .trim_end_matches('\0')
            .to_string();

        // Get detailed monitor information using EnumDisplayDevices
        let mut display_device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };

        // Get adapter information
        let device_name_wide: Vec<u16> = device_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut friendly_name = device_name.clone();

        // The device interface path identifies the physical display; the GDI
        // device name can move to another display after a topology change.
        let device_path = gevrek_system::display_ids::device_path_of(&device_name);

        if EnumDisplayDevicesW(
            windows::core::PCWSTR(device_name_wide.as_ptr()),
            0,
            &mut display_device,
            0,
        )
        .as_bool()
        {
            // Check if this is an active, attached monitor (not a pseudo device)
            const DISPLAY_DEVICE_ACTIVE: u32 = 0x00000001;
            const DISPLAY_DEVICE_ATTACHED: u32 = 0x00000002;
            const DISPLAY_DEVICE_MIRRORING_DRIVER: u32 = 0x00000008;

            let state_flags = display_device.StateFlags;

            // Skip inactive, detached, or mirroring pseudo monitors
            if (state_flags & DISPLAY_DEVICE_ACTIVE) == 0 {
                log::debug!("Skipping inactive monitor: {}", device_name);
                return true.into();
            }

            if (state_flags & DISPLAY_DEVICE_ATTACHED) == 0 {
                log::debug!("Skipping detached monitor: {}", device_name);
                return true.into();
            }

            if (state_flags & DISPLAY_DEVICE_MIRRORING_DRIVER) != 0 {
                log::debug!("Skipping mirroring driver: {}", device_name);
                return true.into();
            }

            // Get the friendly monitor name (DeviceString contains monitor description)
            let monitor_desc = String::from_utf16_lossy(&display_device.DeviceString)
                .trim_end_matches('\0')
                .to_string();

            if !monitor_desc.is_empty() && monitor_desc != "Generic PnP Monitor" {
                friendly_name = monitor_desc;
            } else {
                // Try to get more specific monitor info
                let device_id = String::from_utf16_lossy(&display_device.DeviceID)
                    .trim_end_matches('\0')
                    .to_string();

                // Parse monitor vendor/model from device ID if available
                // Format: MONITOR\XXX1234\{...} where XXX is vendor, 1234 is model
                if device_id.contains("MONITOR\\") {
                    if let Some(monitor_part) = device_id.split("MONITOR\\").nth(1) {
                        if let Some(model_code) = monitor_part.split('\\').next() {
                            if model_code.len() >= 3 {
                                friendly_name = format!("Monitor {}", model_code);
                            }
                        }
                    }
                }
            }
        }

        // MONITORINFOF_PRIMARY = 0x00000001
        let is_primary = (monitor_info.monitorInfo.dwFlags & 0x00000001) != 0;

        let full_rect = monitor_info.monitorInfo.rcMonitor;
        let work_rect = monitor_info.monitorInfo.rcWork;

        let info = MonitorInfo {
            handle,
            name: friendly_name,
            device_path,
            // (Given once all displays are known: `give_stable_ids`.)
            stable_id: String::new(),
            is_primary,
            x: full_rect.left,
            y: full_rect.top,
            width: full_rect.right - full_rect.left,
            height: full_rect.bottom - full_rect.top,
            work_x: work_rect.left,
            work_y: work_rect.top,
            work_width: work_rect.right - work_rect.left,
            work_height: work_rect.bottom - work_rect.top,
            dpi_scale: {
                use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
                let mut dpi_x: u32 = 96;
                let mut dpi_y: u32 = 96;
                let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                dpi_x as f32 / 96.0
            },
        };

        //log::debug!("Detected active monitor: {} ({}x{})", info.name, info.width, info.height);
        monitors.insert(handle, info);
    }

    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(handle: isize, name: &str, path: &str) -> MonitorInfo {
        MonitorInfo {
            handle: MonitorHandle(handle),
            name: name.to_string(),
            device_path: path.to_string(),
            stable_id: String::new(),
            is_primary: handle == 1,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: 0,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            dpi_scale: 1.0,
        }
    }

    fn split_unmanaged(all: Vec<MonitorInfo>, names: &[String]) -> (Vec<MonitorInfo>, Vec<MonitorInfo>) {
        super::split_unmanaged(all, names, &[], &[])
    }

    #[test]
    fn a_display_switched_off_is_left_alone_but_never_all_of_them() {
        let all = || {
            vec![
                display(1, "Monitor ACM2401", r"\\?\DISPLAY#ACM2401#one"),
                display(2, "Monitor ACM2702", r"\\?\DISPLAY#ACM2702#two"),
            ]
        };
        let handles =
            |list: &[MonitorInfo]| list.iter().map(|info| info.handle.0).collect::<Vec<_>>();

        let (managed, unmanaged) = split_unmanaged(all(), &[]);
        assert_eq!((handles(&managed).len(), unmanaged.len()), (2, 0));

        // By the name `gevrek state` shows, whatever its case.
        let (managed, unmanaged) = split_unmanaged(all(), &["monitor acm2702".to_string()]);
        assert_eq!((handles(&managed), handles(&unmanaged)), (vec![1], vec![2]));

        // By the device path.
        let (managed, unmanaged) =
            split_unmanaged(all(), &[r"\\?\display#acm2401#one".to_string()]);
        assert_eq!((handles(&managed), handles(&unmanaged)), (vec![2], vec![1]));

        // A name no display has changes nothing.
        let (managed, unmanaged) = split_unmanaged(all(), &["hdmi-a-1".to_string()]);
        assert_eq!((managed.len(), unmanaged.len()), (2, 0));

        // Two of one model: each has its number, and the name without a
        // number still means both.
        let twins = || {
            let mut list = vec![
                display(1, "Monitor ACM2401", r"\\?\DISPLAY#ACM2401#one"),
                display(2, "Monitor ACM2702", r"\\?\DISPLAY#ACM2702#b"),
                display(3, "Monitor ACM2702", r"\\?\DISPLAY#ACM2702#a"),
            ];
            gevrek_engine::outputs::number_same_names(&mut list);
            list
        };
        let names: Vec<String> = twins().into_iter().map(|info| info.name).collect();
        assert_eq!(
            names,
            [
                "Monitor ACM2401",
                "Monitor ACM2702 (2)",
                "Monitor ACM2702 (1)"
            ]
        );
        let (managed, unmanaged) = split_unmanaged(twins(), &["monitor acm2702 (1)".to_string()]);
        assert_eq!(
            (handles(&managed), handles(&unmanaged)),
            (vec![1, 2], vec![3])
        );
        let (managed, unmanaged) = split_unmanaged(twins(), &["monitor acm2702".to_string()]);
        assert_eq!(
            (handles(&managed), handles(&unmanaged)),
            (vec![1], vec![2, 3])
        );

        // Every display switched off: not followed.
        let names = ["monitor acm2401".to_string(), "monitor acm2702".to_string()];
        let (managed, unmanaged) = split_unmanaged(all(), &names);
        assert_eq!((managed.len(), unmanaged.len()), (2, 0));
    }

    #[test]
    fn a_display_that_is_no_screen_is_left_alone_unless_it_is_named() {
        let all = || {
            vec![
                display(1, "Monitor ACM2401", r"\\?\DISPLAY#ACM2401#one"),
                display(2, "Headset", r"\\?\DISPLAY#MADEUP#two"),
            ]
        };
        let handles =
            |list: &[MonitorInfo]| list.iter().map(|info| info.handle.0).collect::<Vec<_>>();
        let headset = [MonitorHandle(2)];

        // Left alone, with nothing said of it.
        let (managed, unmanaged) = super::split_unmanaged(all(), &[], &[], &headset);
        assert_eq!((handles(&managed), handles(&unmanaged)), (vec![1], vec![2]));

        // Used when the configuration names it, by either of its names.
        for name in ["headset", r"\\?\display#madeup#two"] {
            let (managed, unmanaged) =
                super::split_unmanaged(all(), &[], &[name.to_string()], &headset);
            assert_eq!((managed.len(), unmanaged.len()), (2, 0), "{name}");
        }

        // Naming another display does not bring it in.
        let (managed, _) =
            super::split_unmanaged(all(), &[], &["monitor acm2401".to_string()], &headset);
        assert_eq!(handles(&managed), vec![1]);

        // A desktop of nothing else (a machine reached from afar): used.
        let both = [MonitorHandle(1), MonitorHandle(2)];
        let (managed, unmanaged) = super::split_unmanaged(all(), &[], &[], &both);
        assert_eq!((managed.len(), unmanaged.len()), (2, 0));

        // The screen switched off and the other no screen: both are used
        // (never none).
        let (managed, unmanaged) =
            super::split_unmanaged(all(), &["monitor acm2401".to_string()], &[], &headset);
        assert_eq!((managed.len(), unmanaged.len()), (2, 0));
    }
}
