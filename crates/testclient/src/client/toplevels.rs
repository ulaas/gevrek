//! `gevrek-testclient --windows [--do TITLE:what]`: what another panel's
//! taskbar does (wlr-foreign-toplevel-management). Without `--do` it says
//! what the compositor tells of each window; with it, it asks something
//! of the window with that title. `what` is one of `activate`, `close`,
//! `minimize`, `unminimize`, `maximize`, `unmaximize`, `fullscreen`,
//! `unfullscreen`.

use super::{say, word};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

struct Entry {
    handle: ZwlrForeignToplevelHandleV1,
    title: String,
    app_id: String,
    states: Vec<&'static str>,
    outputs: Vec<wl_output::WlOutput>,
    closed: bool,
}

#[derive(Default)]
struct Bar {
    entries: Vec<Entry>,
    outputs: Vec<(wl_output::WlOutput, String)>,
}

pub fn run(connection: &Connection, ask: Option<String>) -> bool {
    let (globals, mut queue) = registry_queue_init::<Bar>(connection).expect("registry");
    let handle = queue.handle();
    let mut bar = Bar::default();
    // The displays first, so that a window's display is one that is known.
    globals.contents().with_list(|list| {
        for global in list {
            if global.interface == "wl_output" {
                let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                    global.name,
                    global.version.min(4),
                    &handle,
                    (),
                );
                bar.outputs.push((output, String::new()));
            }
        }
    });
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&handle, 1..=5, ())
        .ok();
    if queue.roundtrip(&mut bar).is_err() {
        return false;
    }
    let Ok(_manager) = globals.bind::<ZwlrForeignToplevelManagerV1, _, _>(&handle, 1..=3, ())
    else {
        say(
            "error",
            &[("message", word("the compositor has no toplevel management"))],
        );
        return false;
    };
    for _ in 0..2 {
        if queue.roundtrip(&mut bar).is_err() {
            return false;
        }
    }
    let Some(ask) = ask else {
        for entry in bar.entries.iter().filter(|entry| !entry.closed) {
            let outputs: Vec<String> = entry
                .outputs
                .iter()
                .filter_map(|output| bar.outputs.iter().find(|(known, _)| known == output))
                .map(|(_, name)| word(name))
                .collect();
            let states: Vec<String> = entry.states.iter().map(|state| word(state)).collect();
            say(
                "window",
                &[
                    ("title", word(&entry.title)),
                    ("app_id", word(&entry.app_id)),
                    ("states", format!("[{}]", states.join(","))),
                    ("outputs", format!("[{}]", outputs.join(","))),
                ],
            );
        }
        return true;
    };
    let (title, what) = ask.split_once(':').unwrap_or((ask.as_str(), ""));
    let Some(entry) = bar
        .entries
        .iter()
        .find(|entry| !entry.closed && entry.title == title)
    else {
        say("error", &[("message", word("no window with that title"))]);
        return false;
    };
    match what {
        "activate" => match &seat {
            Some(seat) => entry.handle.activate(seat),
            None => return false,
        },
        "close" => entry.handle.close(),
        "minimize" => entry.handle.set_minimized(),
        "unminimize" => entry.handle.unset_minimized(),
        "maximize" => entry.handle.set_maximized(),
        "unmaximize" => entry.handle.unset_maximized(),
        "fullscreen" => entry.handle.set_fullscreen(None),
        "unfullscreen" => entry.handle.unset_fullscreen(),
        _ => {
            say("error", &[("message", word("no such thing to ask"))]);
            return false;
        }
    }
    let done = queue.roundtrip(&mut bar).is_ok();
    say("asked", &[("what", word(what))]);
    done
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Bar {
    fn event(
        bar: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            bar.entries.push(Entry {
                handle: toplevel,
                title: String::new(),
                app_id: String::new(),
                states: Vec::new(),
                outputs: Vec::new(),
                closed: false,
            });
        }
    }

    event_created_child!(Bar, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Bar {
    fn event(
        bar: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event;
        let Some(entry) = bar.entries.iter_mut().find(|entry| entry.handle == *handle) else {
            return;
        };
        match event {
            Event::Title { title } => entry.title = title,
            Event::AppId { app_id } => entry.app_id = app_id,
            Event::OutputEnter { output } => entry.outputs.push(output),
            Event::OutputLeave { output } => entry.outputs.retain(|known| *known != output),
            Event::State { state } => {
                entry.states = state
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|bytes| u32::from_ne_bytes(*bytes))
                    .map(|number| match number {
                        0 => "maximized",
                        1 => "minimized",
                        2 => "activated",
                        3 => "fullscreen",
                        _ => "other",
                    })
                    .collect();
            }
            Event::Closed => entry.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Bar {
    fn event(
        bar: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(known) = bar.outputs.iter_mut().find(|(known, _)| known == output) {
                known.1 = name;
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Bar {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Bar {
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
