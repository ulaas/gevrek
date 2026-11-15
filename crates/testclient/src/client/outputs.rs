//! `gevrek-testclient --displays [--set NAME:what,what…]`: what a display
//! tool does (wlr-output-management). Without `--set` it says what the
//! compositor tells of each display; with it, it asks for a display to be
//! set up and says what the compositor answered. `what` is one of
//! `transform=90`, `scale=1.5`, `position=10x20`, `mode=1280x720`, `off`.

use super::{say, word};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

#[derive(Default)]
struct Head {
    head: Option<ZwlrOutputHeadV1>,
    name: String,
    enabled: bool,
    modes: Vec<(ZwlrOutputModeV1, (i32, i32, i32))>,
    current: Option<ZwlrOutputModeV1>,
    position: (i32, i32),
    transform: String,
    scale: f64,
}

#[derive(Default)]
struct Tool {
    heads: Vec<Head>,
    serial: Option<u32>,
    answer: Option<&'static str>,
}

fn turn(transform: WEnum<wl_output::Transform>) -> String {
    match transform {
        WEnum::Value(wl_output::Transform::Normal) => "normal",
        WEnum::Value(wl_output::Transform::_90) => "90",
        WEnum::Value(wl_output::Transform::_180) => "180",
        WEnum::Value(wl_output::Transform::_270) => "270",
        _ => "other",
    }
    .to_string()
}

pub fn run(connection: &Connection, set: Option<String>) -> bool {
    let (globals, mut queue) = registry_queue_init::<Tool>(connection).expect("registry");
    let handle = queue.handle();
    let Ok(manager) = globals.bind::<ZwlrOutputManagerV1, _, _>(&handle, 1..=2, ()) else {
        say(
            "error",
            &[("message", word("the compositor has no output management"))],
        );
        return false;
    };
    let mut tool = Tool::default();
    while tool.serial.is_none() {
        if queue.blocking_dispatch(&mut tool).is_err() {
            return false;
        }
    }
    let Some(set) = set else {
        for head in &tool.heads {
            let current = head
                .current
                .as_ref()
                .and_then(|current| head.modes.iter().find(|(mode, _)| mode == current))
                .map_or((0, 0, 0), |(_, size)| *size);
            say(
                "head",
                &[
                    ("name", word(&head.name)),
                    ("enabled", head.enabled.to_string()),
                    ("width", current.0.to_string()),
                    ("height", current.1.to_string()),
                    ("refresh", current.2.to_string()),
                    ("x", head.position.0.to_string()),
                    ("y", head.position.1.to_string()),
                    ("transform", word(&head.transform)),
                    ("scale", head.scale.to_string()),
                    ("modes", head.modes.len().to_string()),
                ],
            );
        }
        return true;
    };
    let (name, what) = set.split_once(':').unwrap_or((set.as_str(), ""));
    let configuration = manager.create_configuration(tool.serial.expect("told above"), &handle, ());
    let mut found = false;
    for head in &tool.heads {
        let Some(object) = &head.head else { continue };
        if head.name != name {
            // The others stay as they are.
            configuration.enable_head(object, &handle, ());
            continue;
        }
        found = true;
        if what.split(',').any(|word| word == "off") {
            configuration.disable_head(object);
            continue;
        }
        let asked = configuration.enable_head(object, &handle, ());
        for word in what.split(',').filter(|word| !word.is_empty()) {
            let (key, value) = word.split_once('=').unwrap_or((word, ""));
            match key {
                "transform" => asked.set_transform(match value {
                    "90" => wl_output::Transform::_90,
                    "180" => wl_output::Transform::_180,
                    "270" => wl_output::Transform::_270,
                    "flipped" => wl_output::Transform::Flipped,
                    _ => wl_output::Transform::Normal,
                }),
                "scale" => asked.set_scale(value.parse().expect("scale=NUMBER")),
                "position" => {
                    let (x, y) = value.split_once('x').expect("position=XxY");
                    asked.set_position(x.parse().expect("x"), y.parse().expect("y"));
                }
                "mode" => {
                    let (width, height) = value.split_once('x').expect("mode=WxH");
                    let size: (i32, i32) = (
                        width.parse().expect("width"),
                        height.parse().expect("height"),
                    );
                    match head.modes.iter().find(|(_, mode)| (mode.0, mode.1) == size) {
                        Some((mode, _)) => asked.set_mode(mode),
                        None => asked.set_custom_mode(size.0, size.1, 0),
                    }
                }
                other => panic!("unknown setting {other}"),
            }
        }
    }
    if !found {
        say("error", &[("message", word("no such display"))]);
        return false;
    }
    configuration.apply();
    while tool.answer.is_none() {
        if queue.blocking_dispatch(&mut tool).is_err() {
            return false;
        }
    }
    say("configured", &[("answer", word(tool.answer.unwrap_or("")))]);
    tool.answer == Some("succeeded")
}

impl Dispatch<ZwlrOutputManagerV1, ()> for Tool {
    fn event(
        tool: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => tool.heads.push(Head {
                head: Some(head),
                scale: 1.0,
                ..Default::default()
            }),
            zwlr_output_manager_v1::Event::Done { serial } => tool.serial = Some(serial),
            _ => {}
        }
    }

    event_created_child!(Tool, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for Tool {
    fn event(
        tool: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(head) = tool
            .heads
            .iter_mut()
            .find(|head| head.head.as_ref() == Some(proxy))
        else {
            return;
        };
        match event {
            zwlr_output_head_v1::Event::Name { name } => head.name = name,
            zwlr_output_head_v1::Event::Mode { mode } => head.modes.push((mode, (0, 0, 0))),
            zwlr_output_head_v1::Event::Enabled { enabled } => head.enabled = enabled != 0,
            zwlr_output_head_v1::Event::CurrentMode { mode } => head.current = Some(mode),
            zwlr_output_head_v1::Event::Position { x, y } => head.position = (x, y),
            zwlr_output_head_v1::Event::Transform { transform } => head.transform = turn(transform),
            zwlr_output_head_v1::Event::Scale { scale } => head.scale = scale,
            zwlr_output_head_v1::Event::Finished => head.head = None,
            _ => {}
        }
    }

    event_created_child!(Tool, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for Tool {
    fn event(
        tool: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        for head in &mut tool.heads {
            for (mode, size) in &mut head.modes {
                if mode != proxy {
                    continue;
                }
                match event {
                    zwlr_output_mode_v1::Event::Size { width, height } => {
                        (size.0, size.1) = (width, height);
                    }
                    zwlr_output_mode_v1::Event::Refresh { refresh } => size.2 = refresh,
                    _ => {}
                }
                return;
            }
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for Tool {
    fn event(
        tool: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        tool.answer = Some(match event {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            zwlr_output_configuration_v1::Event::Cancelled => "cancelled",
            _ => return,
        });
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for Tool {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Tool {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
