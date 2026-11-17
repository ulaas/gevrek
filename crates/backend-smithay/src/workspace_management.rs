//! Other panels' workspace lists (ext-workspace): a panel that is not
//! gevrek's (`waybar`'s `ext/workspaces`, and others as they take the
//! protocol up) is told of every display's workspaces, which of them is
//! shown, and can ask for one to be shown.
//!
//! Each display is a group of its own with its own workspaces, as they
//! are in gevrek. Showing a workspace is the command a click on gevrek's
//! own panel gives (`focus_monitor_workspace`, by the display and the
//! workspace's place on it: two displays may each have one of a name).
//! Making and removing workspaces is not offered: gevrek makes them as
//! they are gone to and drops the empty ones itself.

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

const VERSION: u32 = 1;

/// What a panel is told of one workspace.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Facts {
    /// The engine's number of it, the same while it lives.
    id: usize,
    /// What it is known by (to keys and rules; the same from session to
    /// session).
    name: String,
    /// What is shown of it: its title, or its name where it has none.
    /// This is what a panel is told as its name.
    title: String,
    output: MonitorHandle,
    output_name: String,
    /// Its place among the display's workspaces.
    index: usize,
    /// It is the one its display shows.
    active: bool,
}

impl Facts {
    /// A name for it that is the same in the next session.
    fn lasting_id(&self) -> String {
        format!("{}:{}", self.output_name, self.name)
    }
}

struct Group {
    output: MonitorHandle,
    handle: ExtWorkspaceGroupHandleV1,
}

struct Entry {
    handle: ExtWorkspaceHandleV1,
    told: Facts,
}

struct Bound {
    manager: ExtWorkspaceManagerV1,
    groups: Vec<Group>,
    entries: Vec<Entry>,
    /// What the panel asked for since its last `commit`: the workspaces
    /// to show, by the engine's number.
    asked: Vec<usize>,
}

/// The panels that are connected, and what they were told.
#[derive(Default)]
pub struct WorkspaceManagement {
    bound: Vec<Bound>,
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ExtWorkspaceManagerV1, ()>(VERSION, ());
}

fn state_of(fact: &Facts) -> ext_workspace_handle_v1::State {
    if fact.active {
        ext_workspace_handle_v1::State::Active
    } else {
        ext_workspace_handle_v1::State::empty()
    }
}

/// Tell one panel what it does not know yet. Whether anything was told.
fn tell(
    display: &DisplayHandle,
    outputs: &[(MonitorHandle, smithay::output::Output)],
    bound: &mut Bound,
    facts: &[Facts],
) -> bool {
    let Some(client) = bound.manager.client() else {
        return false;
    };
    let mut changed = false;
    // Workspaces that are gone, or went to a display that is gone.
    let groups = &bound.groups;
    bound.entries.retain(|entry| {
        let there = facts.iter().any(|fact| fact.id == entry.told.id);
        if !there {
            if let Some(group) = groups
                .iter()
                .find(|group| group.output == entry.told.output)
            {
                group.handle.workspace_leave(&entry.handle);
            }
            entry.handle.removed();
            changed = true;
        }
        there
    });
    // Displays that are gone.
    bound.groups.retain(|group| {
        let there = outputs.iter().any(|(output, _)| *output == group.output);
        if !there {
            group.handle.removed();
            changed = true;
        }
        there
    });
    // Displays that are new.
    for (output, shown_as) in outputs {
        if bound.groups.iter().any(|group| group.output == *output) {
            continue;
        }
        let Ok(handle) = client.create_resource::<ExtWorkspaceGroupHandleV1, _, State>(
            display,
            bound.manager.version(),
            (),
        ) else {
            continue;
        };
        bound.manager.workspace_group(&handle);
        handle.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());
        for object in shown_as.client_outputs(&client) {
            handle.output_enter(&object);
        }
        bound.groups.push(Group {
            output: *output,
            handle,
        });
        changed = true;
    }
    for fact in facts {
        let group_of = |output: MonitorHandle| {
            bound
                .groups
                .iter()
                .find(|group| group.output == output)
                .map(|group| group.handle.clone())
        };
        match bound
            .entries
            .iter_mut()
            .find(|entry| entry.told.id == fact.id)
        {
            Some(entry) => {
                if entry.told == *fact {
                    continue;
                }
                if entry.told.title != fact.title {
                    entry.handle.name(fact.title.clone());
                }
                if entry.told.index != fact.index {
                    entry
                        .handle
                        .coordinates((fact.index as u32).to_ne_bytes().to_vec());
                }
                if entry.told.output != fact.output {
                    if let Some(group) = group_of(entry.told.output) {
                        group.workspace_leave(&entry.handle);
                    }
                    if let Some(group) = group_of(fact.output) {
                        group.workspace_enter(&entry.handle);
                    }
                }
                if entry.told.active != fact.active {
                    entry.handle.state(state_of(fact));
                }
                entry.told = fact.clone();
            }
            None => {
                let Ok(handle) = client.create_resource::<ExtWorkspaceHandleV1, _, State>(
                    display,
                    bound.manager.version(),
                    fact.id,
                ) else {
                    continue;
                };
                bound.manager.workspace(&handle);
                handle.id(fact.lasting_id());
                handle.name(fact.title.clone());
                handle.coordinates((fact.index as u32).to_ne_bytes().to_vec());
                handle.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);
                handle.state(state_of(fact));
                if let Some(group) = group_of(fact.output) {
                    group.workspace_enter(&handle);
                }
                bound.entries.push(Entry {
                    handle,
                    told: fact.clone(),
                });
            }
        }
        changed = true;
    }
    changed
}

impl State {
    /// What the panels are told of the workspaces now: every display's,
    /// in the displays' order and each display's own.
    pub(crate) fn workspace_facts(&self) -> Vec<Facts> {
        let manager = &self.core.monitor_manager;
        let mut facts = Vec::new();
        for (output, _) in &self.outputs {
            let (Some(monitor), Some(workspaces)) = (
                manager.get_monitor(*output),
                manager.get_workspaces(*output),
            ) else {
                continue;
            };
            let shown = workspaces.focused_idx();
            for (index, workspace) in workspaces.iter().enumerate() {
                facts.push(Facts {
                    id: workspace.id,
                    name: workspace.name.clone(),
                    title: workspace.label().to_string(),
                    output: *output,
                    output_name: monitor.name.clone(),
                    index,
                    active: index == shown,
                });
            }
        }
        facts
    }

    /// Tell the panels what changed about the workspaces, if anything did.
    pub(crate) fn refresh_workspace_management(&mut self) {
        if self.workspace_management.bound.is_empty() {
            return;
        }
        let facts = self.workspace_facts();
        let display = self.display.clone();
        let mut management = std::mem::take(&mut self.workspace_management);
        management.bound.retain(|bound| bound.manager.is_alive());
        for bound in &mut management.bound {
            if tell(&display, &self.outputs, bound, &facts) {
                bound.manager.done();
            }
        }
        self.workspace_management = management;
    }
}

impl GlobalDispatch<ExtWorkspaceManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        let mut bound = Bound {
            manager,
            groups: Vec::new(),
            entries: Vec::new(),
            asked: Vec::new(),
        };
        tell(
            display,
            &state.outputs,
            &mut bound,
            &state.workspace_facts(),
        );
        bound.manager.done();
        state.workspace_management.bound.push(bound);
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        _: &(),
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        match request {
            // What was asked since the last commit is done now, in the
            // order it was asked.
            ext_workspace_manager_v1::Request::Commit => {
                let asked = state
                    .workspace_management
                    .bound
                    .iter_mut()
                    .find(|bound| bound.manager == *manager)
                    .map(|bound| std::mem::take(&mut bound.asked))
                    .unwrap_or_default();
                let facts = state.workspace_facts();
                for id in asked {
                    let Some(fact) = facts.iter().find(|fact| fact.id == id) else {
                        continue;
                    };
                    let command = DispatchCommand::FocusMonitorWorkspace {
                        target: MonitorTarget::Name(fact.output_name.clone()),
                        workspace: WorkspaceSelector::Index(fact.index),
                    };
                    if let Err(error) = state.dispatch(command) {
                        log::warn!("a panel asked for workspace {}: {error}", fact.name);
                    }
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                manager.finished();
                state
                    .workspace_management
                    .bound
                    .retain(|bound| bound.manager != *manager);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn request(
        _: &mut Self,
        _client: &Client,
        _: &ExtWorkspaceGroupHandleV1,
        _: ext_workspace_group_handle_v1::Request,
        _: &(),
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        // (Making a workspace is not among what a group can do here.)
    }
}

impl Dispatch<ExtWorkspaceHandleV1, usize> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        handle: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        id: &usize,
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        // Only showing one is offered; it waits for the panel's commit.
        if let ext_workspace_handle_v1::Request::Activate = request {
            if let Some(bound) = state
                .workspace_management
                .bound
                .iter_mut()
                .find(|bound| bound.entries.iter().any(|entry| entry.handle == *handle))
            {
                bound.asked.push(*id);
            }
        }
    }
}
