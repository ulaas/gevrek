//! Bluetooth: whether the machine's radio is on, the devices it knows,
//! and connecting one. Asked of BlueZ through its own tool,
//! `bluetoothctl` (`GEVREK_BLUETOOTHCTL` names another program in its
//! place: the checks have no radio). A machine without the tool, or
//! without a radio, has no bluetooth to show.
//!
//! On Windows the radio and the devices the machine is paired with are
//! asked of the system's own interfaces. Connecting a device, or letting
//! it go, is not something Windows lets a program ask for: there
//! `connect` does nothing and says so (`can_connect`), and the one who
//! shows the devices sends the person to Windows' own page.

/// A device the machine knows (it was paired with it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Its address (`34:E3:FB:60:89:38`).
    pub address: String,
    pub name: String,
    pub connected: bool,
}

/// How bluetooth stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bluetooth {
    /// The radio is on.
    pub powered: bool,
    /// The devices that are known, the connected ones first.
    pub devices: Vec<Device>,
}

/// Whether the tool's answer to `show` names a radio at all.
pub fn has_radio(show: &str) -> bool {
    show.lines()
        .any(|line| line.trim_start().starts_with("Controller "))
}

/// Whether the radio is on, out of the tool's answer to `show`.
pub fn powered_in(show: &str) -> bool {
    show.lines()
        .filter_map(|line| line.trim().strip_prefix("Powered:"))
        .any(|said| said.trim() == "yes")
}

/// The devices of a list of the tool's (`Device <address> <name>`).
pub fn devices_in(list: &str) -> Vec<(String, String)> {
    list.lines()
        .filter_map(|line| line.trim().strip_prefix("Device "))
        .filter_map(|rest| {
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            is_address(address).then(|| {
                let name = name.trim();
                (
                    address.to_string(),
                    if name.is_empty() { address } else { name }.to_string(),
                )
            })
        })
        .collect()
}

/// An address as a device has it: six pairs of hexadecimal digits.
pub fn is_address(text: &str) -> bool {
    let parts: Vec<&str> = text.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|digit| digit.is_ascii_hexdigit()))
}

/// The known devices with which of them are connected, those first.
pub fn together(known: Vec<(String, String)>, connected: &[(String, String)]) -> Vec<Device> {
    let mut devices: Vec<Device> = known
        .into_iter()
        .map(|(address, name)| Device {
            connected: connected.iter().any(|(other, _)| *other == address),
            address,
            name,
        })
        .collect();
    devices.sort_by_key(|device| !device.connected);
    devices
}

#[cfg(target_os = "linux")]
fn tool(arguments: &[&str], seconds: u32) -> Option<String> {
    let program =
        std::env::var("GEVREK_BLUETOOTHCTL").unwrap_or_else(|_| "bluetoothctl".to_string());
    let output = std::process::Command::new(program)
        .arg("--timeout")
        .arg(seconds.to_string())
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// An address as it is written, from the number Windows has for it.
pub fn address_of(number: u64) -> String {
    let bytes = number.to_be_bytes();
    bytes[2..]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Whether a device can be connected and let go from here (`connect`).
pub fn can_connect() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(windows)]
mod system {
    use super::{Bluetooth, Device};
    use windows::core::HSTRING;
    use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice};
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};

    fn radio() -> Option<Radio> {
        let radios = Radio::GetRadiosAsync().ok()?.get().ok()?;
        radios
            .into_iter()
            .find(|radio| radio.Kind().ok() == Some(RadioKind::Bluetooth))
    }

    /// The devices a selector of the system's finds, by their names in
    /// the system's list.
    fn found(selector: windows::core::Result<HSTRING>) -> Vec<(HSTRING, String)> {
        let Some(list) = selector
            .ok()
            .and_then(|selector| DeviceInformation::FindAllAsyncAqsFilter(&selector).ok())
            .and_then(|asked| asked.get().ok())
        else {
            return Vec::new();
        };
        list.into_iter()
            .filter_map(|device| {
                Some((
                    device.Id().ok()?,
                    device.Name().map(|name| name.to_string()).unwrap_or_default(),
                ))
            })
            .collect()
    }

    pub fn read() -> Option<Bluetooth> {
        let radio = radio()?;
        let powered = radio.State().ok() == Some(RadioState::On);
        let mut devices: Vec<Device> = Vec::new();
        let mut add = |number: u64, name: String, connected: bool| {
            let address = super::address_of(number);
            match devices
                .iter_mut()
                .find(|known| known.address == address || (!name.trim().is_empty() && known.name == name))
            {
                // (A device of both kinds is one device, also where it
                // has an address for each: it is known by its name.)
                Some(known) => known.connected |= connected,
                None => devices.push(Device {
                    name: if name.trim().is_empty() {
                        address.clone()
                    } else {
                        name
                    },
                    address,
                    connected,
                }),
            }
        };
        for (id, name) in found(BluetoothDevice::GetDeviceSelectorFromPairingState(true)) {
            let Some(device) = BluetoothDevice::FromIdAsync(&id)
                .ok()
                .and_then(|asked| asked.get().ok())
            else {
                continue;
            };
            let Ok(number) = device.BluetoothAddress() else {
                continue;
            };
            let connected =
                device.ConnectionStatus().ok() == Some(BluetoothConnectionStatus::Connected);
            add(number, name, powered && connected);
        }
        for (id, name) in found(BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)) {
            let Some(device) = BluetoothLEDevice::FromIdAsync(&id)
                .ok()
                .and_then(|asked| asked.get().ok())
            else {
                continue;
            };
            let Ok(number) = device.BluetoothAddress() else {
                continue;
            };
            let connected =
                device.ConnectionStatus().ok() == Some(BluetoothConnectionStatus::Connected);
            add(number, name, powered && connected);
        }
        devices.sort_by_key(|device| !device.connected);
        Some(Bluetooth { powered, devices })
    }

    pub fn set_power(on: bool) {
        let Some(radio) = radio() else {
            return;
        };
        let state = if on { RadioState::On } else { RadioState::Off };
        match radio.SetStateAsync(state).and_then(|asked| asked.get()) {
            Ok(answer) => log::debug!("bluetooth's radio asked to be {state:?}: {answer:?}"),
            Err(error) => log::warn!("bluetooth's radio was not switched: {error}"),
        }
    }
}

/// How bluetooth stands now; `None` on a machine without it.
pub fn read() -> Option<Bluetooth> {
    #[cfg(windows)]
    {
        return system::read();
    }
    #[allow(unreachable_code)]
    read_elsewhere()
}

fn read_elsewhere() -> Option<Bluetooth> {
    #[cfg(target_os = "linux")]
    {
        let show = tool(&["show"], 3)?;
        if !has_radio(&show) {
            return None;
        }
        let known = devices_in(&tool(&["devices", "Paired"], 3).unwrap_or_default());
        let connected = devices_in(&tool(&["devices", "Connected"], 3).unwrap_or_default());
        Some(Bluetooth {
            powered: powered_in(&show),
            devices: together(known, &connected),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Switch the radio on or off.
pub fn set_power(on: bool) {
    #[cfg(target_os = "linux")]
    {
        let _ = tool(&["power", if on { "on" } else { "off" }], 5);
    }
    #[cfg(windows)]
    {
        system::set_power(on);
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = on;
    }
}

/// Connect a known device, or let it go. (Connecting takes its seconds:
/// call it beside what draws.)
pub fn connect(address: &str, on: bool) {
    if !is_address(address) {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = tool(&[if on { "connect" } else { "disconnect" }, address], 12);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOW: &str = "Controller 44:38:E8:D9:63:44 (public)\n\tManufacturer: 0x0002 (2)\n\tName: box\n\tPowered: yes\n\tPowerState: on\n\tDiscoverable: no\n";

    #[test]
    fn the_radio_and_whether_it_is_on_are_read_from_the_tool() {
        assert!(has_radio(SHOW));
        assert!(powered_in(SHOW));
        assert!(!powered_in(&SHOW.replace("Powered: yes", "Powered: no")));
        assert!(!has_radio("No default controller available\n"));
        assert!(!has_radio(""));
    }

    #[test]
    fn the_devices_are_listed_with_the_connected_ones_first() {
        let known = devices_in(
            "Device 34:E3:FB:60:89:38 Galaxy Buds3 Pro (8938)\nDevice AA:BB:CC:DD:EE:FF Keyboard K380\nDevice 11:22:33:44:55:66\nsomething else\nDevice not-an-address Thing\n",
        );
        assert_eq!(known.len(), 3);
        assert_eq!(
            known[0],
            (
                "34:E3:FB:60:89:38".to_string(),
                "Galaxy Buds3 Pro (8938)".to_string()
            )
        );
        // (One without a name is known by its address.)
        assert_eq!(known[2].1, "11:22:33:44:55:66");
        let connected = devices_in("Device AA:BB:CC:DD:EE:FF Keyboard K380\n");
        let devices = together(known, &connected);
        assert_eq!(devices[0].name, "Keyboard K380");
        assert!(devices[0].connected && !devices[1].connected && !devices[2].connected);
    }

    #[test]
    fn an_address_is_written_from_its_number() {
        assert_eq!(address_of(0x34E3_FB60_8938), "34:E3:FB:60:89:38");
        assert_eq!(address_of(0x0000_0000_0001), "00:00:00:00:00:01");
        assert!(is_address(&address_of(0xAABB_CCDD_EEFF)));
    }

    /// What this machine's bluetooth says of itself, printed to be looked
    /// at (only read):
    ///
    ///     cargo test -p gevrek-system bluetooth_is_read -- --ignored --nocapture
    #[test]
    #[ignore = "reads this machine's bluetooth and prints it"]
    fn bluetooth_is_read_to_be_looked_at() {
        match read() {
            None => println!("no bluetooth radio, or nothing answers"),
            Some(bluetooth) => {
                println!("the radio is on: {}", bluetooth.powered);
                for device in &bluetooth.devices {
                    println!(
                        "  {}  {}{}",
                        device.address,
                        if device.connected { "connected  " } else { "" },
                        device.name
                    );
                }
            }
        }
    }

    /// The radio switched to the other state and back. **It really
    /// switches this machine's bluetooth radio**, for some seconds:
    ///
    ///     cargo test -p gevrek-system bluetooths_radio_is_switched -- --ignored --nocapture
    #[test]
    #[ignore = "switches this machine's bluetooth radio, and back"]
    fn bluetooths_radio_is_switched_and_switched_back() {
        let Some(before) = read() else {
            println!("no bluetooth radio, or nothing answers");
            return;
        };
        let wait_for = |on: bool| {
            (0..20).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                read().is_some_and(|radio| radio.powered == on)
            })
        };
        println!("the radio was on: {}, devices: {}", before.powered, before.devices.len());
        set_power(!before.powered);
        println!("the other way within ten seconds: {}", wait_for(!before.powered));
        set_power(before.powered);
        println!("as it was within ten seconds: {}", wait_for(before.powered));
    }

    #[test]
    fn only_an_address_is_handed_to_the_tool() {
        assert!(is_address("34:E3:FB:60:89:38"));
        for no in [
            "",
            "34:E3:FB:60:89",
            "34:E3:FB:60:89:3G",
            "--help",
            "34-E3-FB-60-89-38",
            "34:E3:FB:60:89:38; reboot",
        ] {
            assert!(!is_address(no), "{no}");
        }
    }
}
