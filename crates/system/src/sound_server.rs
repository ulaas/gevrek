//! The session's sound server, spoken to in its own protocol.
//!
//! PulseAudio's native protocol, which PipeWire's sound server answers
//! as well (`pipewire-pulse`): one connection to its socket, kept for
//! as long as it holds, on which the default devices, every device and
//! every program that plays are asked for and set. Where there is no
//! such socket the callers fall back on the server's own command
//! (`sound.rs`).
//!
//! The server is the one `PULSE_SERVER` names (`unix:/path`), else the
//! socket in the session's runtime folder. A name that is no socket on
//! this machine means that there is none: nothing else is tried, so a
//! program that is told a server speaks to that one or to none.

use crate::sound::{Device, Level, Mixer, Output, Stream};
use pulseaudio::protocol::{self, ChannelVolume, Command, Prop, Volume};
use std::ffi::CString;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;

/// The server's socket, by what the environment says.
pub fn socket_of(
    server: Option<&str>,
    runtime_path: Option<&str>,
    runtime_dir: Option<&str>,
) -> Option<PathBuf> {
    if let Some(server) = server.filter(|server| !server.is_empty()) {
        // (Only a socket of this machine is spoken to; a server on the
        // network is another program's to reach.)
        return server.strip_prefix("unix:").map(PathBuf::from);
    }
    runtime_path
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join("native"))
        .or_else(|| {
            runtime_dir
                .filter(|folder| !folder.is_empty())
                .map(|folder| PathBuf::from(folder).join("pulse").join("native"))
        })
}

/// How loud the channels of a device or a program are, as one measure
/// (0 to 1): the loudest of them.
pub fn measure_of(volume: &ChannelVolume) -> f32 {
    volume
        .channels()
        .iter()
        .map(|channel| channel.as_u32())
        .max()
        .map_or(0.0, |raw| raw as f32 / Volume::NORM.as_u32() as f32)
}

/// The channels after a step of so many percent of full: each by the
/// same (what is louder on one side stays so), none under nothing, none
/// above full.
pub fn stepped(volume: &ChannelVolume, by_percent: i32) -> ChannelVolume {
    let full = i64::from(Volume::NORM.as_u32());
    let by = full * i64::from(by_percent) / 100;
    let mut after = ChannelVolume::empty();
    for channel in volume.channels() {
        let raw = (i64::from(channel.as_u32()) + by).clamp(0, full);
        after.push(Volume::from_u32_clamped(raw as u32));
    }
    after
}

/// So many channels, each at this percent of full.
pub fn at_percent(channels: usize, percent: u32) -> ChannelVolume {
    let raw = u64::from(Volume::NORM.as_u32()) * u64::from(percent.min(100)) / 100;
    let mut volume = ChannelVolume::empty();
    for _ in 0..channels.max(1) {
        volume.push(Volume::from_u32_clamped(raw as u32));
    }
    volume
}

fn level_of(volume: &ChannelVolume, muted: bool) -> Level {
    Level {
        percent: (measure_of(volume) * 100.0).round() as u32,
        muted,
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_string()
}

/// A connection that was let in.
struct Server {
    socket: BufReader<UnixStream>,
    version: u16,
    next: u32,
}

impl Server {
    fn connect() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        let path = socket_of(
            var("PULSE_SERVER").as_deref(),
            var("PULSE_RUNTIME_PATH").as_deref(),
            var("XDG_RUNTIME_DIR").as_deref(),
        )?;
        let stream = UnixStream::connect(path).ok()?;
        // (A server that does not answer is not waited for long: the
        // panel asks from its own loop.)
        let wait = Some(std::time::Duration::from_millis(1500));
        stream.set_read_timeout(wait).ok()?;
        stream.set_write_timeout(wait).ok()?;
        let mut server = Self {
            socket: BufReader::new(stream),
            version: protocol::MAX_VERSION,
            next: 0,
        };
        let cookie = pulseaudio::cookie_path_from_env()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();
        let let_in: protocol::AuthReply = server.ask(&Command::Auth(protocol::AuthParams {
            version: protocol::MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie,
        }))?;
        server.version = protocol::MAX_VERSION.min(let_in.version);
        let mut props = protocol::Props::new();
        props.set(Prop::ApplicationName, CString::new("gevrek").ok()?);
        let _: protocol::SetClientNameReply = server.ask(&Command::SetClientName(props))?;
        Some(server)
    }

    fn send(&mut self, command: &Command) -> Option<()> {
        self.next = self.next.wrapping_add(1);
        protocol::write_command_message(self.socket.get_mut(), self.next, command, self.version)
            .ok()
    }

    /// Ask, and read what the server answers.
    fn ask<T: protocol::CommandReply>(&mut self, command: &Command) -> Option<T> {
        self.send(command)?;
        protocol::read_reply_message::<T>(&mut self.socket, self.version)
            .ok()
            .map(|(_, answer)| answer)
    }

    /// Tell, and read that the server took it.
    fn tell(&mut self, command: &Command) -> Option<()> {
        self.send(command)?;
        protocol::read_ack_message(&mut self.socket)
            .ok()
            .map(|_| ())
    }

    fn level(&mut self, device: Device) -> Option<Level> {
        let info: protocol::ServerInfo = self.ask(&Command::GetServerInfo)?;
        match device {
            Device::Speaker => {
                let name = info.default_sink_name?;
                let sinks: protocol::SinkInfoList = self.ask(&Command::GetSinkInfoList)?;
                let sink = sinks.iter().find(|sink| sink.name == name)?;
                Some(level_of(&sink.cvolume, sink.muted))
            }
            Device::Microphone => {
                let name = info.default_source_name?;
                let sources: protocol::SourceInfoList = self.ask(&Command::GetSourceInfoList)?;
                let source = sources.iter().find(|source| source.name == name)?;
                Some(level_of(&source.cvolume, source.muted))
            }
        }
    }

    /// Change the default device of a kind: its channels by what
    /// `volume` makes of them, its muting by what `muted` makes of it.
    fn change(
        &mut self,
        device: Device,
        volume: impl Fn(&ChannelVolume) -> Option<ChannelVolume>,
        muted: impl Fn(bool) -> Option<bool>,
    ) -> Option<Level> {
        let info: protocol::ServerInfo = self.ask(&Command::GetServerInfo)?;
        let (name, now, is_muted) = match device {
            Device::Speaker => {
                let name = info.default_sink_name?;
                let sinks: protocol::SinkInfoList = self.ask(&Command::GetSinkInfoList)?;
                let sink = sinks.into_iter().find(|sink| sink.name == name)?;
                (name, sink.cvolume, sink.muted)
            }
            Device::Microphone => {
                let name = info.default_source_name?;
                let sources: protocol::SourceInfoList = self.ask(&Command::GetSourceInfoList)?;
                let source = sources.into_iter().find(|source| source.name == name)?;
                (name, source.cvolume, source.muted)
            }
        };
        if let Some(volume) = volume(&now) {
            let params = protocol::SetDeviceVolumeParams {
                device_index: None,
                device_name: Some(name.clone()),
                volume,
            };
            self.tell(&match device {
                Device::Speaker => Command::SetSinkVolume(params),
                Device::Microphone => Command::SetSourceVolume(params),
            })?;
        }
        if let Some(mute) = muted(is_muted) {
            let params = protocol::SetDeviceMuteParams {
                device_index: None,
                device_name: Some(name),
                mute,
            };
            self.tell(&match device {
                Device::Speaker => Command::SetSinkMute(params),
                Device::Microphone => Command::SetSourceMute(params),
            })?;
        }
        self.level(device)
    }

    fn mixer(&mut self) -> Option<Mixer> {
        let info: protocol::ServerInfo = self.ask(&Command::GetServerInfo)?;
        let sinks: protocol::SinkInfoList = self.ask(&Command::GetSinkInfoList)?;
        let sources: protocol::SourceInfoList = self.ask(&Command::GetSourceInfoList)?;
        let playing: protocol::SinkInputInfoList = self.ask(&Command::GetSinkInputInfoList)?;
        let called = |description: &Option<CString>, name: &CString| {
            description
                .as_ref()
                .map(|description| text(description.as_bytes()))
                .filter(|description| !description.is_empty())
                .unwrap_or_else(|| text(name.as_bytes()))
        };
        Some(Mixer {
            outputs: sinks
                .iter()
                .map(|sink| Output {
                    id: sink.index,
                    name: called(&sink.description, &sink.name),
                    chosen: Some(&sink.name) == info.default_sink_name.as_ref(),
                })
                .collect(),
            // (What only listens to a device that plays is no microphone.)
            inputs: sources
                .iter()
                .filter(|source| source.monitor_of_sink_index.is_none())
                .map(|source| Output {
                    id: source.index,
                    name: called(&source.description, &source.name),
                    chosen: Some(&source.name) == info.default_source_name.as_ref(),
                })
                .collect(),
            streams: playing
                .iter()
                .map(|stream| Stream {
                    id: stream.index,
                    name: stream
                        .props
                        .get(Prop::ApplicationName)
                        .map(text)
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| text(stream.name.as_bytes())),
                    level: level_of(&stream.cvolume, stream.muted),
                })
                .collect(),
        })
    }

    fn choose(&mut self, device: Device, id: u32) -> Option<()> {
        match device {
            Device::Speaker => {
                let sinks: protocol::SinkInfoList = self.ask(&Command::GetSinkInfoList)?;
                let sink = sinks.into_iter().find(|sink| sink.index == id)?;
                self.tell(&Command::SetDefaultSink(sink.name))
            }
            Device::Microphone => {
                let sources: protocol::SourceInfoList = self.ask(&Command::GetSourceInfoList)?;
                let source = sources.into_iter().find(|source| source.index == id)?;
                self.tell(&Command::SetDefaultSource(source.name))
            }
        }
    }

    fn stream(&mut self, id: u32) -> Option<protocol::SinkInputInfo> {
        let playing: protocol::SinkInputInfoList = self.ask(&Command::GetSinkInputInfoList)?;
        playing.into_iter().find(|stream| stream.index == id)
    }
}

/// The connection, once made; made again when it broke.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// Do something on the connection. `None`: there is no server (the
/// caller asks another way), or it did not answer: the connection is
/// then given up, and made anew the next time.
fn with<T>(deed: impl Fn(&mut Server) -> Option<T>) -> Option<T> {
    let mut kept = SERVER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // (Twice: a connection that broke since it was last used is made
    // again once.)
    for _ in 0..2 {
        if kept.is_none() {
            *kept = Some(Server::connect()?);
        }
        if let Some(answer) = kept.as_mut().and_then(&deed) {
            return Some(answer);
        }
        *kept = None;
    }
    None
}

/// Whether there is a server to speak to in this way at all.
pub fn there() -> bool {
    with(|server| server.ask::<protocol::ServerInfo>(&Command::GetServerInfo)).is_some()
}

pub fn level(device: Device) -> Option<Level> {
    with(|server| server.level(device))
}

pub fn change(device: Device, by_percent: i32) -> Option<Level> {
    with(|server| server.change(device, |volume| Some(stepped(volume, by_percent)), |_| None))
}

pub fn toggle_mute(device: Device) -> Option<Level> {
    with(|server| server.change(device, |_| None, |muted| Some(!muted)))
}

pub fn mixer() -> Option<Mixer> {
    with(Server::mixer)
}

pub fn choose(device: Device, id: u32) -> Option<()> {
    with(|server| server.choose(device, id))
}

pub fn set_stream(id: u32, percent: u32) -> Option<()> {
    with(|server| {
        let stream = server.stream(id)?;
        server.tell(&Command::SetSinkInputVolume(
            protocol::SetStreamVolumeParams {
                index: id,
                volume: at_percent(stream.cvolume.channels().len(), percent),
            },
        ))
    })
}

pub fn toggle_stream_mute(id: u32) -> Option<()> {
    with(|server| {
        let stream = server.stream(id)?;
        server.tell(&Command::SetSinkInputMute(protocol::SetStreamMuteParams {
            index: id,
            mute: !stream.muted,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(raw: &[u32]) -> ChannelVolume {
        let mut volume = ChannelVolume::empty();
        for raw in raw {
            volume.push(Volume::from_u32_clamped(*raw));
        }
        volume
    }

    fn raw(volume: &ChannelVolume) -> Vec<u32> {
        volume.channels().iter().map(Volume::as_u32).collect()
    }

    #[test]
    fn the_server_is_the_one_that_is_named_or_the_sessions() {
        let path = |text: &str| Some(PathBuf::from(text));
        assert_eq!(
            socket_of(None, None, Some("/run/user/1000")),
            path("/run/user/1000/pulse/native")
        );
        assert_eq!(
            socket_of(None, Some("/run/user/1000/pulse"), Some("/elsewhere")),
            path("/run/user/1000/pulse/native")
        );
        // A named one is the only one: never the session's beside it.
        assert_eq!(
            socket_of(Some("unix:/nonexistent/none"), None, Some("/run/user/1000")),
            path("/nonexistent/none")
        );
        // One on the network is not spoken to, nor is the session's then.
        assert_eq!(
            socket_of(Some("tcp:host:4713"), None, Some("/run/user/1000")),
            None
        );
        assert_eq!(socket_of(None, None, None), None);
        assert_eq!(socket_of(Some(""), Some(""), Some("")), None);
    }

    #[test]
    fn a_step_moves_every_channel_and_keeps_within_nothing_and_full() {
        let full = Volume::NORM.as_u32();
        let five = full * 5 / 100;
        // 40 and 50 of a hundred, five more: 45 and 55.
        let before = channels(&[full * 40 / 100, full / 2]);
        let after = stepped(&before, 5);
        assert_eq!(raw(&after), [full * 40 / 100 + five, full / 2 + five]);
        assert_eq!(
            level_of(&after, false).percent,
            55,
            "the loudest is the measure"
        );
        assert_eq!(raw(&stepped(&after, -5)), raw(&before), "and back");
        // Never above full, never under nothing.
        assert_eq!(
            raw(&stepped(&channels(&[full - 10, full / 2]), 5)),
            [full, full / 2 + five]
        );
        assert_eq!(
            raw(&stepped(&channels(&[100, full / 10]), -5)),
            [0, full / 10 - five]
        );
        // A device driven harder than full comes back to full with a step.
        assert_eq!(raw(&stepped(&channels(&[full + full / 2]), 5)), [full]);
        assert_eq!(measure_of(&ChannelVolume::empty()), 0.0);
    }

    #[test]
    fn a_program_is_set_to_a_percent_on_each_of_its_channels() {
        let full = Volume::NORM.as_u32();
        assert_eq!(raw(&at_percent(2, 50)), [full / 2, full / 2]);
        assert_eq!(raw(&at_percent(1, 250)), [full], "never above full");
        assert_eq!(raw(&at_percent(0, 100)), [full], "one channel at the least");
        assert_eq!(
            level_of(&at_percent(6, 35), true),
            Level {
                percent: 35,
                muted: true
            }
        );
    }

    #[test]
    fn without_a_server_nothing_is_answered() {
        // (What a check's programs are told: a server that is not there.)
        assert!(UnixStream::connect("/nonexistent/gevrek-checks-have-no-sound-server").is_err());
    }
}
