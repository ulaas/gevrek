//! Other programs' display tools (wlr-output-management): `wlr-randr`,
//! `wdisplays`, `kanshi` and the like see the displays and their modes
//! here and set them up.
//!
//! What such a tool applies is written into `desktop.outputs` of the
//! running configuration and carried out the way a change of the file is,
//! so there is one place the displays are set up from, and the studio and
//! the file say what a tool did. It is kept at once (a change of the file
//! is on trial for a while first): a tool that applies settings by itself
//! as displays come and go could not answer a trial.
//!
//! A display the configuration switches off has no output here, so a tool
//! does not see it and cannot switch it on again; that is done in the
//! configuration.

use crate::state::State;
use gevrek_host::wm::configuration::Configuration as _;
use gevrek_host::wm::reload::Reload as _;
use gevrek_model::outputs::{OutputMode, OutputProfile, OutputTransform};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use std::sync::{Arc, Mutex};

const VERSION: u32 = 2;

/// A mode: width, height, refresh in thousandths of a hertz.
type Mode = (i32, i32, i32);

/// What a tool is told of one display.
#[derive(Debug, Clone, PartialEq)]
struct Facts {
    name: String,
    /// With whether it is the one the display asks for.
    modes: Vec<(Mode, bool)>,
    current: Option<Mode>,
    position: (i32, i32),
    transform: OutputTransform,
    scale: f64,
}

struct Head {
    head: ZwlrOutputHeadV1,
    modes: Vec<(ZwlrOutputModeV1, Mode)>,
    told: Facts,
}

struct Bound {
    manager: ZwlrOutputManagerV1,
    heads: Vec<Head>,
}

/// The tools that are connected, and what they were told.
#[derive(Default)]
pub struct OutputManagement {
    /// Counts the states told: a tool's settings are for the state it saw.
    serial: u32,
    bound: Vec<Bound>,
}

/// What a tool asks for one display.
#[derive(Debug, Clone, Default, PartialEq)]
struct Asked {
    name: String,
    enabled: bool,
    mode: Option<Mode>,
    position: Option<(i32, i32)>,
    transform: Option<OutputTransform>,
    scale: Option<f64>,
}

/// A set of settings a tool is putting together.
#[derive(Debug, Default)]
pub struct Pending {
    serial: u32,
    asked: Vec<Asked>,
    /// Something in it cannot be done here (a mirrored picture).
    impossible: bool,
    used: bool,
}

/// The turn as Wayland names it.
fn turn_out(transform: OutputTransform) -> WlTransform {
    match transform {
        OutputTransform::Normal => WlTransform::Normal,
        OutputTransform::Right => WlTransform::_90,
        OutputTransform::UpsideDown => WlTransform::_180,
        OutputTransform::Left => WlTransform::_270,
    }
}

fn turn_in(transform: WlTransform) -> Option<OutputTransform> {
    match transform {
        WlTransform::Normal => Some(OutputTransform::Normal),
        WlTransform::_90 => Some(OutputTransform::Right),
        WlTransform::_180 => Some(OutputTransform::UpsideDown),
        WlTransform::_270 => Some(OutputTransform::Left),
        // A mirrored picture is not something gevrek draws.
        _ => None,
    }
}

/// `desktop.outputs` with what a tool asks written into it: a display it
/// names gets its settings (one it does not name stays as it was).
fn with_asked(profiles: &[OutputProfile], asked: &[Asked]) -> Vec<OutputProfile> {
    let mut profiles = profiles.to_vec();
    for ask in asked {
        let at = profiles
            .iter()
            .position(|profile| profile.name.trim().eq_ignore_ascii_case(ask.name.trim()));
        let profile = match at {
            Some(at) => &mut profiles[at],
            None => {
                profiles.push(OutputProfile {
                    name: ask.name.clone(),
                    ..Default::default()
                });
                profiles.last_mut().expect("just pushed")
            }
        };
        profile.enabled = ask.enabled;
        if !ask.enabled {
            continue;
        }
        if let Some((width, height, millihertz)) = ask.mode {
            profile.mode = Some(OutputMode {
                width: width.max(0) as u32,
                height: height.max(0) as u32,
                refresh: f64::from(millihertz) / 1000.0,
            });
        }
        if let Some(position) = ask.position {
            profile.position = Some(position);
        }
        if let Some(transform) = ask.transform {
            profile.transform = transform;
        }
        if let Some(scale) = ask.scale {
            profile.scale = Some(scale);
        }
    }
    profiles
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ZwlrOutputManagerV1, ()>(VERSION, ());
}

impl State {
    /// What the tools are told of the displays now.
    fn output_facts(&self) -> Vec<Facts> {
        self.desktop
            .outputs
            .iter()
            .map(|output| {
                let detail = self.core.monitor_manager.detail(output.id);
                let mut modes: Vec<(Mode, bool)> = detail
                    .map(|detail| {
                        detail
                            .modes
                            .iter()
                            .map(|mode| {
                                (
                                    (mode.width, mode.height, mode.refresh_millihertz),
                                    mode.preferred,
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let current = detail
                    .and_then(|detail| detail.modes.iter().find(|mode| mode.current))
                    .map(|mode| (mode.width, mode.height, mode.refresh_millihertz))
                    .unwrap_or((
                        output.mode.0,
                        output.mode.1,
                        detail.map_or(60_000, |detail| detail.refresh_millihertz.max(1)),
                    ));
                // An output that names no modes (one in memory, a window in
                // another session) has the one it is in.
                if !modes.iter().any(|(mode, _)| *mode == current) {
                    modes.push((current, modes.is_empty()));
                }
                Facts {
                    name: output.name.clone(),
                    modes,
                    current: Some(current),
                    position: (output.rect.x, output.rect.y),
                    transform: output.transform,
                    scale: output.scale,
                }
            })
            .collect()
    }

    /// Tell the tools what changed about the displays, if anything did.
    pub(crate) fn refresh_output_management(&mut self) {
        if self.output_management.bound.is_empty() {
            return;
        }
        let facts = self.output_facts();
        let display = self.display.clone();
        let mut management = std::mem::take(&mut self.output_management);
        management.bound.retain(|bound| bound.manager.is_alive());
        let mut changed = false;
        for bound in &mut management.bound {
            changed |= tell(&display, bound, &facts);
        }
        if changed {
            management.serial = management.serial.wrapping_add(1);
            for bound in &management.bound {
                bound.manager.done(management.serial);
            }
        }
        self.output_management = management;
    }

    /// Carry out what a tool asked, or only say whether it could be.
    fn settle_output_configuration(
        &mut self,
        configuration: &ZwlrOutputConfigurationV1,
        pending: &Arc<Mutex<Pending>>,
        apply: bool,
    ) {
        let (serial, asked, impossible) = {
            let mut pending = pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if std::mem::replace(&mut pending.used, true) {
                configuration.post_error(
                    zwlr_output_configuration_v1::Error::AlreadyUsed,
                    "the configuration was applied or tested already",
                );
                return;
            }
            (pending.serial, pending.asked.clone(), pending.impossible)
        };
        // The displays are no longer as the tool saw them.
        if serial != self.output_management.serial {
            configuration.cancelled();
            return;
        }
        let profiles = with_asked(&self.config.desktop.outputs, &asked);
        let nothing_left = !self.desktop.outputs.iter().any(|output| {
            gevrek_model::outputs::profile_of(&profiles, &output.name)
                .is_none_or(|profile| profile.enabled)
        });
        if impossible || nothing_left || !gevrek_model::outputs::problems(&profiles).is_empty() {
            configuration.failed();
            return;
        }
        if apply && profiles != self.config.desktop.outputs {
            log::info!("A display tool set the displays up");
            let mut config = self.config.clone();
            config.desktop.outputs = profiles;
            self.reload_configuration(config);
            // Kept at once (see the top of this file), and in the file.
            self.answer_output_trial(gevrek_model::command::TrialAnswer::Keep);
            self.save_running_configuration();
        }
        configuration.succeeded();
    }
}

/// Tell one tool what it does not know yet. Returns whether anything was
/// told.
fn tell(display: &DisplayHandle, bound: &mut Bound, facts: &[Facts]) -> bool {
    let mut changed = false;
    // Displays that are gone.
    bound.heads.retain(|head| {
        let there = facts.iter().any(|fact| fact.name == head.told.name);
        if !there {
            for (mode, _) in &head.modes {
                mode.finished();
            }
            head.head.finished();
            changed = true;
        }
        there
    });
    let Some(client) = bound.manager.client() else {
        return changed;
    };
    for fact in facts {
        let known = bound
            .heads
            .iter()
            .position(|head| head.told.name == fact.name);
        let at = match known {
            Some(at) => at,
            None => {
                let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, _, State>(
                    display,
                    bound.manager.version(),
                    fact.name.clone(),
                ) else {
                    continue;
                };
                bound.manager.head(&head);
                head.name(fact.name.clone());
                head.description(fact.name.clone());
                bound.heads.push(Head {
                    head,
                    modes: Vec::new(),
                    // (Nothing of it is told yet: all of it differs.)
                    told: Facts {
                        name: fact.name.clone(),
                        modes: Vec::new(),
                        current: None,
                        position: (i32::MIN, i32::MIN),
                        transform: fact.transform,
                        scale: -1.0,
                    },
                });
                changed = true;
                bound.heads.len() - 1
            }
        };
        let head = &mut bound.heads[at];
        if head.told == *fact {
            continue;
        }
        changed = true;
        // Modes the tool does not have yet. (One that is gone stays known
        // to it: displays do not lose modes while they are plugged in.)
        for (mode, preferred) in &fact.modes {
            if head.modes.iter().any(|(_, known)| known == mode) {
                continue;
            }
            let Ok(object) = client.create_resource::<ZwlrOutputModeV1, _, State>(
                display,
                head.head.version(),
                *mode,
            ) else {
                continue;
            };
            head.head.mode(&object);
            object.size(mode.0, mode.1);
            object.refresh(mode.2);
            if *preferred {
                object.preferred();
            }
            head.modes.push((object, *mode));
        }
        head.head.enabled(1);
        if let Some((object, _)) = fact
            .current
            .and_then(|current| head.modes.iter().find(|(_, mode)| *mode == current))
        {
            head.head.current_mode(object);
        }
        head.head.position(fact.position.0, fact.position.1);
        head.head.transform(turn_out(fact.transform));
        head.head.scale(fact.scale);
        head.told = fact.clone();
    }
    changed
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        log::debug!(
            "{} binds the output manager (it may set the displays up)",
            crate::state::named_client(client)
        );
        let manager = init.init(resource, ());
        let mut bound = Bound {
            manager,
            heads: Vec::new(),
        };
        tell(display, &mut bound, &state.output_facts());
        bound.manager.done(state.output_management.serial);
        state.output_management.bound.push(bound);
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _: &(),
        _display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                init.init(
                    id,
                    Arc::new(Mutex::new(Pending {
                        serial,
                        ..Default::default()
                    })),
                );
            }
            zwlr_output_manager_v1::Request::Stop => {
                manager.finished();
                state
                    .output_management
                    .bound
                    .retain(|bound| bound.manager != *manager);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputHeadV1, String> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputHeadV1,
        _: zwlr_output_head_v1::Request,
        _: &String,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, Mode> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputModeV1,
        _: zwlr_output_mode_v1::Request,
        _: &Mode,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Arc<Mutex<Pending>>> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        configuration: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        pending: &Arc<Mutex<Pending>>,
        _display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let named = |head: &ZwlrOutputHeadV1| head.data::<String>().cloned().unwrap_or_default();
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let name = named(&head);
                pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .asked
                    .push(Asked {
                        name: name.clone(),
                        enabled: true,
                        ..Default::default()
                    });
                init.init(id, (Arc::clone(pending), name));
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .asked
                    .push(Asked {
                        name: named(&head),
                        enabled: false,
                        ..Default::default()
                    });
            }
            zwlr_output_configuration_v1::Request::Apply => {
                state.settle_output_configuration(configuration, pending, true);
            }
            zwlr_output_configuration_v1::Request::Test => {
                state.settle_output_configuration(configuration, pending, false);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, (Arc<Mutex<Pending>>, String)> for State {
    fn request(
        _: &mut Self,
        _client: &Client,
        _: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        (pending, name): &(Arc<Mutex<Pending>>, String),
        _display: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let mut pending = pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut impossible = false;
        if let Some(asked) = pending.asked.iter_mut().find(|asked| asked.name == *name) {
            match request {
                zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                    asked.mode = mode.data::<Mode>().copied();
                }
                zwlr_output_configuration_head_v1::Request::SetCustomMode {
                    width,
                    height,
                    refresh,
                } => asked.mode = Some((width, height, refresh)),
                zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                    asked.position = Some((x, y));
                }
                zwlr_output_configuration_head_v1::Request::SetTransform { transform } => {
                    match transform {
                        WEnum::Value(transform) => match turn_in(transform) {
                            Some(transform) => asked.transform = Some(transform),
                            None => impossible = true,
                        },
                        WEnum::Unknown(_) => impossible = true,
                    }
                }
                zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                    asked.scale = Some(scale);
                }
                _ => {}
            }
        }
        pending.impossible |= impossible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_tool_asks_is_written_into_the_displays_of_the_configuration() {
        let before = vec![
            OutputProfile {
                name: "dp-1".to_string(),
                primary: true,
                variable_refresh: true,
                ..Default::default()
            },
            OutputProfile {
                name: "HDMI-A-1".to_string(),
                scale: Some(2.0),
                ..Default::default()
            },
        ];
        let asked = [
            Asked {
                name: "DP-1".to_string(),
                enabled: true,
                mode: Some((2560, 1440, 143_973)),
                position: Some((1920, 0)),
                transform: Some(OutputTransform::Right),
                scale: Some(1.25),
            },
            Asked {
                name: "DP-2".to_string(),
                enabled: false,
                ..Default::default()
            },
        ];
        let after = with_asked(&before, &asked);
        // The display by the name the file has for it, whatever the case;
        // what the tool did not ask about is kept.
        let first = &after[0];
        assert_eq!(first.name, "dp-1");
        assert!(first.primary && first.variable_refresh && first.enabled);
        let mode = first.mode.expect("a mode");
        assert_eq!((mode.width, mode.height), (2560, 1440));
        assert!((mode.refresh - 143.973).abs() < 1e-9);
        assert_eq!(first.position, Some((1920, 0)));
        assert_eq!(first.transform, OutputTransform::Right);
        assert_eq!(first.scale, Some(1.25));
        // One it did not name is as it was.
        assert_eq!(after[1], before[1]);
        // One the file did not have is added, here switched off.
        assert_eq!(after.len(), 3);
        assert!(after[2].name == "DP-2" && !after[2].enabled);
    }

    #[test]
    fn a_turn_is_named_the_same_way_both_ways_and_a_mirror_is_refused() {
        for transform in [
            OutputTransform::Normal,
            OutputTransform::Right,
            OutputTransform::UpsideDown,
            OutputTransform::Left,
        ] {
            assert_eq!(turn_in(turn_out(transform)), Some(transform));
        }
        assert_eq!(turn_in(WlTransform::Flipped), None);
        assert_eq!(turn_in(WlTransform::Flipped90), None);
    }
}
