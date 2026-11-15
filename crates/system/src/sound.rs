//! The volume of what is heard and of the microphone.
//!
//! On Linux, asked of and told to the session's sound server in its own
//! protocol, on one connection that is kept (`sound_server.rs`). Where
//! that server's socket is not there, through PipeWire's own command
//! instead (`wpctl`, of its session manager), started once for every
//! question.
//!
//! On Windows, the default device of each kind is asked through the
//! system's own interface for it (the endpoint volume of Core Audio), and
//! [`watch`] tells when what is heard changes, whoever changed it (the
//! keyboard's keys are the system's there): told by the system, not asked
//! for on a timer.

#[cfg(not(windows))]
use std::process::Command;

/// Which device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// What is heard (the default output).
    Speaker,
    /// The default microphone.
    Microphone,
}

#[cfg(not(windows))]
impl Device {
    fn name(self) -> &'static str {
        match self {
            Device::Speaker => "@DEFAULT_AUDIO_SINK@",
            Device::Microphone => "@DEFAULT_AUDIO_SOURCE@",
        }
    }
}

/// How loud a device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// 0 to 100 (more where the server lets a device be driven harder).
    pub percent: u32,
    pub muted: bool,
}

/// Read the sound server's answer: `Volume: 0.35` or `Volume: 0.35 [MUTED]`.
pub fn parse_level(answer: &str) -> Option<Level> {
    let rest = answer.trim().strip_prefix("Volume:")?.trim();
    let number = rest.split_whitespace().next()?.parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some(Level {
        percent: (number * 100.0).round() as u32,
        muted: rest.contains("[MUTED]"),
    })
}

/// A device's own measure of how loud it is (0 to 1) as a percentage.
pub fn percent_of(scalar: f32) -> u32 {
    if scalar.is_finite() {
        (scalar.clamp(0.0, 1.0) * 100.0).round() as u32
    } else {
        0
    }
}

/// The measure a device has after a step of so many percent of full:
/// never under nothing, never above full.
pub fn stepped(scalar: f32, by_percent: i32) -> f32 {
    (scalar + by_percent as f32 / 100.0).clamp(0.0, 1.0)
}

#[cfg(not(windows))]
fn ask(arguments: &[&str]) -> Option<String> {
    let output = Command::new("wpctl").args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// How loud the device is now. `None`: there is no sound server to ask.
#[cfg(not(windows))]
pub fn level(device: Device) -> Option<Level> {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::level(device);
    }
    parse_level(&ask(&["get-volume", device.name()])?)
}

/// Make the device louder or quieter by so many percent of full, never
/// above full. Returns how loud it is afterwards.
#[cfg(not(windows))]
pub fn change(device: Device, by_percent: i32) -> Option<Level> {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::change(device, by_percent);
    }
    let step = format!(
        "{}%{}",
        by_percent.unsigned_abs(),
        if by_percent < 0 { "-" } else { "+" }
    );
    ask(&["set-volume", "--limit", "1.0", device.name(), &step])?;
    level(device)
}

/// Mute the device, or let it be heard again. Returns how it is afterwards.
#[cfg(not(windows))]
pub fn toggle_mute(device: Device) -> Option<Level> {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::toggle_mute(device);
    }
    ask(&["set-mute", device.name(), "toggle"])?;
    level(device)
}

/// A device sound can be played on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The sound server's number for it.
    pub id: u32,
    pub name: String,
    /// It is the one sound goes to now.
    pub chosen: bool,
}

/// A program that plays sound now, with how loud it is by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// The sound server's number for what it plays.
    pub id: u32,
    pub name: String,
    pub level: Level,
}

/// What the sound server has: the devices to play on and to record
/// from, and the programs that play.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mixer {
    pub outputs: Vec<Output>,
    /// The devices to record from (microphones).
    pub inputs: Vec<Output>,
    pub streams: Vec<Stream>,
}

/// The devices to play on that the sound server's own list names
/// (`wpctl status`: the lines under "Sinks" of its "Audio" part).
pub fn outputs_in(status: &str) -> Vec<Output> {
    devices_in(status, "Sinks")
}

/// The devices to record from that the list names (under "Sources").
pub fn inputs_in(status: &str) -> Vec<Output> {
    devices_in(status, "Sources")
}

/// The programs that play now, by the list (under "Streams" of its
/// "Audio" part): each with its number and the name it gives itself. A
/// program that records is no such one (its lines of connections come
/// in, `<`), nor one that is paused (it has none).
pub fn streams_in(status: &str) -> Vec<(u32, String)> {
    let mut found: Vec<(u32, String, bool)> = Vec::new();
    let mut in_audio = false;
    let mut in_streams = false;
    for line in status.lines() {
        if !line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            in_audio = line.trim() == "Audio";
            in_streams = false;
            continue;
        }
        let words = line.trim_start_matches([' ', '│', '├', '└', '─', '\t']);
        if line.contains("├─") || line.contains("└─") {
            in_streams = in_audio && words.trim_end().trim_end_matches(':') == "Streams";
            continue;
        }
        if !in_streams {
            continue;
        }
        let Some((number, rest)) = words.split_once('.') else {
            continue;
        };
        let Ok(id) = number.trim().parse::<u32>() else {
            continue;
        };
        let rest = rest.trim();
        // A connection of the program before it: `output_FL > device`.
        if let Some((port, _)) = rest.split_once(" > ") {
            if !port.trim_start().starts_with("input") {
                if let Some(last) = found.last_mut() {
                    last.2 = true;
                }
            }
            continue;
        }
        if rest.contains(" < ") {
            continue;
        }
        if !rest.is_empty() {
            found.push((id, rest.to_string(), false));
        }
    }
    found
        .into_iter()
        .filter(|(_, _, plays)| *plays)
        .map(|(id, name, _)| (id, name))
        .collect()
}

/// The devices of one kind that the list names: the lines under that
/// heading of its "Audio" part.
fn devices_in(status: &str, heading: &str) -> Vec<Output> {
    let mut found = Vec::new();
    let mut in_audio = false;
    let mut in_sinks = false;
    for line in status.lines() {
        // A part of the list begins at the start of its line.
        if !line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            in_audio = line.trim() == "Audio";
            in_sinks = false;
            continue;
        }
        let words = line.trim_start_matches([' ', '│', '├', '└', '─', '\t']);
        if line.contains("├─") || line.contains("└─") {
            in_sinks = in_audio && words.trim_end().trim_end_matches(':') == heading;
            continue;
        }
        if !in_sinks {
            continue;
        }
        let (chosen, rest) = match words.strip_prefix('*') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, words),
        };
        let Some((number, name)) = rest.split_once('.') else {
            continue;
        };
        let Ok(id) = number.trim().parse::<u32>() else {
            continue;
        };
        // (Its loudness follows the name in brackets.)
        let name = name
            .rsplit_once("[vol:")
            .map_or(name, |(name, _)| name)
            .trim();
        if !name.is_empty() {
            found.push(Output {
                id,
                name: name.to_string(),
                chosen,
            });
        }
    }
    found
}

/// The devices sound can be played on. Empty: there is no sound server
/// to ask.
#[cfg(not(windows))]
pub fn outputs() -> Vec<Output> {
    #[cfg(target_os = "linux")]
    if let Some(mixer) = crate::sound_server::mixer() {
        return mixer.outputs;
    }
    ask(&["status"])
        .map(|status| outputs_in(&status))
        .unwrap_or_default()
}

/// Play on this device from now on. Whether the sound server took it.
#[cfg(not(windows))]
pub fn choose_output(id: u32) -> bool {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::choose(Device::Speaker, id).is_some();
    }
    ask(&["set-default", &id.to_string()]).is_some()
}

/// The devices and the programs that play, as the sound server has them
/// now. Nothing in it: there is no sound server to ask.
#[cfg(not(windows))]
pub fn mixer() -> Mixer {
    #[cfg(target_os = "linux")]
    if let Some(mixer) = crate::sound_server::mixer() {
        return mixer;
    }
    let Some(status) = ask(&["status"]) else {
        return Mixer::default();
    };
    let streams = streams_in(&status)
        .into_iter()
        .filter_map(|(id, name)| {
            let level = ask(&["get-volume", &id.to_string()])
                .as_deref()
                .and_then(parse_level)?;
            Some(Stream { id, name, level })
        })
        .collect();
    Mixer {
        outputs: outputs_in(&status),
        inputs: inputs_in(&status),
        streams,
    }
}

/// Record from this device from now on. Whether the sound server took it.
#[cfg(not(windows))]
pub fn choose_input(id: u32) -> bool {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::choose(Device::Microphone, id).is_some();
    }
    ask(&["set-default", &id.to_string()]).is_some()
}

/// Make one program so loud, in percent of full (the others and the
/// device stay as they are). Whether the sound server took it.
#[cfg(not(windows))]
pub fn set_stream(id: u32, percent: u32) -> bool {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::set_stream(id, percent).is_some();
    }
    let measure = format!("{:.2}", f64::from(percent.min(100)) / 100.0);
    ask(&["set-volume", &id.to_string(), &measure]).is_some()
}

/// Mute one program, or let it be heard again.
#[cfg(not(windows))]
pub fn toggle_stream_mute(id: u32) -> bool {
    #[cfg(target_os = "linux")]
    if crate::sound_server::there() {
        return crate::sound_server::toggle_stream_mute(id).is_some();
    }
    ask(&["set-mute", &id.to_string(), "toggle"]).is_some()
}

/// On Windows: the devices to play on and to record from as the system
/// lists them, and the programs that play now on the device that is
/// heard, each known by the number of its process.
#[cfg(windows)]
pub fn mixer() -> Mixer {
    Mixer {
        outputs: outputs(),
        inputs: inputs(),
        streams: streams(),
    }
}

#[cfg(windows)]
pub use endpoint::{
    change, choose_input, choose_output, inputs, level, measure, outputs, set, set_stream,
    streams, toggle_mute, toggle_stream_mute, watch,
};

/// What a program that plays is called: the name it gave what it plays,
/// where it gave one that is a name (not a place in a file of the
/// system's, which is what the system's own sounds have); else its
/// program's file without the ending.
pub fn stream_name(given: &str, program: &str) -> String {
    let given = given.trim();
    if !given.is_empty() && !given.starts_with('@') {
        return given.to_string();
    }
    let file = program.rsplit(['\\', '/']).next().unwrap_or(program);
    let name = file
        .rsplit_once('.')
        .filter(|(_, ending)| ending.eq_ignore_ascii_case("exe"))
        .map_or(file, |(name, _)| name);
    if name.is_empty() {
        "A program".to_string()
    } else {
        name.to_string()
    }
}

// (The names of the system's undocumented interface are the system's, and
// most of its calls stand there only to give the one that is used its place.)
#[cfg(windows)]
#[allow(non_snake_case, dead_code)]
mod endpoint {
    use super::{percent_of, stepped, Device, Level, Output};
    use std::ffi::c_void;
    use std::sync::{mpsc, Arc};
    use windows::core::{implement, Result, HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
        DEVICE_STATE,
    };
    use windows::Win32::Media::Audio::{eCommunications, eMultimedia, DEVICE_STATE_ACTIVE};
    use windows::Win32::Media::Audio::{
        AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume,
    };
    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Com::{CoTaskMemFree, STGM_READ};
    use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

    /// What is told of a change.
    type Tell = Arc<dyn Fn(Level) + Send + Sync>;

    fn devices() -> Result<IMMDeviceEnumerator> {
        // SAFETY: an object of the system is asked for by its identifier.
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
    }

    /// The volume of the default device of this kind, as it is now.
    fn volume_of(devices: &IMMDeviceEnumerator, device: Device) -> Result<IAudioEndpointVolume> {
        let flow = match device {
            Device::Speaker => eRender,
            Device::Microphone => eCapture,
        };
        // SAFETY: the calls of the interfaces, with what they ask for.
        unsafe {
            devices
                .GetDefaultAudioEndpoint(flow, eConsole)?
                .Activate(CLSCTX_ALL, None)
        }
    }

    fn read(volume: &IAudioEndpointVolume) -> Result<Level> {
        // SAFETY: both calls only read.
        unsafe {
            Ok(Level {
                percent: percent_of(volume.GetMasterVolumeLevelScalar()?),
                muted: volume.GetMute()?.as_bool(),
            })
        }
    }

    /// Do something with the default device of a kind; `None` when there
    /// is no such device, or it refuses.
    fn with<T>(device: Device, act: impl FnOnce(&IAudioEndpointVolume) -> Result<T>) -> Option<T> {
        let _com = crate::com::Com::new();
        let done = devices()
            .and_then(|devices| volume_of(&devices, device))
            .and_then(|volume| act(&volume));
        match done {
            Ok(value) => Some(value),
            Err(error) => {
                log::debug!("the sound device does not answer: {error}");
                None
            }
        }
    }

    /// How loud the device is now. `None`: there is no such device.
    pub fn level(device: Device) -> Option<Level> {
        with(device, read)
    }

    /// Make the device louder or quieter by so many percent of full, never
    /// above full. Returns how loud it is afterwards.
    pub fn change(device: Device, by_percent: i32) -> Option<Level> {
        with(device, |volume| {
            // SAFETY: the calls of the interface; no context is named for
            // the change, so everyone who listens is told of it.
            unsafe {
                let now = volume.GetMasterVolumeLevelScalar()?;
                volume.SetMasterVolumeLevelScalar(stepped(now, by_percent), std::ptr::null())?;
            }
            read(volume)
        })
    }

    /// The device's own measure of how loud it is, from 0 to 1, as it
    /// stands: what [`set`] puts back exactly.
    pub fn measure(device: Device) -> Option<f32> {
        // SAFETY: the call only reads.
        with(device, |volume| unsafe {
            volume.GetMasterVolumeLevelScalar()
        })
    }

    /// Make the device exactly this loud, by its own measure (0 to 1).
    /// Returns how loud it is afterwards.
    pub fn set(device: Device, measure: f32) -> Option<Level> {
        with(device, |volume| {
            // SAFETY: as in `change`.
            unsafe {
                volume.SetMasterVolumeLevelScalar(measure.clamp(0.0, 1.0), std::ptr::null())?;
            }
            read(volume)
        })
    }

    /// Mute the device, or let it be heard again. Returns how it is afterwards.
    pub fn toggle_mute(device: Device) -> Option<Level> {
        with(device, |volume| {
            // SAFETY: as in `change`.
            unsafe {
                let muted = volume.GetMute()?.as_bool();
                volume.SetMute(!muted, std::ptr::null())?;
            }
            read(volume)
        })
    }

    /// The name the system shows for a device (`PKEY_Device_FriendlyName`).
    const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    /// The system's own way of saying which device is the default one. It
    /// is in no header of the system's: the sound settings use it, and so
    /// has every program that offers this choice since Windows 7. Only
    /// the one call that is used is ever made; the others stand here
    /// because the call's place among them is what finds it.
    #[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: windows_core::IUnknown {
        unsafe fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        unsafe fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            default: i32,
            format: *mut *mut c_void,
        ) -> HRESULT;
        unsafe fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        unsafe fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint: *mut c_void,
            mix: *mut c_void,
        ) -> HRESULT;
        unsafe fn GetProcessingPeriod(
            &self,
            device: PCWSTR,
            default: i32,
            usual: *mut i64,
            least: *mut i64,
        ) -> HRESULT;
        unsafe fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        unsafe fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        unsafe fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        unsafe fn GetPropertyValue(
            &self,
            device: PCWSTR,
            store: i32,
            key: *const c_void,
            value: *mut c_void,
        ) -> HRESULT;
        unsafe fn SetPropertyValue(
            &self,
            device: PCWSTR,
            store: i32,
            key: *const c_void,
            value: *mut c_void,
        ) -> HRESULT;
        unsafe fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        unsafe fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }

    /// The object that answers `IPolicyConfig`.
    const POLICY_CONFIG: windows::core::GUID =
        windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    /// The devices to play on that are there now, each with the system's
    /// own name for it (what it is known by between programs) and the
    /// name it shows, in the system's order.
    fn playing_devices(devices: &IMMDeviceEnumerator) -> Result<Vec<(String, String)>> {
        devices_of(devices, eRender)
    }

    /// The devices of a kind (to play on, to record from) that are there
    /// now, each with the system's own name for it and the name it shows.
    fn devices_of(devices: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Vec<(String, String)>> {
        let mut found = Vec::new();
        // SAFETY: the calls of the interfaces; what the system hands out
        // to be freed is freed.
        unsafe {
            let all = devices.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
            for index in 0..all.GetCount()? {
                let device = all.Item(index)?;
                let id = taken(device.GetId()?);
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .and_then(|store| store.GetValue(&FRIENDLY_NAME))
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                found.push((
                    id,
                    if name.is_empty() {
                        id_as_name(index)
                    } else {
                        name
                    },
                ));
            }
        }
        Ok(found)
    }

    fn id_as_name(index: u32) -> String {
        format!("Device {}", index + 1)
    }

    /// A text the system allocated for us: copied, and given back.
    unsafe fn taken(text: windows::core::PWSTR) -> String {
        let copy = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const c_void));
        copy
    }

    /// The devices sound can be played on, numbered in the system's order
    /// (the number holds until a device comes or goes: long enough for a
    /// menu). Empty: there is none, or the system does not answer.
    pub fn outputs() -> Vec<Output> {
        listed(eRender)
    }

    /// The devices sound can be recorded from, numbered as [`outputs`]
    /// are; the one in use is the one programs record from unless they
    /// choose another themselves.
    pub fn inputs() -> Vec<Output> {
        listed(eCapture)
    }

    fn listed(flow: EDataFlow) -> Vec<Output> {
        let _com = crate::com::Com::new();
        let listed = devices().and_then(|devices| {
            // SAFETY: as above.
            let chosen = unsafe {
                devices
                    .GetDefaultAudioEndpoint(flow, eConsole)
                    .and_then(|device| device.GetId())
                    .map(|id| taken(id))
                    .unwrap_or_default()
            };
            Ok(devices_of(&devices, flow)?
                .into_iter()
                .enumerate()
                .map(|(index, (id, name))| Output {
                    id: index as u32,
                    name,
                    chosen: id == chosen,
                })
                .collect::<Vec<_>>())
        });
        match listed {
            Ok(outputs) => outputs,
            Err(error) => {
                log::debug!("the sound devices are not listed: {error}");
                Vec::new()
            }
        }
    }

    /// Play on this device from now on (for everything: what programs
    /// play, and what they play to talk). Whether the system took it.
    pub fn choose_output(id: u32) -> bool {
        choose(eRender, id)
    }

    /// Record from this device from now on (for every program that does
    /// not choose one itself). Whether the system took it.
    pub fn choose_input(id: u32) -> bool {
        choose(eCapture, id)
    }

    fn choose(flow: EDataFlow, id: u32) -> bool {
        let _com = crate::com::Com::new();
        let chosen = devices().and_then(|devices| {
            let Some((device, _)) = devices_of(&devices, flow)?.into_iter().nth(id as usize) else {
                return Ok(false);
            };
            let wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: the object is the system's; the text lives until the
            // calls are over.
            unsafe {
                let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG, None, CLSCTX_ALL)?;
                for role in [eConsole, eMultimedia, eCommunications] {
                    policy
                        .SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role)
                        .ok()?;
                }
            }
            Ok(true)
        });
        match chosen {
            Ok(taken) => taken,
            Err(error) => {
                log::warn!("the sound device was not changed: {error}");
                false
            }
        }
    }

    /// What plays on the device that is heard: every stream of a program
    /// (a program may have several), with the number of its process, the
    /// name it gave, and its own volume. Not the system's own sounds, and
    /// not what is there and silent (a program that paused).
    fn playing(devices: &IMMDeviceEnumerator) -> Result<Vec<(u32, String, ISimpleAudioVolume)>> {
        let mut found = Vec::new();
        // SAFETY: the calls of the interfaces; what the system hands out
        // to be freed is freed.
        unsafe {
            let manager: IAudioSessionManager2 = devices
                .GetDefaultAudioEndpoint(eRender, eConsole)?
                .Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for index in 0..sessions.GetCount()? {
                let Ok(session) = sessions
                    .GetSession(index)
                    .and_then(|session| session.cast::<IAudioSessionControl2>())
                else {
                    continue;
                };
                // (Yes is said with the plain "all is well".)
                if session.IsSystemSoundsSession().0 == 0 {
                    continue;
                }
                if session.GetState().ok() != Some(AudioSessionStateActive) {
                    continue;
                }
                let Ok(process) = session.GetProcessId() else {
                    continue;
                };
                let given = session
                    .GetDisplayName()
                    .map(|name| taken(name))
                    .unwrap_or_default();
                let Ok(volume) = session.cast::<ISimpleAudioVolume>() else {
                    continue;
                };
                found.push((process, given, volume));
            }
        }
        Ok(found)
    }

    /// The file of a running program, by the number of its process.
    fn program_of(process: u32) -> String {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: the handle is closed below; the buffer is ours.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) else {
                return String::new();
            };
            let mut path = [0u16; 512];
            let mut length = path.len() as u32;
            let named = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut length,
            )
            .is_ok();
            let _ = CloseHandle(handle);
            if named {
                String::from_utf16_lossy(&path[..length as usize])
            } else {
                String::new()
            }
        }
    }

    /// The programs that play now, one line for each (what a program
    /// plays in several streams is one: as loud as the loudest, muted
    /// when all are), known by the number of the program's process.
    pub fn streams() -> Vec<super::Stream> {
        let _com = crate::com::Com::new();
        let listed = devices().and_then(|devices| {
            let mut streams: Vec<super::Stream> = Vec::new();
            for (process, given, volume) in playing(&devices)? {
                // SAFETY: both calls only read.
                let (loud, muted) = unsafe {
                    (
                        volume.GetMasterVolume().unwrap_or(1.0),
                        volume.GetMute().map(|muted| muted.as_bool()).unwrap_or(false),
                    )
                };
                let level = Level {
                    percent: percent_of(loud),
                    muted,
                };
                match streams.iter_mut().find(|stream| stream.id == process) {
                    Some(known) => {
                        known.level.percent = known.level.percent.max(level.percent);
                        known.level.muted &= level.muted;
                    }
                    None => streams.push(super::Stream {
                        id: process,
                        name: super::stream_name(&given, &program_of(process)),
                        level,
                    }),
                }
            }
            Ok(streams)
        });
        match listed {
            Ok(streams) => streams,
            Err(error) => {
                log::debug!("the programs that play are not listed: {error}");
                Vec::new()
            }
        }
    }

    /// Do something with every stream of one program; whether it had one
    /// and all of them took it.
    fn with_streams(process: u32, act: impl Fn(&ISimpleAudioVolume) -> Result<()>) -> bool {
        let _com = crate::com::Com::new();
        let done = devices().and_then(|devices| {
            let mut any = false;
            for (_, _, volume) in playing(&devices)?
                .into_iter()
                .filter(|(owner, ..)| *owner == process)
            {
                act(&volume)?;
                any = true;
            }
            Ok(any)
        });
        match done {
            Ok(any) => any,
            Err(error) => {
                log::warn!("a program's own volume was not changed: {error}");
                false
            }
        }
    }

    /// Make one program so loud, in percent of full (of what the device
    /// is set to: its own share).
    pub fn set_stream(process: u32, percent: u32) -> bool {
        let measure = percent.min(100) as f32 / 100.0;
        // SAFETY: the call of the interface; no context is named.
        with_streams(process, |volume| unsafe {
            volume.SetMasterVolume(measure, std::ptr::null())
        })
    }

    /// Mute one program, or let it be heard again (all of what it plays
    /// the same way: muted unless all of it is already).
    pub fn toggle_stream_mute(process: u32) -> bool {
        let muted = streams()
            .iter()
            .find(|stream| stream.id == process)
            .is_some_and(|stream| stream.level.muted);
        // SAFETY: as above.
        with_streams(process, |volume| unsafe {
            volume.SetMute(!muted, std::ptr::null())
        })
    }

    /// Told by the system when the volume of the device it is given to
    /// changes.
    #[implement(IAudioEndpointVolumeCallback)]
    struct Changed(Tell);

    impl IAudioEndpointVolumeCallback_Impl for Changed_Impl {
        fn OnNotify(&self, notice: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
            // SAFETY: the system hands over a structure that lives for
            // the length of this call, or nothing.
            if let Some(notice) = unsafe { notice.as_ref() } {
                (self.0)(Level {
                    percent: percent_of(notice.fMasterVolume),
                    muted: notice.bMuted.as_bool(),
                });
            }
            Ok(())
        }
    }

    /// Told by the system when another device becomes the default one.
    #[implement(IMMNotificationClient)]
    struct Another(mpsc::Sender<()>);

    impl IMMNotificationClient_Impl for Another_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
            Ok(())
        }

        fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }

        fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }

        fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
            if flow == eRender && role == eConsole {
                let _ = self.0.send(());
            }
            Ok(())
        }

        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
            Ok(())
        }
    }

    /// Tell `tell` each time what is heard is louder, quieter, muted or
    /// heard again, whoever did it, and when another device becomes the
    /// one that is heard (then with that device's volume). The system may
    /// tell the same level twice: who listens compares. On a thread of
    /// its own, for as long as the program runs. `false` when the thread
    /// could not be started.
    pub fn watch(tell: impl Fn(Level) + Send + Sync + 'static) -> bool {
        let tell: Tell = Arc::new(tell);
        std::thread::Builder::new()
            .name("gevrek-sound".to_string())
            .spawn(move || {
                if let Err(error) = watching(tell) {
                    log::warn!("changes of the volume are not heard of: {error}");
                }
            })
            .is_ok()
    }

    fn watching(tell: Tell) -> Result<()> {
        // The system calls back on threads of its own: this thread only
        // keeps what it registered, and waits.
        // SAFETY: the thread is this function's own.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        let devices = devices()?;
        let (another, changes) = mpsc::channel();
        let client: IMMNotificationClient = Another(another).into();
        // SAFETY: `client` lives as long as this function, which does
        // not return while the program runs.
        unsafe { devices.RegisterEndpointNotificationCallback(&client)? };
        let mut first = true;
        loop {
            // The device that is heard now, if there is one, is listened
            // to until another takes its place.
            let listened = volume_of(&devices, Device::Speaker).and_then(|volume| {
                let callback: IAudioEndpointVolumeCallback = Changed(Arc::clone(&tell)).into();
                // SAFETY: taken back below, before either is dropped.
                unsafe { volume.RegisterControlChangeNotify(&callback)? };
                Ok((volume, callback))
            });
            // Another device is heard: as loud as that one is.
            if let (Ok((volume, _)), false) = (&listened, first) {
                if let Ok(level) = read(volume) {
                    tell(level);
                }
            }
            first = false;
            let more = changes.recv().is_ok();
            if let Ok((volume, callback)) = &listened {
                // SAFETY: the callback registered above.
                let _ = unsafe { volume.UnregisterControlChangeNotify(callback) };
            }
            if !more {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_servers_answer_is_read() {
        assert_eq!(
            parse_level("Volume: 0.35\n"),
            Some(Level {
                percent: 35,
                muted: false
            })
        );
        assert_eq!(
            parse_level("Volume: 1.00 [MUTED]\n"),
            Some(Level {
                percent: 100,
                muted: true
            })
        );
        assert_eq!(
            parse_level("Volume: 0.00"),
            Some(Level {
                percent: 0,
                muted: false
            })
        );
        assert_eq!(
            parse_level("Volume: 1.25"),
            Some(Level {
                percent: 125,
                muted: false
            })
        );
        assert_eq!(parse_level("Could not connect to PipeWire"), None);
        assert_eq!(parse_level("Volume: loud"), None);
    }

    #[test]
    fn a_devices_measure_is_a_percentage_and_a_step_stays_within_it() {
        assert_eq!(percent_of(0.0), 0);
        assert_eq!(percent_of(0.354), 35);
        assert_eq!(percent_of(1.0), 100);
        assert_eq!(percent_of(1.7), 100);
        assert_eq!(percent_of(f32::NAN), 0);
        assert!((stepped(0.40, 5) - 0.45).abs() < 1e-6);
        assert!((stepped(0.40, -5) - 0.35).abs() < 1e-6);
        // Never above full, never under nothing.
        assert_eq!(stepped(0.98, 5), 1.0);
        assert_eq!(stepped(0.02, -5), 0.0);
    }

    #[test]
    fn a_program_that_plays_is_called_by_its_own_name_or_its_files() {
        assert_eq!(stream_name("Spotify", r"C:\Apps\Spotify.exe"), "Spotify");
        assert_eq!(stream_name("", r"C:\Program Files\Mozilla Firefox\firefox.exe"), "firefox");
        assert_eq!(stream_name("  ", "vlc.EXE"), "vlc");
        // A place in one of the system's files is no name.
        assert_eq!(
            stream_name(r"@%SystemRoot%\System32\AudioSrv.Dll,-202", r"C:\x\player.exe"),
            "player"
        );
        assert_eq!(stream_name("", "my.app.exe"), "my.app");
        assert_eq!(stream_name("", ""), "A program");
    }

    #[cfg(windows)]
    #[test]
    fn the_system_says_what_its_sound_has() {
        // Whatever this machine has: asking changes nothing and does not
        // fail; one device of a kind at most is the one in use; a program
        // that is not there has nothing to set.
        let mixer = mixer();
        println!("{mixer:?}");
        assert!(mixer.outputs.iter().filter(|device| device.chosen).count() <= 1);
        assert!(mixer.inputs.iter().filter(|device| device.chosen).count() <= 1);
        assert!(mixer.streams.iter().all(|stream| !stream.name.is_empty()));
        assert!(!set_stream(0xFFFF_FFF0, 50));
        assert!(!choose_input(10_000));
    }

    /// With a program that plays (the number of its process in
    /// `GEVREK_TEST_STREAM`): it is listed, made so loud, muted and heard
    /// again, and left as loud as it was. Only that program's own volume
    /// is touched. `cargo test -p gevrek-system own_volume -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs a program that plays: GEVREK_TEST_STREAM=<its process>"]
    fn a_programs_own_volume_is_set_and_muted() {
        let process: u32 = std::env::var("GEVREK_TEST_STREAM")
            .expect("GEVREK_TEST_STREAM")
            .parse()
            .expect("the number of a process");
        let of = || {
            streams()
                .into_iter()
                .find(|stream| stream.id == process)
                .expect("the program plays")
        };
        let was = of();
        println!("{was:?}");
        assert!(!was.name.is_empty());
        assert!(set_stream(process, 40));
        assert_eq!(of().level.percent, 40);
        assert!(toggle_stream_mute(process));
        assert_eq!(of().level.muted, !was.level.muted);
        assert!(toggle_stream_mute(process));
        assert_eq!(of().level.muted, was.level.muted);
        assert!(set_stream(process, was.level.percent));
        assert_eq!(of().level, was.level);
        // The device to record from: the one that is in use is chosen
        // again (the system takes the choice, and nothing is another).
        let before = inputs();
        if let Some(in_use) = before.iter().find(|device| device.chosen) {
            assert!(choose_input(in_use.id));
            assert_eq!(inputs(), before);
        }
    }
}

/// The name of a sound of the desktop's sound theme as it may be handed
/// to a program: letters, digits, `-` and `_` only (a sender of a
/// notification names it), else the sound for a message.
pub fn event_name(asked: Option<&str>) -> String {
    const MESSAGE: &str = "message-new-instant";
    match asked.map(str::trim) {
        Some(name)
            if name.starts_with(|letter: char| letter.is_ascii_alphanumeric())
                && name.len() <= 64
                && name.chars().all(|letter| {
                    letter.is_ascii_alphanumeric() || letter == '-' || letter == '_'
                }) =>
        {
            name.to_string()
        }
        _ => MESSAGE.to_string(),
    }
}

/// Play a sound of the desktop's sound theme by its name
/// (`message-new-instant`), without waiting for it: through the sound
/// theme's own player where it is installed, else the file of the usual
/// theme through the sound server's. Nothing is heard, and nothing said,
/// where neither is there.
#[cfg(target_os = "linux")]
pub fn play_event(asked: Option<&str>) {
    let name = event_name(asked);
    let spawned = std::thread::Builder::new()
        .name("gevrek-sound".to_string())
        .spawn(move || {
            let by_theme = Command::new("canberra-gtk-play")
                .args(["-i", &name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if by_theme.is_ok_and(|status| status.success()) {
                return;
            }
            let file = format!("/usr/share/sounds/freedesktop/stereo/{name}.oga");
            if std::path::Path::new(&file).is_file() {
                let _ = Command::new("pw-play")
                    .arg(&file)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        });
    if let Err(error) = spawned {
        log::debug!("no thread to play a sound on: {error}");
    }
}

/// The system plays its own sounds here.
#[cfg(not(target_os = "linux"))]
pub fn play_event(_asked: Option<&str>) {}

#[cfg(test)]
mod event_tests {
    use super::event_name;

    #[test]
    fn the_programs_that_play_and_the_microphones_are_read_from_the_list() {
        let status = "PipeWire 'pipewire-0' [1.6]\n └─ Clients:\n        34. WirePlumber\n\nAudio\n ├─ Devices:\n │      74. A card [alsa]\n │  \n ├─ Sinks:\n │  *   48. Speakers [vol: 0.30]\n │  \n ├─ Sources:\n │      59. Built-in microphone [vol: 1.00]\n │  *   71. Webcam Analog Stereo [vol: 0.80]\n │  \n ├─ Filters:\n │  \n └─ Streams:\n        86. Firefox                                 \n             94. output_FR       > Speakers:playback_FR\t[active]\n             95. output_FL       > Speakers:playback_FL\t[active]\n        99. PipeWire ALSA [audacity]\n            100. input_FL        < Webcam:capture_FL\t[active]\n       101. mpv\n       103. Music player\n            104. output_FL       > Speakers:playback_FL\t[init]\n\nVideo\n ├─ Sources:\n │  *   33. Webcam (V4L2)\n └─ Streams:\n        120. A camera's viewer\n            121. output_0 > somewhere\n";
        let inputs = super::inputs_in(status);
        assert_eq!(
            inputs
                .iter()
                .map(|input| (input.id, input.name.as_str(), input.chosen))
                .collect::<Vec<_>>(),
            vec![
                (59, "Built-in microphone", false),
                (71, "Webcam Analog Stereo", true)
            ]
        );
        // Who plays: not the one that records, not the one that is
        // paused, not what is under "Video".
        assert_eq!(
            super::streams_in(status),
            vec![
                (86, "Firefox".to_string()),
                (103, "Music player".to_string())
            ]
        );
        assert!(super::streams_in("").is_empty());
        assert_eq!(super::outputs_in(status).len(), 1);
    }

    #[test]
    fn the_devices_to_play_on_are_read_from_the_servers_list() {
        let status = "PipeWire 'pipewire-0' [1.6.11]\n └─ Clients:\n        34. WirePlumber  [1.6.11]\n\nAudio\n ├─ Devices:\n │      54. Chipset Audio [alsa]\n │  \n ├─ Sinks:\n │      46. max                                 [vol: 1.00]\n │  *   58. Chipset Audio Analog Stereo [vol: 0.25]\n │      67. HDMI Digital Stereo (HDMI) [27GL850] [vol: 0.40 MUTED]\n │  \n ├─ Sources:\n │  *   57. Webcam Analog Stereo [vol: 1.00]\n │  \n └─ Streams:\n\nVideo\n ├─ Sinks:\n │      99. A screen [vol: 1.00]\n";
        let found = super::outputs_in(status);
        let seen: Vec<(u32, &str, bool)> = found
            .iter()
            .map(|output| (output.id, output.name.as_str(), output.chosen))
            .collect();
        assert_eq!(
            seen,
            [
                (46, "max", false),
                (58, "Chipset Audio Analog Stereo", true),
                (67, "HDMI Digital Stereo (HDMI) [27GL850]", false),
            ]
        );
        assert!(super::outputs_in("").is_empty());
        assert!(super::outputs_in("Audio\n └─ Sources:\n │  *   57. Webcam\n").is_empty());
    }

    #[test]
    fn a_sound_is_named_by_plain_letters_or_is_the_one_for_a_message() {
        assert_eq!(event_name(Some("dialog-warning")), "dialog-warning");
        assert_eq!(event_name(Some(" bell_2 ")), "bell_2");
        assert_eq!(event_name(None), "message-new-instant");
        assert_eq!(event_name(Some("")), "message-new-instant");
        // Nothing that could be read as an option or a path.
        assert_eq!(event_name(Some("../../etc/passwd")), "message-new-instant");
        assert_eq!(event_name(Some("a b")), "message-new-instant");
        assert_eq!(event_name(Some("-f; rm")), "message-new-instant");
        assert_eq!(event_name(Some("--version")), "message-new-instant");
    }
}
