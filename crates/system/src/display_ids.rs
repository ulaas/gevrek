//! Who a display is wherever it is plugged in, as Windows can be asked.
//!
//! The id itself is made in the model (`gevrek_model::edid`), the same
//! on every system. This is the Windows half of it: reading what
//! the display said of itself out of the system. It lives here and not in
//! the window manager's backend because two programs need the same ids:
//! the window manager, for its outputs, and the shell, for what the zones
//! keep per display. A zone must come back to "the left monitor" whichever
//! of the two found the display first.

use windows::Win32::Foundation::ERROR_SUCCESS;

/// The device path of a display by Windows' own name for it
/// (`\\.\DISPLAY1`): `\\?\DISPLAY#GSM5B7F#...`, which names the display
/// itself, on the port it is plugged into. Windows' own name can pass to
/// another display when the displays change. Where the system gives no
/// path, the name is all there is.
pub fn device_path_of(device_name: &str) -> String {
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};
    const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x00000001;
    let wide: Vec<u16> = device_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut interface = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    // SAFETY: the name ends with a zero; the structure is ours and says
    // how large it is.
    let found = unsafe {
        EnumDisplayDevicesW(
            windows::core::PCWSTR(wide.as_ptr()),
            0,
            &mut interface,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    }
    .as_bool();
    let path = if found {
        String::from_utf16_lossy(&interface.DeviceID)
            .trim_end_matches('\0')
            .to_string()
    } else {
        String::new()
    };
    if path.is_empty() {
        device_name.to_string()
    } else {
        path
    }
}

/// The ways from a picture to a display that are in use now, as Windows'
/// display configuration has them. None when it could not be asked.
pub fn active_paths() -> Vec<windows::Win32::Devices::Display::DISPLAYCONFIG_PATH_INFO> {
    use windows::Win32::Devices::Display::{
        GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, QDC_ONLY_ACTIVE_PATHS,
    };
    let (mut paths_there, mut modes_there) = (0u32, 0u32);
    // SAFETY: the two counts are ours.
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_there, &mut modes_there)
    } != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_there as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_there as usize];
    // SAFETY: the lists are as long as the counts say; the system says
    // how many of each it filled.
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
        // (The displays changed between the two questions: asked again
        // with the next look at the displays.)
        return Vec::new();
    }
    paths.truncate(paths_there as usize);
    paths
}

/// The EDID of a display as Windows keeps it, by the display's device path
/// (`\\?\DISPLAY#GSM5B7F#...`): the value `EDID` of the device's own
/// registry key (`...\Enum\DISPLAY\<model>\<instance>\Device Parameters`),
/// where the monitor's driver puts what the display sent. The key is
/// reached the way the system gives for it (the device of the interface,
/// that device's key), not by putting the registry's path together. `None`
/// for a display that has none there (one a program makes up, a path that
/// is no device's).
pub fn edid_of(device_path: &str) -> Option<Vec<u8>> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_Interface_PropertyW, CM_Locate_DevNodeW, CM_Open_DevNode_Key,
        RegDisposition_OpenExisting, CM_LOCATE_DEVNODE_NORMAL, CM_REGISTRY_HARDWARE, CR_SUCCESS,
    };
    use windows::Win32::Devices::Properties::{
        DEVPKEY_Device_InstanceId, DEVPROPTYPE, DEVPROP_TYPE_STRING,
    };
    use windows::Win32::System::Registry::{RegCloseKey, RegQueryValueExW, HKEY, KEY_READ};

    let interface: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // The device the interface belongs to (`DISPLAY\GSM5B7F\5&...`).
    let mut instance = [0u16; 512];
    let mut kind = DEVPROPTYPE::default();
    let mut size = std::mem::size_of_val(&instance) as u32;
    // SAFETY: the path ends with a zero; the buffer is as large as `size`
    // says, in bytes.
    let found = unsafe {
        CM_Get_Device_Interface_PropertyW(
            PCWSTR(interface.as_ptr()),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            Some(instance.as_mut_ptr().cast()),
            &mut size,
            0,
        )
    };
    if found != CR_SUCCESS || kind != DEVPROP_TYPE_STRING {
        return None;
    }
    // (It ends with a zero as the system wrote it; made sure of.)
    *instance.last_mut()? = 0;
    let mut device = 0u32;
    // SAFETY: the instance id ends with a zero; the number is ours.
    let located = unsafe {
        CM_Locate_DevNodeW(
            &mut device,
            PCWSTR(instance.as_ptr()),
            CM_LOCATE_DEVNODE_NORMAL,
        )
    };
    if located != CR_SUCCESS {
        return None;
    }
    let mut key = HKEY::default();
    // SAFETY: the key is ours to receive; it is closed below.
    let opened = unsafe {
        CM_Open_DevNode_Key(
            device,
            KEY_READ.0,
            0,
            RegDisposition_OpenExisting,
            &mut key,
            CM_REGISTRY_HARDWARE,
        )
    };
    if opened != CR_SUCCESS {
        return None;
    }
    // A base block and a few extension blocks is what a display sends; the
    // base block, at the start, is all that says who the display is.
    let mut bytes = vec![0u8; 1024];
    let mut length = bytes.len() as u32;
    // SAFETY: the buffer is as long as `length` says.
    let read = unsafe {
        RegQueryValueExW(
            key,
            w!("EDID"),
            None,
            None,
            Some(bytes.as_mut_ptr()),
            Some(&mut length),
        )
    };
    // SAFETY: the key was opened above.
    let _ = unsafe { RegCloseKey(key) };
    if read != ERROR_SUCCESS {
        return None;
    }
    bytes.truncate((length as usize).min(bytes.len()));
    Some(bytes)
}

/// What Windows' display configuration tells of each display in use that
/// it knows the maker and model of, by the display's device path: the id of
/// a display of that maker and model without a serial number
/// (`GSM-5B7F-noserial`). It is what is left to go by when the EDID itself
/// cannot be read.
pub fn ids_by_makers_and_models() -> Vec<(String, String)> {
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_TARGET_DEVICE_NAME,
    };
    active_paths()
        .iter()
        .filter_map(|path| {
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            // SAFETY: the structure is ours and says what it is and how
            // large.
            if unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } != 0 {
                return None;
            }
            // The third of the flags says that the two ids are the EDID's.
            const EDID_IDS_VALID: u32 = 0x4;
            // SAFETY: the flags are a number, whichever way they are read.
            if unsafe { target.flags.Anonymous.value } & EDID_IDS_VALID == 0 {
                return None;
            }
            // (The maker's two bytes are in the number in the order they
            // have in the EDID: the first of them is the low one.)
            let id = gevrek_model::edid::stable_id_without_serial(
                target.edidManufactureId.to_le_bytes(),
                target.edidProductCodeId,
            )?;
            Some((
                String::from_utf16_lossy(&target.monitorDevicePath)
                    .trim_end_matches('\0')
                    .to_string(),
                id,
            ))
        })
        .collect()
}

/// The id each display has wherever it is plugged in
/// (`MonitorInfo::stable_id`): of its EDID as Windows keeps it; else of the
/// maker and model the display configuration tells; else its device path.
/// Twins that come out with the same id are told apart by where they are.
///
/// `displays` says of each one its device path, x, y and handle; all
/// displays of the desktop must be given at once, for whether a display
/// has a twin is a matter of all of them. The window manager and the
/// shell both ask here, and so have the same id for the same display.
pub fn stable_ids(displays: &[(String, i32, i32, isize)]) -> Vec<String> {
    use gevrek_model::edid::{number_same_ids, stable_id_of_place, Edid};
    // (Asked only when a display's EDID could not be read.)
    let mut told: Option<Vec<(String, String)>> = None;
    let mut ids: Vec<String> = displays
        .iter()
        .map(
            |(path, ..)| match edid_of(path).and_then(|bytes| Edid::parse(&bytes)) {
                Some(edid) => edid.stable_id(),
                None => told
                    .get_or_insert_with(ids_by_makers_and_models)
                    .iter()
                    .find(|(known, _)| known.eq_ignore_ascii_case(path))
                    .map(|(_, id)| id.clone())
                    .unwrap_or_else(|| stable_id_of_place(path)),
            },
        )
        .collect();
    number_same_ids(&mut ids, displays);
    ids
}
