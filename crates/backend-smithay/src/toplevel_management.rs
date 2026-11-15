//! Other panels' taskbars (wlr-foreign-toplevel-management): `waybar`'s
//! taskbar, `nwg-dock`, `sfwbar` and the like are told of every window
//! (its title, its program, the display it is on, whether it has the
//! keyboard, is minimized, maximized or fullscreen) and can ask for a
//! window to be gone to, closed, minimized, maximized or made fullscreen.
//!
//! What such a panel asks is carried out as the same asked by a key is:
//! going to a window is the command `focus_window` (so its workspace is
//! shown), closing it `close_window`. A tiled window is not maximized out
//! of its layout, as it is not when it asks itself.

use crate::state::State;
use gevrek_engine::contract::StateChange;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::{Desktop, WindowHandle};
use gevrek_model::command::{DispatchCommand, WindowSelector};
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

const VERSION: u32 = 3;

/// What a panel is told of one window.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Facts {
    window: WindowHandle,
    title: String,
    app_id: String,
    output: Option<MonitorHandle>,
    maximized: bool,
    minimized: bool,
    activated: bool,
    fullscreen: bool,
}

impl Facts {
    /// The states as the protocol lists them: the numbers of those that
    /// hold, four bytes each.
    fn states(&self) -> Vec<u8> {
        use zwlr_foreign_toplevel_handle_v1::State as Told;
        [
            (self.maximized, Told::Maximized),
            (self.minimized, Told::Minimized),
            (self.activated, Told::Activated),
            (self.fullscreen, Told::Fullscreen),
        ]
        .into_iter()
        .filter(|(holds, _)| *holds)
        .flat_map(|(_, state)| (state as u32).to_ne_bytes())
        .collect()
    }
}

struct Entry {
    handle: ZwlrForeignToplevelHandleV1,
    /// What the panel knows of the window; nothing yet when it is new.
    told: Option<Facts>,
}

struct Bound {
    manager: ZwlrForeignToplevelManagerV1,
    entries: Vec<(WindowHandle, Entry)>,
}

/// The panels that are connected, and what they were told.
#[derive(Default)]
pub struct ToplevelManagement {
    bound: Vec<Bound>,
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ZwlrForeignToplevelManagerV1, ()>(VERSION, ());
}

/// Tell one panel what it does not know yet.
fn tell(
    display: &DisplayHandle,
    outputs: &[(MonitorHandle, smithay::output::Output)],
    bound: &mut Bound,
    facts: &[Facts],
) {
    let Some(client) = bound.manager.client() else {
        return;
    };
    // Windows that are gone.
    bound.entries.retain(|(window, entry)| {
        let there = facts.iter().any(|fact| fact.window == *window);
        if !there {
            entry.handle.closed();
        }
        there
    });
    for fact in facts {
        let at = match bound
            .entries
            .iter()
            .position(|(window, _)| *window == fact.window)
        {
            Some(at) => at,
            None => {
                let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
                    display,
                    bound.manager.version(),
                    fact.window,
                ) else {
                    continue;
                };
                bound.manager.toplevel(&handle);
                bound
                    .entries
                    .push((fact.window, Entry { handle, told: None }));
                bound.entries.len() - 1
            }
        };
        let entry = &mut bound.entries[at].1;
        if entry.told.as_ref() == Some(fact) {
            continue;
        }
        let told = entry.told.as_ref();
        if told.map(|told| &told.title) != Some(&fact.title) {
            entry.handle.title(fact.title.clone());
        }
        if told.map(|told| &told.app_id) != Some(&fact.app_id) {
            entry.handle.app_id(fact.app_id.clone());
        }
        let was_on = told.and_then(|told| told.output);
        if was_on != fact.output {
            let of = |id: Option<MonitorHandle>| -> Vec<_> {
                outputs
                    .iter()
                    .filter(|(known, _)| Some(*known) == id)
                    .flat_map(|(_, output)| output.client_outputs(&client))
                    .collect()
            };
            for output in of(was_on) {
                entry.handle.output_leave(&output);
            }
            for output in of(fact.output) {
                entry.handle.output_enter(&output);
            }
        }
        if told.map(Facts::states) != Some(fact.states()) {
            entry.handle.state(fact.states());
        }
        entry.handle.done();
        entry.told = Some(fact.clone());
    }
}

impl State {
    /// What the panels are told of the windows now: those that are shown
    /// or could be (a window that has drawn itself), in the order they
    /// came.
    pub(crate) fn toplevel_facts(&self) -> Vec<Facts> {
        let mut facts: Vec<Facts> = self
            .desktop
            .windows
            .iter()
            .filter(|(_, window)| window.alive && window.mapped)
            .map(|(id, window)| {
                let centre = (
                    window.rect.x + window.rect.width / 2,
                    window.rect.y + window.rect.height / 2,
                );
                Facts {
                    window: *id,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    output: self
                        .desktop
                        .outputs
                        .iter()
                        .find(|output| output.rect.contains_point(centre.0, centre.1))
                        .map(|output| output.id),
                    maximized: window.maximized,
                    minimized: window.minimized,
                    activated: self.desktop.focused == Some(*id),
                    fullscreen: window.fullscreen,
                }
            })
            .collect();
        facts.sort_by_key(|fact| fact.window.0);
        facts
    }

    /// Tell the panels what changed about the windows, if anything did.
    pub(crate) fn refresh_toplevel_management(&mut self) {
        if self.toplevel_management.bound.is_empty() {
            return;
        }
        let facts = self.toplevel_facts();
        let display = self.display.clone();
        let mut management = std::mem::take(&mut self.toplevel_management);
        management.bound.retain(|bound| bound.manager.is_alive());
        for bound in &mut management.bound {
            tell(&display, &self.outputs, bound, &facts);
        }
        self.toplevel_management = management;
    }

    /// A panel asks for a state of a window.
    fn panel_state_request(&mut self, id: WindowHandle, change: StateChange) {
        if !self.desktop.windows.contains_key(&id) {
            return;
        }
        // (See `client_state_request`: a tiled window keeps its tile.)
        let manager = &self.core.monitor_manager;
        let tiled = manager.find_window(id).is_some() && !manager.is_window_floating(id);
        if tiled && change == StateChange::Maximized(true) {
            return;
        }
        if let Err(error) = self
            .desktop
            .apply(&gevrek_engine::Effect::State { id, change })
        {
            log::warn!("a panel asked for {change:?}: {error}");
        }
        self.window_event(WindowEvent::MetadataChanged(id));
        if let Some(monitor) = self.desktop.output_of(id) {
            self.core.executor.mark_layout_dirty(monitor);
        }
        self.settle();
    }

    /// A panel asks for a command on a window.
    fn panel_command(&mut self, command: DispatchCommand) {
        if let Err(error) = self.dispatch(command) {
            log::warn!("what a panel asked of a window was not done: {error}");
        }
        self.settle();
    }
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        let mut bound = Bound {
            manager,
            entries: Vec::new(),
        };
        tell(display, &state.outputs, &mut bound, &state.toplevel_facts());
        state.toplevel_management.bound.push(bound);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _: &(),
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            manager.finished();
            state
                .toplevel_management
                .bound
                .retain(|bound| bound.manager != *manager);
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, WindowHandle> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        window: &WindowHandle,
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;
        let id = *window;
        let selector = WindowSelector::Handle(id.0 as i64);
        match request {
            Request::SetMaximized => state.panel_state_request(id, StateChange::Maximized(true)),
            Request::UnsetMaximized => state.panel_state_request(id, StateChange::Maximized(false)),
            Request::SetMinimized => state.panel_state_request(id, StateChange::Minimized(true)),
            Request::UnsetMinimized => state.panel_state_request(id, StateChange::Minimized(false)),
            Request::SetFullscreen { .. } => {
                state.panel_state_request(id, StateChange::Fullscreen(true))
            }
            Request::UnsetFullscreen => {
                state.panel_state_request(id, StateChange::Fullscreen(false))
            }
            Request::Activate { .. } => {
                // One that is minimized is brought back first.
                if state
                    .desktop
                    .windows
                    .get(&id)
                    .is_some_and(|window| window.minimized)
                {
                    state.panel_state_request(id, StateChange::Minimized(false));
                }
                state.panel_command(DispatchCommand::FocusWindow { selector });
            }
            Request::Close => state.panel_command(DispatchCommand::CloseWindow { selector }),
            // Where the panel shows the window (for an animation of
            // minimizing towards it): gevrek has none such.
            Request::SetRectangle { .. } => {}
            Request::Destroy => {
                for bound in &mut state.toplevel_management.bound {
                    bound.entries.retain(|(_, entry)| entry.handle.is_alive());
                }
            }
            _ => {}
        }
    }
}
