//! What a panel shows of the machine: the battery and the network.
//!
//! On Linux the battery is read from the kernel's files and the network
//! asked of NetworkManager (`nmcli`), with the kernel's own word on the
//! interfaces where that is not there. On Windows the battery is the
//! system's power status, and the network is read from the machine's
//! adapters (a cable or a radio that is up and has a gateway), named as
//! the system names the network it is on. On other systems there is
//! nothing here yet, and a panel shows nothing.

/// The battery of a machine that has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// How full, 0 to 100.
    pub percent: u8,
    /// On the mains (charging, or full and plugged in).
    pub charging: bool,
}

/// The network the machine is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub connected: bool,
    /// By radio (else by cable).
    pub wireless: bool,
    /// The connection's name (the wireless network's, or the cable's
    /// profile); empty when there is none.
    pub name: String,
}

/// A battery from what the kernel says of one power supply: its type, how
/// full and its status. `None` for what is not a battery of the machine
/// (the mains, a mouse's battery has `scope` Device and is left out by
/// the caller).
pub fn parse_battery(kind: &str, capacity: &str, status: &str) -> Option<Battery> {
    if kind.trim() != "Battery" {
        return None;
    }
    let percent = capacity.trim().parse::<u32>().ok()?.min(100) as u8;
    let status = status.trim();
    Some(Battery {
        percent,
        charging: status == "Charging" || status == "Full" || status == "Not charging",
    })
}

/// The network from NetworkManager's list of devices, one a line as
/// `TYPE:STATE:CONNECTION` (`nmcli -t -f TYPE,STATE,CONNECTION device`):
/// the first connected cable, else the first connected wireless, else not
/// connected. `None` when no line names a cable or a wireless device.
pub fn parse_network(devices: &str) -> Option<Network> {
    let mut any = false;
    let mut found: Option<Network> = None;
    for line in devices.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(kind), Some(state)) = (parts.next(), parts.next()) else {
            continue;
        };
        let wireless = match kind {
            "ethernet" => false,
            "wifi" => true,
            _ => continue,
        };
        any = true;
        // ("connected", not "connected (externally)" nor "disconnected".)
        if state != "connected" {
            continue;
        }
        let name = parts.next().unwrap_or_default().replace("\\:", ":");
        if found
            .as_ref()
            .is_none_or(|known| known.wireless && !wireless)
        {
            found = Some(Network {
                connected: true,
                wireless,
                name,
            });
        }
    }
    found.or(any.then(|| Network {
        connected: false,
        wireless: false,
        name: String::new(),
    }))
}

/// The machine's battery; `None` when it has none (or on a system this
/// does not read yet).
#[cfg(target_os = "linux")]
pub fn battery() -> Option<Battery> {
    let read = |path: std::path::PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let mut supplies: Vec<std::path::PathBuf> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    supplies.sort();
    supplies.into_iter().find_map(|supply| {
        // A mouse or a headset has a battery too: not the machine's.
        if read(supply.join("scope")).trim() == "Device" {
            return None;
        }
        parse_battery(
            &read(supply.join("type")),
            &read(supply.join("capacity")),
            &read(supply.join("status")),
        )
    })
}

/// The machine's battery, as Windows tells it.
#[cfg(windows)]
pub fn battery() -> Option<Battery> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: the call fills in the structure it is given.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    battery_of(
        status.BatteryFlag,
        status.BatteryLifePercent,
        status.ACLineStatus,
    )
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn battery() -> Option<Battery> {
    None
}

/// A battery from Windows' power status: the battery's flags (128: the
/// machine has none; 255: not known), how full in percent (255: not
/// known) and whether the machine is on the mains (1: yes).
pub fn battery_of(flags: u8, percent: u8, mains: u8) -> Option<Battery> {
    if flags == 255 || flags & 128 != 0 || percent > 100 {
        return None;
    }
    Some(Battery {
        percent,
        charging: mains == 1,
    })
}

/// One of the machine's network adapters, as far as a panel cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// By radio (else by cable).
    pub wireless: bool,
    /// It is up and has somewhere to send to (a gateway): an adapter that
    /// is only up (a virtual machine's switch) leads nowhere.
    pub connected: bool,
    /// What the connection is called: the network's name where the system
    /// knows one, else the adapter's own.
    pub name: String,
}

/// The network from the machine's adapters: the first connected cable,
/// else the first connected radio, else not connected. `None` when there
/// is neither a cable nor a radio to connect with.
pub fn network_of(adapters: &[Adapter]) -> Option<Network> {
    let connected = |wireless: bool| {
        adapters
            .iter()
            .find(|adapter| adapter.connected && adapter.wireless == wireless)
    };
    match connected(false).or(connected(true)) {
        Some(adapter) => Some(Network {
            connected: true,
            wireless: adapter.wireless,
            name: adapter.name.clone(),
        }),
        None => (!adapters.is_empty()).then(|| Network {
            connected: false,
            wireless: false,
            name: String::new(),
        }),
    }
}

/// The network the machine is on; `None` when nothing can say.
#[cfg(target_os = "linux")]
pub fn network() -> Option<Network> {
    let asked = std::process::Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    if let Some(network) = asked.as_deref().and_then(parse_network) {
        return Some(network);
    }
    // Without NetworkManager: whether an interface that is not the
    // machine's own loop is up.
    let mut any = false;
    let mut up: Option<bool> = None;
    for entry in std::fs::read_dir("/sys/class/net").ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        any = true;
        let state = std::fs::read_to_string(entry.path().join("operstate")).unwrap_or_default();
        if state.trim() == "up" {
            let wireless = entry.path().join("wireless").exists();
            if up.is_none_or(|known| known && !wireless) {
                up = Some(wireless);
            }
        }
    }
    any.then(|| Network {
        connected: up.is_some(),
        wireless: up.unwrap_or(false),
        name: String::new(),
    })
}

/// The network the machine is on, from its adapters as Windows lists them.
#[cfg(windows)]
pub fn network() -> Option<Network> {
    network_of(&windows_net::adapters())
}

/// Wait until something about the machine's network addresses changes (a
/// cable, a radio network joined or left). `false` when the system cannot
/// be waited on: whoever asks in a loop then has to wait by the clock.
#[cfg(windows)]
pub fn wait_for_network_change() -> bool {
    use windows::Win32::NetworkManagement::IpHelper::NotifyAddrChange;
    // SAFETY: with neither a handle nor an overlapped structure the call
    // simply returns when something changed.
    unsafe { NotifyAddrChange(std::ptr::null_mut(), std::ptr::null()) == 0 }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn network() -> Option<Network> {
    None
}

#[cfg(windows)]
mod windows_net {
    use super::Adapter;
    use windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };

    /// The kinds of adapter a panel speaks of (the system's numbers).
    const ETHERNET: u32 = 6;
    const WIRELESS: u32 = 71;
    /// `IfOperStatusUp`.
    const UP: i32 = 1;

    /// The machine's cable and radio adapters.
    pub fn adapters() -> Vec<Adapter> {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS
            | GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER;
        // (In whole words, so that the structures in it lie as they must.)
        let mut size: u32 = 16 * 1024;
        let mut buffer: Vec<u64> = Vec::new();
        for _ in 0..4 {
            buffer = vec![0u64; (size as usize).div_ceil(8)];
            // SAFETY: the buffer is as large as `size` says; the system
            // writes the list into it or says how large it has to be.
            let said = unsafe {
                GetAdaptersAddresses(
                    0, // every family of addresses
                    flags,
                    None,
                    Some(buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>()),
                    &mut size,
                )
            };
            if said == 0 {
                break;
            }
            if said != ERROR_BUFFER_OVERFLOW.0 {
                return Vec::new();
            }
        }
        let names = network_names();
        let mut found = Vec::new();
        let mut next = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: the list is the system's, in the buffer that is kept
        // until the end of this function; it ends with a null pointer.
        unsafe {
            while !next.is_null() {
                let adapter = &*next;
                next = adapter.Next;
                let wireless = match adapter.IfType {
                    ETHERNET => false,
                    WIRELESS => true,
                    _ => continue,
                };
                let connected =
                    adapter.OperStatus.0 == UP && !adapter.FirstGatewayAddress.is_null();
                let id = adapter.AdapterName.to_string().unwrap_or_default();
                let own = adapter.FriendlyName.to_string().unwrap_or_default();
                let name = names
                    .iter()
                    .find(|(adapter, _)| same_id(adapter, &id))
                    .map(|(_, name)| name.clone())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(own);
                found.push(Adapter {
                    wireless,
                    connected,
                    name,
                });
            }
        }
        found
    }

    /// Whether two ways of writing an adapter's id mean the same one
    /// (`{AB12...}` and `ab12...`).
    fn same_id(one: &str, other: &str) -> bool {
        let plain = |id: &str| {
            id.trim_matches(|letter| letter == '{' || letter == '}')
                .to_ascii_lowercase()
        };
        plain(one) == plain(other)
    }

    /// The names of the networks the machine is connected to, each with
    /// the id of the adapter it is on: what the system calls the network
    /// (a radio network's own name). Empty where the system does not say.
    fn network_names() -> Vec<(String, String)> {
        use windows::Win32::Networking::NetworkListManager::{
            INetworkConnection, INetworkListManager, NetworkListManager,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        let _com = crate::com::Com::new();
        let mut names = Vec::new();
        // SAFETY: the system's own objects, used as their interfaces say.
        unsafe {
            let Ok(manager) =
                CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL)
            else {
                return names;
            };
            let Ok(connections) = manager.GetNetworkConnections() else {
                return names;
            };
            loop {
                let mut one: [Option<INetworkConnection>; 1] = [None];
                let mut got = 0u32;
                if connections.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                    break;
                }
                let Some(connection) = one[0].take() else {
                    break;
                };
                let (Ok(adapter), Ok(network)) =
                    (connection.GetAdapterId(), connection.GetNetwork())
                else {
                    continue;
                };
                let name = network
                    .GetName()
                    .map(|name| name.to_string())
                    .unwrap_or_default();
                names.push((format!("{adapter:?}"), name));
            }
        }
        names
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn an_adapters_id_is_the_same_with_and_without_braces() {
            assert!(super::same_id(
                "{4D36E972-E325-11CE-BFC1-08002BE10318}",
                "4d36e972-e325-11ce-bfc1-08002be10318"
            ));
            assert!(!super::same_id("{4D36E972}", "{4D36E973}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_battery_is_read_from_what_the_kernel_says() {
        assert_eq!(
            parse_battery("Battery\n", "87\n", "Discharging\n"),
            Some(Battery {
                percent: 87,
                charging: false
            })
        );
        assert!(parse_battery("Battery", "100", "Full").unwrap().charging);
        assert!(parse_battery("Battery", "40", "Charging").unwrap().charging);
        assert_eq!(
            parse_battery("Battery", "140", "Unknown").unwrap().percent,
            100
        );
        assert_eq!(parse_battery("Mains", "", ""), None);
        assert_eq!(parse_battery("Battery", "", "Charging"), None);
    }

    #[test]
    fn a_battery_is_read_from_windows_power_status() {
        let battery = |flags, percent, mains| battery_of(flags, percent, mains);
        assert_eq!(
            battery(1, 87, 0),
            Some(Battery {
                percent: 87,
                charging: false
            })
        );
        // Charging (flag 8), on the mains.
        assert!(battery(8, 40, 1).unwrap().charging);
        // Full and plugged in.
        assert!(battery(1, 100, 1).unwrap().charging);
        // A machine without a battery; a state that is not known.
        assert_eq!(battery(128, 255, 1), None);
        assert_eq!(battery(255, 255, 255), None);
        assert_eq!(battery(1, 255, 0), None);
    }

    #[test]
    fn the_network_of_adapters_is_the_cable_before_the_radio() {
        let adapter = |wireless, connected, name: &str| Adapter {
            wireless,
            connected,
            name: name.to_string(),
        };
        let both = [adapter(true, true, "Home"), adapter(false, true, "Network 2")];
        assert_eq!(
            network_of(&both),
            Some(Network {
                connected: true,
                wireless: false,
                name: "Network 2".to_string()
            })
        );
        let radio = [adapter(false, false, "Ethernet"), adapter(true, true, "Home")];
        assert_eq!(
            network_of(&radio).map(|network| (network.wireless, network.name)),
            Some((true, "Home".to_string()))
        );
        let none = [adapter(false, false, "Ethernet")];
        assert_eq!(network_of(&none).map(|network| network.connected), Some(false));
        assert_eq!(network_of(&[]), None);
    }

    #[test]
    fn the_network_is_the_cable_before_the_radio() {
        let both = "wifi:connected:Home\\: upstairs\nethernet:connected:Wired connection 1\nloopback:connected (externally):lo\n";
        assert_eq!(
            parse_network(both),
            Some(Network {
                connected: true,
                wireless: false,
                name: "Wired connection 1".to_string()
            })
        );
        let radio = "ethernet:unavailable:\nwifi:connected:Home\\: upstairs\n";
        assert_eq!(
            parse_network(radio),
            Some(Network {
                connected: true,
                wireless: true,
                name: "Home: upstairs".to_string()
            })
        );
        let none = "ethernet:unavailable:\nwifi:disconnected:\n";
        assert_eq!(
            parse_network(none).map(|network| network.connected),
            Some(false)
        );
        assert_eq!(parse_network("loopback:connected (externally):lo\n"), None);
        assert_eq!(parse_network(""), None);
    }
}
