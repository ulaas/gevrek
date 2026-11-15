//! Wireless networks: whether the machine's radio is on, the networks in
//! reach, and joining one. Asked of NetworkManager through its own tool,
//! `nmcli` (`GEVREK_NMCLI` names another program in its place: the checks
//! join nothing). A machine without the tool, or without a wireless
//! device, has nothing of this to show.
//!
//! On Windows the same is asked of the system's own wireless service
//! (the WLAN interface): the radio, the networks it has heard, the ones
//! it keeps a profile for. A network joined for the first time gets a
//! profile, as one joined from Windows' own list does; a password goes
//! to the service inside that profile, never on a command line.

/// A wireless network in reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    /// Its name.
    pub name: String,
    /// How well it is heard, 0 to 100.
    pub signal: u8,
    /// It asks for a password.
    pub secured: bool,
    /// The machine is on it.
    pub in_use: bool,
    /// The machine has been on it before: it is joined without asking.
    pub known: bool,
}

/// How the wireless side stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wifi {
    /// The radio is on.
    pub enabled: bool,
    /// The networks in reach: the one in use first, then by how well
    /// they are heard.
    pub networks: Vec<Network>,
}

/// The parts of one line of the tool's terse output: divided by colons, a
/// colon that belongs to a part written `\:` (and a backslash `\\`).
pub fn fields(line: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut letters = line.chars();
    while let Some(letter) = letters.next() {
        match letter {
            '\\' => {
                if let Some(next) = letters.next() {
                    parts.last_mut().expect("never empty").push(next);
                }
            }
            ':' => parts.push(String::new()),
            other => parts.last_mut().expect("never empty").push(other),
        }
    }
    parts
}

/// Whether the tool's list of devices (`-f TYPE device`) names a
/// wireless one.
pub fn has_device(devices: &str) -> bool {
    devices
        .lines()
        .any(|line| fields(line).first().is_some_and(|kind| kind == "wifi"))
}

/// The names of the wireless connections the machine keeps
/// (`-f NAME,TYPE connection show`).
pub fn known_in(connections: &str) -> Vec<String> {
    connections
        .lines()
        .map(fields)
        .filter(|parts| parts.get(1).is_some_and(|kind| kind == "802-11-wireless"))
        .filter_map(|parts| parts.into_iter().next())
        .collect()
}

/// The networks of the tool's list (`-f IN-USE,SIGNAL,SECURITY,SSID
/// device wifi list`): each name once (a network heard from several
/// places counts as its best), the one in use first, then by how well
/// they are heard. A network that hides its name is left out.
pub fn networks_in(list: &str, known: &[String]) -> Vec<Network> {
    let mut found: Vec<Network> = Vec::new();
    for parts in list.lines().map(fields) {
        let [in_use, signal, security, name] = parts.as_slice() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let network = Network {
            name: name.clone(),
            signal: signal.trim().parse::<u8>().unwrap_or(0).min(100),
            secured: !security.trim().is_empty() && security.trim() != "--",
            in_use: in_use.trim() == "*",
            known: known.iter().any(|kept| kept == name),
        };
        match found.iter_mut().find(|other| other.name == network.name) {
            Some(other) => {
                other.in_use |= network.in_use;
                other.signal = other.signal.max(network.signal);
            }
            None => found.push(network),
        }
    }
    in_order(found)
}

/// The networks as they are listed: the one in use first, then by how
/// well they are heard, then by name.
pub fn in_order(mut found: Vec<Network>) -> Vec<Network> {
    found.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.signal.cmp(&a.signal))
            .then(a.name.cmp(&b.name))
    });
    found
}

/// How a network is kept safe, as a profile of Windows' names it: the
/// authentication and the encryption. `None`: a kind that is not joined
/// with a password alone (a company's network with accounts).
pub fn profile_security(secured: bool, wpa3: bool, old_wpa: bool, tkip: bool) -> Option<(&'static str, &'static str)> {
    match (secured, wpa3, old_wpa) {
        (false, _, _) => Some(("open", "none")),
        (true, true, _) => Some(("WPA3SAE", "AES")),
        (true, false, true) => Some(("WPAPSK", if tkip { "TKIP" } else { "AES" })),
        (true, false, false) => Some(("WPA2PSK", if tkip { "TKIP" } else { "AES" })),
    }
}

/// Text as it stands inside XML.
pub fn in_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The profile Windows keeps for a network joined with a password (or
/// with none, for an open one): its name is the network's.
pub fn profile_xml(name: &str, security: (&str, &str), password: Option<&str>) -> String {
    let hex: String = name.bytes().map(|byte| format!("{byte:02X}")).collect();
    let key = match password.filter(|password| !password.is_empty()) {
        Some(password) if security.0 != "open" => format!(
            "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            in_xml(password)
        ),
        _ => String::new(),
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
         <name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig>\
         <connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
         <MSM><security><authEncryption><authentication>{}</authentication><encryption>{}</encryption>\
         <useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>",
        security.0,
        security.1,
        name = in_xml(name),
    )
}

#[cfg(target_os = "linux")]
fn tool() -> std::process::Command {
    let program = std::env::var_os("GEVREK_NMCLI").unwrap_or_else(|| "nmcli".into());
    let mut command = std::process::Command::new(program);
    command.stderr(std::process::Stdio::null());
    command
}

#[cfg(target_os = "linux")]
fn ask(arguments: &[&str]) -> Option<String> {
    let output = tool()
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// How the wireless side stands now. `None`: there is no tool to ask, or
/// the machine has no wireless device. (The list is the one the system
/// keeps; it is not made to look for networks again.)
#[cfg(target_os = "linux")]
pub fn read() -> Option<Wifi> {
    let devices = ask(&["-t", "-f", "TYPE", "device"])?;
    if !has_device(&devices) {
        return None;
    }
    let enabled = ask(&["-t", "-f", "WIFI", "radio"]).is_some_and(|said| said.trim() == "enabled");
    if !enabled {
        return Some(Wifi::default());
    }
    let known = ask(&["-t", "-f", "NAME,TYPE", "connection", "show"])
        .map(|connections| known_in(&connections))
        .unwrap_or_default();
    let networks = ask(&[
        "-t",
        "-f",
        "IN-USE,SIGNAL,SECURITY,SSID",
        "device",
        "wifi",
        "list",
        "--rescan",
        "no",
    ])
    .map(|list| networks_in(&list, &known))
    .unwrap_or_default();
    Some(Wifi {
        enabled: true,
        networks,
    })
}

/// Ask the system to look for networks again (it does so by itself only
/// now and then, and not at once when the radio comes on). Whether it
/// took the request: it refuses one that comes too soon after another.
#[cfg(target_os = "linux")]
pub fn rescan() -> bool {
    ask(&["device", "wifi", "rescan"]).is_some()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn rescan() -> bool {
    false
}

/// Switch the radio on, or off. Whether the system took it.
#[cfg(target_os = "linux")]
pub fn set_enabled(on: bool) -> bool {
    ask(&["radio", "wifi", if on { "on" } else { "off" }]).is_some()
}

/// Join a network: one the machine keeps by its kept settings, another
/// with the password given (none for an open one). The password is
/// handed to the tool on its input, not on its command line, where
/// every program of the machine could read it. Whether it was joined;
/// this takes as long as joining takes (seconds).
#[cfg(target_os = "linux")]
pub fn join(name: &str, known: bool, password: Option<&str>) -> bool {
    use std::io::Write;
    if known {
        return ask(&["--wait", "20", "connection", "up", "id", name]).is_some();
    }
    let Some(password) = password.filter(|password| !password.is_empty()) else {
        return ask(&["--wait", "20", "device", "wifi", "connect", name]).is_some();
    };
    let Ok(mut child) = tool()
        .args(["--wait", "20", "--ask", "device", "wifi", "connect", name])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut input) = child.stdin.take() {
        let _ = writeln!(input, "{password}");
    }
    child.wait().is_ok_and(|status| status.success())
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn read() -> Option<Wifi> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn set_enabled(_on: bool) -> bool {
    false
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn join(_name: &str, _known: bool, _password: Option<&str>) -> bool {
    false
}

#[cfg(windows)]
pub use system::{join, read, rescan, set_enabled};

/// Windows' wireless service.
#[cfg(windows)]
mod system {
    use super::{Network, Wifi};
    use windows::core::{GUID, HSTRING, PCWSTR};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        dot11_BSS_type_infrastructure, dot11_radio_state_off, dot11_radio_state_on,
        wlan_connection_mode_profile, wlan_intf_opcode_radio_state, WlanCloseHandle, WlanConnect,
        WlanDeleteProfile, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
        WlanOpenHandle, WlanQueryInterface, WlanScan, WlanSetInterface, WlanSetProfile,
        DOT11_AUTH_ALGO_WPA, DOT11_AUTH_ALGO_WPA3_SAE, DOT11_AUTH_ALGO_WPA_PSK,
        DOT11_CIPHER_ALGO_TKIP, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_LIST,
        WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST, WLAN_PHY_RADIO_STATE,
        WLAN_RADIO_STATE,
    };

    /// The network is the one the machine is on / the machine keeps a
    /// profile for it (`WLAN_AVAILABLE_NETWORK_CONNECTED`, `_HAS_PROFILE`).
    const CONNECTED: u32 = 1;
    const HAS_PROFILE: u32 = 2;

    /// The service, open for as long as this lives, and the machine's
    /// first wireless device.
    struct Service {
        handle: HANDLE,
        device: GUID,
    }

    impl Service {
        fn open() -> Option<Self> {
            let mut version = 0u32;
            let mut handle = HANDLE::default();
            // SAFETY: both are ours to be written to; the handle is
            // closed when this is let go of.
            if unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) } != 0 {
                return None;
            }
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            // SAFETY: the list is the service's until it is given back.
            let device = unsafe {
                let found = WlanEnumInterfaces(handle, None, &mut list) == 0
                    && !list.is_null()
                    && (*list).dwNumberOfItems > 0;
                let device = found.then(|| (*list).InterfaceInfo[0].InterfaceGuid);
                if !list.is_null() {
                    WlanFreeMemory(list.cast());
                }
                device
            };
            match device {
                Some(device) => Some(Self { handle, device }),
                None => {
                    // SAFETY: the handle opened above.
                    unsafe { WlanCloseHandle(handle, None) };
                    None
                }
            }
        }

        /// How many radios the device has, and whether all of them are on.
        fn radio(&self) -> Option<(u32, bool)> {
            let mut size = 0u32;
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: what is handed over is the service's until it is
            // given back; it is read as what the question's kind says.
            unsafe {
                if WlanQueryInterface(
                    self.handle,
                    &self.device,
                    wlan_intf_opcode_radio_state,
                    None,
                    &mut size,
                    &mut data,
                    None,
                ) != 0
                    || data.is_null()
                {
                    return None;
                }
                let state = &*data.cast::<WLAN_RADIO_STATE>();
                let count = state.dwNumberOfPhys.min(64);
                let on = state.PhyRadioState[..count as usize].iter().all(|radio| {
                    radio.dot11SoftwareRadioState == dot11_radio_state_on
                        && radio.dot11HardwareRadioState == dot11_radio_state_on
                });
                WlanFreeMemory(data);
                Some((count, on && count > 0))
            }
        }

        /// The networks the device has heard, as the service lists them:
        /// each with its name, the profile kept for it, and how it is
        /// kept safe.
        fn heard(&self) -> Vec<(Network, String, WLAN_AVAILABLE_NETWORK)> {
            let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
            // SAFETY: the list is the service's until it is given back;
            // it says itself how many entries follow its head.
            unsafe {
                if WlanGetAvailableNetworkList(self.handle, &self.device, 0, None, &mut list) != 0
                    || list.is_null()
                {
                    return Vec::new();
                }
                let entries = std::slice::from_raw_parts(
                    (*list).Network.as_ptr(),
                    (*list).dwNumberOfItems as usize,
                );
                let found = entries
                    .iter()
                    .filter_map(|entry| {
                        let length = (entry.dot11Ssid.uSSIDLength as usize).min(32);
                        let name =
                            String::from_utf8_lossy(&entry.dot11Ssid.ucSSID[..length]).into_owned();
                        if name.is_empty() {
                            return None;
                        }
                        let profile = String::from_utf16_lossy(&entry.strProfileName)
                            .trim_end_matches('\0')
                            .to_string();
                        Some((
                            Network {
                                name,
                                signal: entry.wlanSignalQuality.min(100) as u8,
                                secured: entry.bSecurityEnabled.as_bool(),
                                in_use: entry.dwFlags & CONNECTED != 0,
                                known: entry.dwFlags & HAS_PROFILE != 0,
                            },
                            profile,
                            *entry,
                        ))
                    })
                    .collect();
                WlanFreeMemory(list.cast());
                found
            }
        }
    }

    impl Drop for Service {
        fn drop(&mut self) {
            // SAFETY: the handle this opened.
            unsafe { WlanCloseHandle(self.handle, None) };
        }
    }

    /// How the wireless side stands now. `None`: the machine has no
    /// wireless device, or its service does not answer.
    pub fn read() -> Option<Wifi> {
        let service = Service::open()?;
        let (_, on) = service.radio()?;
        if !on {
            return Some(Wifi::default());
        }
        // (A network is listed once with its profile and once without:
        // each name once, with the best of what is said of it.)
        let mut networks: Vec<Network> = Vec::new();
        for (network, _, _) in service.heard() {
            match networks.iter_mut().find(|other| other.name == network.name) {
                Some(other) => {
                    other.in_use |= network.in_use;
                    other.known |= network.known;
                    other.signal = other.signal.max(network.signal);
                }
                None => networks.push(network),
            }
        }
        Some(Wifi {
            enabled: true,
            networks: super::in_order(networks),
        })
    }

    /// Ask the device to look for networks again.
    pub fn rescan() -> bool {
        Service::open().is_some_and(|service| {
            // SAFETY: a plain call.
            unsafe { WlanScan(service.handle, &service.device, None, None, None) == 0 }
        })
    }

    /// Switch the radio on, or off. Whether the system took it.
    pub fn set_enabled(on: bool) -> bool {
        let Some(service) = Service::open() else {
            return false;
        };
        let Some((count, _)) = service.radio() else {
            return false;
        };
        let mut all = count > 0;
        for index in 0..count {
            let state = WLAN_PHY_RADIO_STATE {
                dwPhyIndex: index,
                dot11SoftwareRadioState: if on {
                    dot11_radio_state_on
                } else {
                    dot11_radio_state_off
                },
                dot11HardwareRadioState: dot11_radio_state_on,
            };
            // SAFETY: the state is ours and of the size that is told.
            all &= unsafe {
                WlanSetInterface(
                    service.handle,
                    &service.device,
                    wlan_intf_opcode_radio_state,
                    std::mem::size_of::<WLAN_PHY_RADIO_STATE>() as u32,
                    std::ptr::from_ref(&state).cast(),
                    None,
                )
            } == 0;
        }
        all
    }

    /// Join a network: one the machine keeps by its profile, another by
    /// a profile made for it with the password given (none for an open
    /// one). Whether the machine is on it afterwards; this takes as long
    /// as joining takes (seconds). A profile made here for a network
    /// that was not joined (a wrong password) is taken away again.
    pub fn join(name: &str, known: bool, password: Option<&str>) -> bool {
        let Some(service) = Service::open() else {
            return false;
        };
        let heard = service.heard();
        let kept = heard
            .iter()
            .find(|(network, profile, _)| network.name == name && network.known && !profile.is_empty())
            .map(|(_, profile, _)| profile.clone());
        let (profile, made) = match kept.filter(|_| known) {
            Some(profile) => (profile, false),
            None => {
                let Some((network, _, entry)) =
                    heard.iter().find(|(network, _, _)| network.name == name)
                else {
                    return false;
                };
                let auth = entry.dot11DefaultAuthAlgorithm;
                let Some(security) = super::profile_security(
                    network.secured,
                    auth == DOT11_AUTH_ALGO_WPA3_SAE,
                    auth == DOT11_AUTH_ALGO_WPA_PSK || auth == DOT11_AUTH_ALGO_WPA,
                    entry.dot11DefaultCipherAlgorithm == DOT11_CIPHER_ALGO_TKIP,
                ) else {
                    return false;
                };
                let xml = HSTRING::from(super::profile_xml(name, security, password));
                let mut why = 0u32;
                // SAFETY: the text lives until the call returns.
                let set = unsafe {
                    WlanSetProfile(
                        service.handle,
                        &service.device,
                        0,
                        PCWSTR(xml.as_ptr()),
                        None,
                        true,
                        None,
                        &mut why,
                    )
                };
                if set != 0 {
                    log::warn!("Windows did not take a profile for the wireless network {name} ({set}, {why})");
                    return false;
                }
                (name.to_string(), true)
            }
        };
        let profile_name = HSTRING::from(profile.as_str());
        let wanted = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(profile_name.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };
        // SAFETY: what is wanted lives until the call returns.
        let asked = unsafe { WlanConnect(service.handle, &service.device, &wanted, None) } == 0;
        let mut joined = false;
        if asked {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if service
                    .heard()
                    .iter()
                    .any(|(network, _, _)| network.name == name && network.in_use)
                {
                    joined = true;
                    break;
                }
            }
        }
        if !joined && made {
            // SAFETY: the text lives until the call returns.
            unsafe {
                WlanDeleteProfile(service.handle, &service.device, PCWSTR(profile_name.as_ptr()), None);
            }
        }
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_divided_at_its_colons_and_not_at_those_of_a_name() {
        assert_eq!(fields("*:70:WPA2:Home"), ["*", "70", "WPA2", "Home"]);
        assert_eq!(
            fields(r" :40:WPA1 WPA2:A\:B \\ C"),
            [" ", "40", "WPA1 WPA2", r"A:B \ C"]
        );
        assert_eq!(fields(""), [""]);
    }

    #[test]
    fn the_networks_in_reach_are_listed_once_each_the_one_in_use_first() {
        let list = " :55:WPA2:Cafe\n*:62:WPA2:Home\n :80:WPA2:Home\n :90::Open one\n :30:WPA2:\n :75:--:Also open\nnot a line\n";
        let known = vec!["Home".to_string(), "Somewhere else".to_string()];
        let found = networks_in(list, &known);
        assert_eq!(
            found
                .iter()
                .map(|network| (
                    network.name.as_str(),
                    network.signal,
                    network.secured,
                    network.in_use,
                    network.known
                ))
                .collect::<Vec<_>>(),
            vec![
                ("Home", 80, true, true, true),
                ("Open one", 90, false, false, false),
                ("Also open", 75, false, false, false),
                ("Cafe", 55, true, false, false),
            ]
        );
        assert!(networks_in("", &known).is_empty());
    }

    /// What this machine's wireless side says of itself, printed to be
    /// looked at (only read):
    ///
    ///     cargo test -p gevrek-system wireless_side_is_read -- --ignored --nocapture
    #[test]
    #[ignore = "reads this machine's wireless side and prints it"]
    fn the_wireless_side_is_read_to_be_looked_at() {
        match read() {
            None => println!("no wireless device, or nothing answers"),
            Some(wifi) => {
                println!("the radio is on: {}", wifi.enabled);
                for network in &wifi.networks {
                    println!(
                        "  {:>3}  {}{}{}  {}",
                        network.signal,
                        if network.in_use { "in use " } else { "" },
                        if network.known { "kept " } else { "" },
                        if network.secured { "secured" } else { "open" },
                        network.name
                    );
                }
            }
        }
    }

    /// The radio switched to the other state and back, with what it
    /// hears while it is on. **It really switches this machine's
    /// wireless radio**, for some seconds, and leaves it as it was:
    ///
    ///     cargo test -p gevrek-system radio_is_switched_and_switched_back -- --ignored --nocapture
    #[test]
    #[ignore = "switches this machine's wireless radio, and back"]
    fn the_radio_is_switched_and_switched_back() {
        let Some(before) = read() else {
            println!("no wireless device, or nothing answers");
            return;
        };
        let wait_for = |on: bool| {
            (0..20).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                read().is_some_and(|wifi| wifi.enabled == on)
            })
        };
        println!("the radio was on: {}", before.enabled);
        let other = !before.enabled;
        println!("asked to be {}: taken={}", if other { "on" } else { "off" }, set_enabled(other));
        println!("it is so within ten seconds: {}", wait_for(other));
        if other {
            println!("asked to look for networks: {}", rescan());
            for _ in 0..12 {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                if read().is_some_and(|wifi| !wifi.networks.is_empty()) {
                    break;
                }
            }
            let heard = read().map(|wifi| wifi.networks).unwrap_or_default();
            println!("networks heard: {}", heard.len());
            for network in heard.iter().take(12) {
                println!(
                    "  {:>3}  {}{}{}",
                    network.signal,
                    if network.known { "kept " } else { "" },
                    if network.secured { "secured  " } else { "open  " },
                    network.name
                );
            }
        }
        println!("asked back: taken={}", set_enabled(before.enabled));
        println!("as it was within ten seconds: {}", wait_for(before.enabled));
    }

    #[test]
    fn a_profile_for_windows_names_the_network_and_keeps_its_password_as_text() {
        // How it is kept safe decides what the profile says.
        assert_eq!(profile_security(false, false, false, false), Some(("open", "none")));
        assert_eq!(profile_security(true, false, false, false), Some(("WPA2PSK", "AES")));
        assert_eq!(profile_security(true, true, false, false), Some(("WPA3SAE", "AES")));
        assert_eq!(profile_security(true, false, true, true), Some(("WPAPSK", "TKIP")));
        let xml = profile_xml("Cafe & <Bar>", ("WPA2PSK", "AES"), Some("p\"a<ss"));
        assert!(xml.contains("<name>Cafe &amp; &lt;Bar&gt;</name>"));
        // (The name's bytes, as the profile wants them beside the name.)
        assert!(xml.contains("<hex>436166652026203C4261723E</hex>"));
        assert!(xml.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"));
        assert!(xml.contains("<keyMaterial>p&quot;a&lt;ss</keyMaterial>"));
        // An open one has no key, whatever is handed over.
        let open = profile_xml("Open", ("open", "none"), Some("ignored"));
        assert!(!open.contains("sharedKey") && open.contains("<authentication>open</authentication>"));
        // The order of a list: the one in use, then the best heard.
        let network = |name: &str, signal, in_use| Network {
            name: name.to_string(),
            signal,
            secured: true,
            in_use,
            known: false,
        };
        let names: Vec<String> = in_order(vec![network("b", 90, false), network("a", 40, true), network("c", 90, false)])
            .into_iter()
            .map(|network| network.name)
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn the_kept_wireless_connections_and_the_device_are_found() {
        let connections = "Home:802-11-wireless\nWired connection 1:802-3-ethernet\nlo:loopback\nA\\:B:802-11-wireless\n";
        assert_eq!(known_in(connections), ["Home", "A:B"]);
        assert!(has_device("ethernet\nloopback\nwifi\n"));
        assert!(!has_device("ethernet\nloopback\nwifi-p2p\n"));
        assert!(!has_device(""));
    }
}
