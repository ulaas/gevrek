//! `gevrek-testclient --workspaces [--show DISPLAY:NAME]`: what another
//! panel's workspace list does (ext-workspace). Without `--show` it says
//! what the compositor tells of each workspace; with it, it asks for the
//! workspace of that name on that display to be shown.

use super::{say, word};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{event_created_child, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

struct Group {
    handle: ExtWorkspaceGroupHandleV1,
    outputs: Vec<wl_output::WlOutput>,
    workspaces: Vec<ExtWorkspaceHandleV1>,
}

struct Entry {
    handle: ExtWorkspaceHandleV1,
    id: String,
    name: String,
    index: Option<u32>,
    active: bool,
    can_show: bool,
    removed: bool,
}

#[derive(Default)]
struct List {
    groups: Vec<Group>,
    entries: Vec<Entry>,
    outputs: Vec<(wl_output::WlOutput, String)>,
    done: u32,
}

impl List {
    /// The name of the display a workspace is on.
    fn display_of(&self, workspace: &ExtWorkspaceHandleV1) -> String {
        self.groups
            .iter()
            .find(|group| group.workspaces.contains(workspace))
            .and_then(|group| group.outputs.first())
            .and_then(|output| self.outputs.iter().find(|(known, _)| known == output))
            .map(|(_, name)| name.clone())
            .unwrap_or_default()
    }
}

pub fn run(connection: &Connection, show: Option<String>) -> bool {
    let (globals, mut queue) = registry_queue_init::<List>(connection).expect("registry");
    let handle = queue.handle();
    let mut list = List::default();
    globals.contents().with_list(|known| {
        for global in known {
            if global.interface == "wl_output" {
                let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                    global.name,
                    global.version.min(4),
                    &handle,
                    (),
                );
                list.outputs.push((output, String::new()));
            }
        }
    });
    if queue.roundtrip(&mut list).is_err() {
        return false;
    }
    let Ok(manager) = globals.bind::<ExtWorkspaceManagerV1, _, _>(&handle, 1..=1, ()) else {
        say(
            "error",
            &[("message", word("the compositor has no workspace list"))],
        );
        return false;
    };
    while list.done == 0 {
        if queue.blocking_dispatch(&mut list).is_err() {
            return false;
        }
    }
    let Some(show) = show else {
        for entry in list.entries.iter().filter(|entry| !entry.removed) {
            say(
                "workspace",
                &[
                    ("display", word(&list.display_of(&entry.handle))),
                    ("name", word(&entry.name)),
                    ("id", word(&entry.id)),
                    ("index", entry.index.map_or(-1, i64::from).to_string()),
                    ("active", entry.active.to_string()),
                    ("can_show", entry.can_show.to_string()),
                ],
            );
        }
        return true;
    };
    let (display, name) = show.split_once(':').unwrap_or(("", show.as_str()));
    let Some(entry) = list.entries.iter().find(|entry| {
        !entry.removed && entry.name == name && list.display_of(&entry.handle) == display
    }) else {
        say("error", &[("message", word("no such workspace"))]);
        return false;
    };
    entry.handle.activate();
    manager.commit();
    let done = queue.roundtrip(&mut list).is_ok();
    say("asked", &[("name", word(name))]);
    done
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for List {
    fn event(
        list: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                list.groups.push(Group {
                    handle: workspace_group,
                    outputs: Vec::new(),
                    workspaces: Vec::new(),
                });
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => list.entries.push(Entry {
                handle: workspace,
                id: String::new(),
                name: String::new(),
                index: None,
                active: false,
                can_show: false,
                removed: false,
            }),
            ext_workspace_manager_v1::Event::Done => list.done += 1,
            _ => {}
        }
    }

    event_created_child!(List, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for List {
    fn event(
        list: &mut Self,
        handle: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_workspace_group_handle_v1::Event;
        let Some(group) = list.groups.iter_mut().find(|group| group.handle == *handle) else {
            return;
        };
        match event {
            Event::OutputEnter { output } => group.outputs.push(output),
            Event::OutputLeave { output } => group.outputs.retain(|known| *known != output),
            Event::WorkspaceEnter { workspace } => group.workspaces.push(workspace),
            Event::WorkspaceLeave { workspace } => {
                group.workspaces.retain(|known| *known != workspace)
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for List {
    fn event(
        list: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_workspace_handle_v1::Event;
        let Some(entry) = list
            .entries
            .iter_mut()
            .find(|entry| entry.handle == *handle)
        else {
            return;
        };
        match event {
            Event::Id { id } => entry.id = id,
            Event::Name { name } => entry.name = name,
            Event::Coordinates { coordinates } => {
                entry.index = coordinates
                    .as_chunks::<4>()
                    .0
                    .first()
                    .map(|bytes| u32::from_ne_bytes(*bytes));
            }
            Event::State { state } => {
                entry.active = matches!(state, WEnum::Value(state) if state.contains(ext_workspace_handle_v1::State::Active));
            }
            Event::Capabilities { capabilities } => {
                entry.can_show = matches!(capabilities, WEnum::Value(can) if can.contains(ext_workspace_handle_v1::WorkspaceCapabilities::Activate));
            }
            Event::Removed => entry.removed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for List {
    fn event(
        list: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(known) = list.outputs.iter_mut().find(|(known, _)| known == output) {
                known.1 = name;
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for List {
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
