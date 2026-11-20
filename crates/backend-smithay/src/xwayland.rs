//! X11 programs, through Xwayland: gevrek is their window manager. An X11
//! window is a window like any other for the engine; its class and its
//! program are known, so rules written for Windows classes keep working.

use crate::state::State;
use crate::surface::WindowSurface;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::Desktop;
use smithay::reexports::calloop::LoopHandle;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata,
    request_data_device_client_selection, set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
    set_primary_selection,
};
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::{Reorder, ResizeEdge, WmWindowProperty, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler};
use std::process::Stdio;

/// An X11 program's drag, as far as the compositor has to do with it
/// (`xdnd.rs`, the second half).
#[derive(Default)]
pub struct XDrag {
    /// The way to the thread that is the X11 side of it.
    to_x: Option<std::sync::mpsc::Sender<crate::xdnd::ToX>>,
    /// An X11 program drags something now.
    dragging: bool,
    /// The unseen window that takes drags stands on the X screen.
    standing: bool,
    /// gevrek's own drag on the Wayland side is going on, with these
    /// kinds of data (each with the name it is fetched by).
    grabbing: bool,
    kinds: Vec<(String, String)>,
    /// That drag is being ended by gevrek itself (the pointer went over
    /// an X11 window): not a let-go.
    leaving: bool,
    /// The surface the X11 program's press was on, with its place: the
    /// button's release is handed back there.
    press: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        smithay::utils::Point<f64, Logical>,
    )>,
    /// The button was let go while gevrek's drag held the pointer: the
    /// X11 program is still to be told.
    release_owed: bool,
    /// What the X11 program asks its drag to do, as gevrek's own drag
    /// has it: the actions allowed, and the one asked for as the keys
    /// would ask (`xdnd::wayland_actions`).
    allowed: Option<smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction>,
    wish: Option<smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction>,
}

impl State {
    /// Watch the X server for drags of its programs (`xdnd::watch`).
    fn watch_x11_drags(&mut self, display: u32) {
        let Some(handle) = self.loop_handle.clone() else {
            return;
        };
        let (tell, told) = smithay::reexports::calloop::channel::channel::<crate::xdnd::FromX>();
        let inserted = handle.insert_source(told, |event, _, state| {
            if let smithay::reexports::calloop::channel::Event::Msg(what) = event {
                state.x_drag_told(what);
            }
        });
        if inserted.is_err() {
            return;
        }
        self.x_drag = XDrag {
            to_x: crate::xdnd::watch(display, move |what| {
                let _ = tell.send(what);
            }),
            ..XDrag::default()
        };
    }

    fn x_drag_say(&self, what: crate::xdnd::ToX) {
        if let Some(to_x) = &self.x_drag.to_x {
            let _ = to_x.send(what);
        }
    }

    /// The X11 side says what an X11 program's drag does.
    fn x_drag_told(&mut self, what: crate::xdnd::FromX) {
        use crate::xdnd::FromX;
        match what {
            // (Not gevrek's own standing in for a Wayland program's drag.)
            FromX::Dragging(now) => {
                // (Another drag: nothing of the one before it counts.)
                if now && !self.x_drag.grabbing {
                    self.x_drag.press = None;
                    self.x_drag.allowed = None;
                    self.x_drag.wish = None;
                }
                self.x_drag.dragging = now && self.dragged.is_none() && self.xdnd_out.is_none();
                if !self.x_drag.dragging && self.x_drag.standing && !self.x_drag.grabbing {
                    self.x_drag.standing = false;
                    self.x_drag_say(crate::xdnd::ToX::Stand(false));
                }
            }
            FromX::Entered(names, asked, listed) => {
                let kinds = crate::xdnd::offered(&names);
                if !self.x_drag.dragging || self.x_drag.grabbing || kinds.is_empty() {
                    return;
                }
                let (allowed, wish) = crate::xdnd::wayland_actions(asked, &listed);
                self.x_drag.kinds = kinds;
                self.x_drag.allowed = Some(allowed);
                self.x_drag.wish = wish;
                self.x_drag_begin_own();
                log::info!("An X11 program's drag is carried over to the Wayland windows");
            }
            // A key went down or up in the X11 program: its drag is to
            // do something else. gevrek's own drag says what it allows
            // once, when it begins; so it ends and begins anew, which the
            // window under the pointer sees as the drag leaving and
            // coming back with other actions.
            FromX::Asks(asked, listed) => {
                let (allowed, wish) = crate::xdnd::wayland_actions(asked, &listed);
                if !self.x_drag.grabbing
                    || (self.x_drag.allowed, self.x_drag.wish) == (Some(allowed), wish)
                {
                    return;
                }
                self.x_drag_end_own();
                self.x_drag.allowed = Some(allowed);
                self.x_drag.wish = wish;
                self.x_drag_begin_own();
            }
            FromX::Left => self.x_drag_end_own(),
        }
    }

    /// Begin gevrek's own drag on the Wayland side for the X11 program's:
    /// with the kinds of data and the actions last carried over, where
    /// the X11 program's press was.
    fn x_drag_begin_own(&mut self) {
        use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let start =
            pointer
                .grab_start_data()
                .unwrap_or_else(|| smithay::input::pointer::GrabStartData {
                    focus: None,
                    button: 0x110,
                    location: pointer.current_location(),
                });
        // (The press is known while its own hold on the pointer lasts:
        // the first time of a drag. Begun again, the drag has it still.)
        if start.focus.is_some() || self.x_drag.press.is_none() {
            self.x_drag.press = start.focus.clone();
        }
        self.x_drag.grabbing = true;
        self.x_drag.leaving = false;
        let kinds = self.x_drag.kinds.clone();
        let (display, seat) = (self.display.clone(), self.seat.clone());
        smithay::wayland::selection::data_device::start_dnd(
            &display,
            &seat,
            self,
            smithay::utils::SERIAL_COUNTER.next_serial(),
            Some(start),
            None,
            smithay::wayland::selection::data_device::SourceMetadata {
                mime_types: kinds.into_iter().map(|(kind, _)| kind).collect(),
                dnd_action: self.x_drag.allowed.unwrap_or(DndAction::Copy),
            },
        );
        // (What is under the pointer now is told of it at once.)
        let (x, y) = self.pointer_exact;
        self.pointer_moved((x, y).into(), now_ms());
    }

    /// Whether the drag that is on is gevrek's own, for an X11 program's.
    pub(crate) fn x_drag_is_own(&self) -> bool {
        self.x_drag.grabbing
    }

    /// What the X11 program asks its drag to do, as a Wayland action.
    pub(crate) fn x_drag_wish(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction>
    {
        self.x_drag.wish
    }

    /// The Wayland side settled on what a drop would do: the X11 program
    /// is told with its drag's next place, and when the drop is done.
    pub(crate) fn x_drag_chose(
        &mut self,
        chosen: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    ) {
        self.x_drag_say(crate::xdnd::ToX::Chose(crate::xdnd::taken_of(chosen)));
    }

    /// End gevrek's own drag without a drop: the X11 program's drag goes
    /// on among X11 windows.
    fn x_drag_end_own(&mut self) {
        if !self.x_drag.grabbing {
            return;
        }
        self.x_drag.leaving = true;
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, smithay::utils::SERIAL_COUNTER.next_serial(), now_ms());
        }
        self.x_drag.grabbing = false;
        self.x_drag.leaving = false;
    }

    /// An X11 program's drag and the pointer: over something that is not
    /// X11's, the unseen window that takes drags stands on the X screen;
    /// over an X11 window it does not, and gevrek's own drag ends.
    pub(crate) fn follow_x11_drag(&mut self) {
        if !self.x_drag.dragging {
            return;
        }
        // (What is topmost there, as the pointer finds it: under a panel,
        // a launcher or zones raised over the windows, an X11 window is
        // not what the drag is over.)
        let over_x11 = self.x11_window_under_pointer().is_some();
        if over_x11 == self.x_drag.standing {
            self.x_drag.standing = !over_x11;
            self.x_drag_say(crate::xdnd::ToX::Stand(!over_x11));
        }
        if over_x11 {
            self.x_drag_end_own();
        }
    }

    /// A Wayland window asks for the dragged data in a kind.
    pub(crate) fn x_drag_fetch(&mut self, kind: String, pipe: std::os::fd::OwnedFd) {
        let by = self
            .x_drag
            .kinds
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, by)| by.clone());
        if let Some(by) = by {
            self.x_drag_say(crate::xdnd::ToX::Fetch(by, pipe));
        }
    }

    /// gevrek's own drag ended with the button let go (`done`: said at
    /// once whether it was taken, where that is known already).
    pub(crate) fn x_drag_let_go(&mut self, done: Option<bool>) {
        if self.x_drag.leaving {
            return;
        }
        self.x_drag.grabbing = false;
        self.x_drag.release_owed = true;
        if let Some(taken) = done {
            self.x_drag_done(taken);
        }
    }

    pub(crate) fn x_drag_done(&mut self, taken: bool) {
        self.x_drag_say(crate::xdnd::ToX::Done(taken));
    }

    /// The X11 program that drags is told that the button was let go:
    /// the release goes to the surface its press was on. (Called when
    /// the button's own handling is over.)
    pub(crate) fn x_drag_release_owed(&mut self) {
        if !std::mem::take(&mut self.x_drag.release_owed) {
            return;
        }
        use smithay::backend::input::ButtonState;
        use smithay::input::pointer::{ButtonEvent, MotionEvent};
        let (Some(pointer), Some(press)) = (self.seat.get_pointer(), self.x_drag.press.take())
        else {
            return;
        };
        let (x, y) = self.pointer_exact;
        let time = now_ms();
        pointer.motion(
            self,
            Some(press),
            &MotionEvent {
                location: (x, y).into(),
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.button(
            self,
            &ButtonEvent {
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time,
                button: 0x110,
                state: ButtonState::Released,
            },
        );
        pointer.frame(self);
        // (And the pointer is where it is again.)
        self.pointer_moved((x, y).into(), time);
    }

    /// Start Xwayland. X11 programs can connect once it reports ready; the
    /// display they use is put in `DISPLAY` for programs started from here.
    pub fn start_xwayland(&mut self, handle: LoopHandle<'static, State>) -> anyhow::Result<()> {
        let (xwayland, client) = XWayland::spawn(
            &self.display,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        )?;
        log::debug!("Xwayland is started; X11 programs can connect once it is ready");
        self.loop_handle = Some(handle.clone());
        let wm_handle = handle.clone();
        handle
            .insert_source(xwayland, move |event, _, state| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => match X11Wm::start_wm(wm_handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        state.xwm = Some(wm);
                        state.x_display = Some(display_number);
                        state.x_client = Some(client.clone());
                        state.watch_x11_drags(display_number);
                        state.x_scale = 1.0;
                        state.tell_x11_scale();
                        std::env::set_var("DISPLAY", format!(":{display_number}"));
                        // The X11 display is known only now.
                        if state.session_mode {
                            crate::session_env::export(&state.config.desktop.session);
                        }
                        log::info!("Xwayland is ready on display :{display_number}");
                    }
                    Err(error) => log::error!("Could not manage X11 windows: {error}"),
                },
                XWaylandEvent::Error => log::error!("Xwayland did not start"),
            })
            .map_err(|error| anyhow::anyhow!("Xwayland source: {error}"))?;
        Ok(())
    }

    /// X11 programs draw at the display's own pixels: Xwayland is told
    /// the scale of the display with the largest one (its screen is then
    /// as many pixels as the displays have, where it would be the logical
    /// size, drawn small and scaled up), and the programs are told how
    /// fine that is where they look for it (`Xft.dpi`, and the pointer's
    /// size). On a display of a smaller scale their picture is scaled
    /// down. Called when Xwayland is ready and when a scale changes.
    pub(crate) fn tell_x11_scale(&mut self) {
        let (Some(client), Some(display)) = (&self.x_client, self.x_display) else {
            return;
        };
        let scale = self
            .desktop
            .outputs
            .iter()
            .map(|output| output.scale)
            .fold(1.0, f64::max);
        if (scale - self.x_scale).abs() < 0.001 {
            return;
        }
        let Some(data) = client.get_data::<smithay::xwayland::XWaylandClientData>() else {
            return;
        };
        data.compositor_state.set_client_scale(scale);
        self.x_scale = scale;
        // (The displays are told again: to Xwayland they are other sizes
        // now, which the library sends when it finds the scale changed.)
        for (_, output) in &self.outputs {
            output.change_current_state(None, None, None, None);
        }
        log::info!("X11 programs draw at {scale} times the logical size");
        // The X server is told on a line of its own (it may be waiting
        // for this one).
        let spawned = std::thread::Builder::new()
            .name("gevrek-x11-scale".to_string())
            .spawn(move || {
                if write_x11_resources(display, scale).is_none() {
                    log::warn!("X11 programs were not told how fine the display is");
                }
            });
        if let Err(error) = spawned {
            log::warn!("X11 programs were not told how fine the display is: {error}");
        }
        self.desktop.needs_redraw = true;
    }

    /// A Wayland program's drag moved: over an X11 window it is told to
    /// that window as X11 programs tell drags (`xdnd.rs`). Called with
    /// every move of the pointer.
    pub(crate) fn follow_drag_onto_x11(&mut self) {
        let Some(source) = self.dragged.clone() else {
            return;
        };
        let Some(display) = self.x_display else {
            return;
        };
        let (x, y) = self.desktop.pointer;
        // The X11 window the drag is over, where one is topmost there: a
        // surface of a shell above it has the drag by itself (the
        // library's drag tells it), and the window under it is told that
        // the drag left.
        let under = self.x11_window_under_pointer();
        match (under, self.xdnd_out.is_some()) {
            (Some(window), begun) => {
                if !begun {
                    // The first X11 window of this drag: its data is
                    // read from the program now, in the kinds it offers.
                    let kinds = smithay::wayland::selection::data_device::with_source_metadata(
                        &source,
                        |about| about.mime_types.clone(),
                    )
                    .unwrap_or_default();
                    let mut pipes = Vec::new();
                    for kind in kinds.into_iter().take(crate::xdnd::MOST_KINDS) {
                        let mut ends = [0; 2];
                        // SAFETY: an array of two for the two ends, which
                        // are owned by nothing else when the call returns.
                        if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                            continue;
                        }
                        use std::os::fd::{AsFd, FromRawFd, OwnedFd};
                        // SAFETY: just made, valid, and owned here alone.
                        let (read, write) = unsafe {
                            (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1]))
                        };
                        source.send(kind.clone(), write.as_fd());
                        pipes.push((kind, read));
                    }
                    self.xdnd_out = crate::xdnd::Out::begin(display, pipes);
                }
                if let Some(out) = self.xdnd_out.as_mut() {
                    let at = (
                        (f64::from(x) * self.x_scale).round() as i32,
                        (f64::from(y) * self.x_scale).round() as i32,
                    );
                    out.tell(crate::xdnd::Told::Over { window, at });
                }
            }
            (None, true) => {
                if let Some(out) = self.xdnd_out.as_mut() {
                    if out.over.is_some() {
                        out.tell(crate::xdnd::Told::Away);
                    }
                }
            }
            (None, false) => {}
        }
    }

    /// Whether a fullscreen X11 window that is shown asked never to be
    /// handed to the display past the compositor: no window's own
    /// picture goes to the display's plane then.
    pub(crate) fn x11_keeps_composited(&self) -> bool {
        !self.x11_composited.is_empty()
            && self.desktop.windows.values().any(|record| {
                record.fullscreen
                    && record.mapped
                    && matches!(&record.surface, WindowSurface::X11(window)
                        if self.x11_composited.contains(&window.window_id()))
            })
    }

    /// Whether an X11 window that places itself (a menu, a tooltip, a
    /// notification that stays up for minutes) is in sight: it goes with
    /// the workspace of its program's windows and comes back with it.
    /// The window it says it belongs to decides, where that is a window
    /// of a workspace; else any window of its program whose workspace is
    /// shown. One of a program with no window at all is shown. (A window
    /// that is minimized is not out of sight in this sense: what its
    /// program puts up meanwhile is seen.)
    pub(crate) fn x11_own_shown(&self, window: &X11Surface) -> bool {
        let managed = |x11_window: u32| {
            self.desktop.windows.values().find(|record| {
                matches!(&record.surface, WindowSurface::X11(other)
                    if record.alive && other.window_id() == x11_window)
            })
        };
        // (A submenu belongs to its menu, that to the window.)
        let mut link = window.is_transient_for();
        for _ in 0..8 {
            let Some(above) = link else {
                break;
            };
            if let Some(record) = managed(above) {
                return !record.hidden_by_wm;
            }
            link = self
                .x11_overrides
                .iter()
                .find(|other| other.window_id() == above)
                .and_then(|other| other.is_transient_for());
        }
        let of_x11_programs: Vec<(Option<u32>, bool)> = self
            .desktop
            .windows
            .values()
            .filter(|record| {
                record.alive && record.mapped && matches!(record.surface, WindowSurface::X11(_))
            })
            .map(|record| (record.pid, !record.hidden_by_wm))
            .collect();
        in_sight_with_its_program(window.pid(), &of_x11_programs)
    }

    /// Whether what is topmost at a place is an X11 window that places
    /// itself: a press there is its program's alone, and no window under
    /// it is given the keyboard for it.
    pub(crate) fn x11_own_at(&self, location: smithay::utils::Point<f64, Logical>) -> bool {
        self.x11_overrides.iter().any(|window| {
            window.geometry().to_f64().contains(location)
                && window.wl_surface().is_some()
                && self.x11_own_shown(window)
        })
    }

    /// Where a new window that floats starts, where that is not the
    /// middle of the display the pointer is on: an X11 window where its
    /// program put it (it says so in its size hints: a tool's window, a
    /// dialog, a splash screen, a window that was hidden and comes back),
    /// any other dialog in the middle of the window it belongs to. Not a
    /// place from which too little of it would be on a display to take
    /// hold of.
    pub(crate) fn own_starting_place(&self, id: gevrek_engine::WindowHandle) -> Option<(i32, i32)> {
        use gevrek_engine::WindowQueries;
        let record = self.desktop.windows.get(&id)?;
        if !self.desktop.floats_by_rules(id) {
            return None;
        }
        let (width, height) = (record.rect.width, record.rect.height);
        let said = match &record.surface {
            WindowSurface::X11(window) => window
                .size_hints()
                .is_some_and(|hints| hints.position.is_some())
                .then(|| window.geometry().loc)
                .map(|at| (at.x, at.y)),
            WindowSurface::Xdg(_) => None,
        };
        let over_its_window = || {
            let above = self.desktop.windows.get(&self.desktop.parent_of(id)?)?;
            above.mapped.then(|| {
                (
                    above.rect.x + (above.rect.width - width) / 2,
                    above.rect.y + (above.rect.height - height) / 2,
                )
            })
        };
        let displays: Vec<gevrek_engine::Rect> = self
            .desktop
            .outputs
            .iter()
            .map(|output| output.rect)
            .collect();
        said.into_iter().chain(over_its_window()).find(|(x, y)| {
            reaches_a_display(gevrek_engine::Rect::new(*x, *y, width, height), &displays)
        })
    }

    /// An X11 window went away (unmapped or destroyed).
    fn x11_window_gone(&mut self, window: &X11Surface) {
        self.x11_composited.remove(&window.window_id());
        self.x11_overrides.retain(|other| other != window);
        self.desktop.needs_redraw = true;
        let Some(id) = self.desktop.id_of(&WindowSurface::X11(window.clone())) else {
            return;
        };
        self.window_gone(id);
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// Xwayland paired an X11 window with its surface. The surface may have
    /// been given its first buffer before this: then no later commit would
    /// tell us, and the window would never be adopted. Look at it now.
    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        _wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        surface: X11Surface,
    ) {
        let id = self.desktop.id_of(&WindowSurface::X11(surface));
        log::trace!("an X11 window has its surface: {id:?}");
        if let Some(id) = id {
            self.toplevel_committed(id);
        }
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("X11 events only come from a started WM")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// An X11 window wants to be shown: it is tracked like a Wayland
    /// toplevel, and adopted when its first buffer arrives.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        log::debug!(
            "an X11 window asks to be shown ({:?}, {:?})",
            window.class(),
            window.title()
        );
        // What it wrote on itself before it is shown
        // (`_NET_WM_STATE_FULLSCREEN`: a game that starts fullscreen says
        // it so, and sends no request). Read now: once the window is
        // mapped, that property is the window manager's to write. The X
        // server is asked on a line of its own and waited for a moment
        // only, never longer.
        let (mut places_itself, mut kept_above) = (false, false);
        if let Some(display) = self.x_display {
            let x11_window = window.window_id();
            let (answer, heard) = std::sync::mpsc::channel();
            let asked = std::thread::Builder::new()
                .name("gevrek-x11-state".to_string())
                .spawn(move || {
                    let _ = answer.send(said_of_itself(display, x11_window).unwrap_or_default());
                });
            let said = asked.ok().and_then(|_| {
                heard
                    .recv_timeout(std::time::Duration::from_millis(150))
                    .ok()
            });
            // (Not heard in time: what the library read of its kind.)
            places_itself = match &said {
                Some(said) => said.kind.as_deref().is_some_and(kind_places_itself),
                None => {
                    use smithay::xwayland::xwm::WmWindowType;
                    matches!(
                        window.window_type(),
                        Some(
                            WmWindowType::Tooltip
                                | WmWindowType::Notification
                                | WmWindowType::PopupMenu
                                | WmWindowType::DropdownMenu
                        )
                    )
                }
            };
            let Said {
                fullscreen,
                bypass,
                above,
                ..
            } = said.unwrap_or_default();
            kept_above = above;
            if fullscreen && !places_itself {
                self.x11_start_fullscreen.push(x11_window);
            }
            // `_NET_WM_BYPASS_COMPOSITOR` 2: its picture is never to go
            // to the display past the compositor (1, that it may, is
            // what a fullscreen window gets anyway).
            if never_past_the_compositor(bypass) && !places_itself {
                log::info!(
                    "An X11 window asks never to be handed to the display past the compositor"
                );
                self.x11_composited.insert(x11_window);
            } else {
                self.x11_composited.remove(&x11_window);
            }
        }
        if let Err(error) = window.set_mapped(true) {
            log::warn!("X11 map: {error}");
            return;
        }
        // A tooltip, a notification, a menu, what is being dragged: its
        // program's own, as if it had said that no window manager is to
        // touch it (some programs leave that out for these). It is no
        // window of a workspace: it is shown where it put itself, is
        // given no border, no tile and never the keyboard.
        if places_itself {
            log::debug!("an X11 window of a kind that places itself is left to its program");
            self.x11_overrides.retain(|other| *other != window);
            self.x11_overrides.push(window);
            self.desktop.needs_redraw = true;
            return;
        }
        let pid = window.pid();
        let geometry = window.geometry();
        let id = self.desktop.track(WindowSurface::X11(window), pid);
        log::debug!(
            "the X11 window is the window {} (shown once it draws)",
            id.0
        );
        if let Some(record) = self.desktop.windows.get_mut(&id) {
            record.rect.width = geometry.size.w;
            record.rect.height = geometry.size.h;
            // (`_NET_WM_STATE_ABOVE` written before it is shown: it is
            // kept above the others from the start.)
            record.always_on_top = kept_above;
        }
    }

    /// Menus, tooltips and the like: they place themselves and are drawn
    /// above everything, without being managed.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // (A tooltip at each hover: not worth a line at debug.)
        log::trace!("an X11 menu or tooltip is shown ({:?})", window.class());
        self.x11_overrides.retain(|other| *other != window);
        self.x11_overrides.push(window);
        self.desktop.needs_redraw = true;
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if window.is_override_redirect() {
            log::trace!("an X11 menu or tooltip is hidden ({:?})", window.class());
        } else {
            log::debug!(
                "an X11 window is hidden by its program ({:?})",
                window.class()
            );
        }
        self.x11_window_gone(&window);
        if !window.is_override_redirect() {
            if let Err(error) = window.set_mapped(false) {
                log::debug!("an X11 window was not marked unmapped: {error}");
            }
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if !window.is_override_redirect() {
            log::debug!("an X11 window is destroyed ({:?})", window.class());
        }
        self.x11_window_gone(&window);
    }

    /// The client asks for a place or a size. A window that has a tile is
    /// told again where it is; a floating one goes where its program puts
    /// it (a tool's window its program moves or makes larger by itself),
    /// and one gevrek does not know yet, or leaves to its program, may
    /// choose.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // A window that is shown asks to be as large as a display, and
        // has no frame: it goes fullscreen in the older way.
        if let (Some(w), Some(h)) = (w, h) {
            let shown_and_not_fullscreen = self
                .desktop
                .id_of(&WindowSurface::X11(window.clone()))
                .and_then(|id| self.desktop.windows.get(&id))
                .is_some_and(|record| record.mapped && !record.fullscreen);
            if shown_and_not_fullscreen && self.x11_covers_a_display(&window, w as i32, h as i32) {
                log::info!("An X11 window asks for a display's size ({w}x{h}): fullscreen");
                self.x11_state_request(
                    &window,
                    gevrek_engine::contract::StateChange::Fullscreen(true),
                );
                return;
            }
        }
        // A fullscreen window asks for the size of a resolution its
        // program asked Xwayland for (a game that "changes the
        // resolution"): it has that size to itself, and Xwayland shows
        // it over the display. Asked for the display's own size again,
        // that is over.
        if let (Some(w), Some(h), Some(display)) = (w, h, self.x_display) {
            let fullscreen = self
                .desktop
                .id_of(&WindowSurface::X11(window.clone()))
                .and_then(|id| self.desktop.windows.get(&id))
                .filter(|record| record.mapped && record.fullscreen)
                .map(|record| record.rect);
            if let Some(rect) = fullscreen {
                let asked = (w as i32, h as i32);
                if asked == (rect.width, rect.height) {
                    crate::surface::Emulated::set(&window, None);
                } else {
                    let (x11_window, scale) = (window.window_id(), self.x_scale);
                    let (answer, heard) = std::sync::mpsc::channel();
                    let read = std::thread::Builder::new()
                        .name("gevrek-x11-modes".to_string())
                        .spawn(move || {
                            let _ = answer
                                .send(emulated_sizes(display, x11_window).unwrap_or_default());
                        });
                    let sizes = read
                        .ok()
                        .and_then(|_| {
                            heard
                                .recv_timeout(std::time::Duration::from_millis(150))
                                .ok()
                        })
                        .unwrap_or_default();
                    if is_emulated(&sizes, asked, scale) {
                        log::info!(
                            "An X11 program has {w}x{h} to itself; its window is shown over the display"
                        );
                        crate::surface::Emulated::set(&window, Some(asked));
                        let _ =
                            window.configure(Rectangle::new((rect.x, rect.y).into(), asked.into()));
                        self.desktop.needs_redraw = true;
                        return;
                    }
                }
            }
        }
        let known = self
            .desktop
            .id_of(&WindowSurface::X11(window.clone()))
            .and_then(|id| Some((id, self.desktop.windows.get(&id)?)))
            .filter(|(_, record)| record.mapped)
            .map(|(id, record)| (id, record.rect, record.fullscreen || record.maximized));
        let Some((id, rect, filling)) = known else {
            // Not shown yet, or left to its program (a notification that
            // moves up as another goes): as it asks.
            let geometry = window.geometry();
            let now = gevrek_engine::Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            );
            let asked = asked_rect(now, (x, y), (w, h));
            if let Err(error) = window.configure(Rectangle::new(
                (asked.x, asked.y).into(),
                (asked.width, asked.height).into(),
            )) {
                log::debug!("an X11 window was not given the place it asked for: {error}");
            }
            self.desktop.needs_redraw = true;
            return;
        };
        // Its place is its program's to choose only where no layout and
        // no state gave it one, and not while the pointer moves it or an
        // overview shows its picture elsewhere.
        let manager = &self.core.monitor_manager;
        let tiled = manager.find_window(id).is_some() && !manager.is_window_floating(id);
        let held = self.drag.as_ref().is_some_and(|drag| drag.id == id);
        let asked = asked_rect(rect, (x, y), (w, h));
        if tiled || filling || held || !self.desktop.decorations_visible || asked == rect {
            if let Err(error) = window.configure(Rectangle::new(
                (rect.x, rect.y).into(),
                (rect.width, rect.height).into(),
            )) {
                log::debug!("an X11 window was not told the place it keeps: {error}");
            }
            return;
        }
        self.desktop.place_now(id, asked);
        if (asked.x, asked.y) != (rect.x, rect.y) {
            self.window_event(WindowEvent::Moved(id, asked.x, asked.y));
        }
        if (asked.width, asked.height) != (rect.width, rect.height) {
            self.window_event(WindowEvent::Resized(id, asked.width, asked.height));
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // An override-redirect window moved itself.
        self.desktop.needs_redraw = true;
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        if matches!(property, WmWindowProperty::Title | WmWindowProperty::Class) {
            self.metadata_changed(&WindowSurface::X11(window));
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Maximized(true),
        );
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Maximized(false),
        );
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Fullscreen(true),
        );
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Fullscreen(false),
        );
    }

    /// The program puts its own window away (the button of a title bar it
    /// draws itself), or brings it back.
    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Minimized(true),
        );
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_state_request(
            &window,
            gevrek_engine::contract::StateChange::Minimized(false),
        );
    }

    /// An X11 program may read the clipboard while an X11 window has the
    /// keyboard (a program in the background may not).
    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.desktop
            .focused
            .and_then(|id| self.desktop.windows.get(&id))
            .is_some_and(|window| match &window.surface {
                WindowSurface::X11(surface) => surface.xwm_id() == Some(xwm),
                WindowSurface::Xdg(_) => false,
            })
    }

    /// An X11 program asks for what a Wayland client put on the clipboard.
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
    ) {
        let result = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|e| e.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|e| e.to_string()),
        };
        if let Err(error) = result {
            log::warn!("clipboard to X11: {error}");
        }
    }

    /// An X11 program put something on the clipboard: Wayland clients are
    /// offered it.
    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        log::debug!(
            "an X11 program put something on the {selection:?} selection ({} kinds of data)",
            mime_types.len()
        );
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display, &self.seat, mime_types, ())
            }
        }
    }

    /// Xwayland ended (it crashed, or was killed): the X11 programs are
    /// gone with it. Their windows are taken out, and Xwayland is started
    /// again so that the next X11 program finds one; not more often than
    /// a few times, should it not stay up.
    fn disconnected(&mut self, _xwm: XwmId) {
        log::warn!("Xwayland ended");
        self.x_drag = XDrag::default();
        self.xwm = None;
        self.x_display = None;
        self.x_client = None;
        std::env::remove_var("DISPLAY");
        let gone: Vec<X11Surface> = self
            .desktop
            .windows
            .values()
            .filter_map(|window| match &window.surface {
                WindowSurface::X11(surface) => Some(surface.clone()),
                WindowSurface::Xdg(_) => None,
            })
            .collect();
        log::debug!(
            "the {} windows of X11 programs went with Xwayland",
            gone.len()
        );
        for window in gone {
            self.x11_window_gone(&window);
        }
        self.x11_overrides.clear();
        // What paired the old Xwayland's windows with their surfaces is
        // thrown away with it. The library keeps the surfaces by the
        // numbers Xwayland gave them, and a new Xwayland counts from the
        // start again: with the old ones kept, a new window was paired,
        // now and then, with a surface of the Xwayland that is gone, and
        // was never shown (found by nested check XR1, about one time in
        // four).
        let old = self.xwayland_shell_state.global();
        self.display.remove_global::<State>(old);
        self.xwayland_shell_state =
            smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&self.display);
        self.desktop.needs_redraw = true;
        if !self.xwayland_wanted {
            return;
        }
        let now = std::time::Instant::now();
        self.xwayland_restarts
            .retain(|at| now.duration_since(*at) < std::time::Duration::from_secs(60));
        if self.xwayland_restarts.len() >= 3 {
            log::error!(
                "Xwayland ended four times within a minute: it is not started again. \
                 X11 programs do not run until gevrek is started anew."
            );
            return;
        }
        self.xwayland_restarts.push(now);
        let Some(handle) = self.loop_handle.clone() else {
            return;
        };
        // A moment later: what is left of the old one is gone by then.
        let restart = handle.clone();
        let inserted = handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(
                std::time::Duration::from_millis(500),
            ),
            move |_, _, state| {
                match state.start_xwayland(restart.clone()) {
                    Ok(()) => log::info!("Xwayland is started again"),
                    Err(error) => log::error!("Xwayland could not be started again: {error}"),
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        );
        if inserted.is_err() {
            log::error!("Xwayland could not be started again");
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display, &self.seat);
                }
            }
        }
    }

    /// An X11 program asks to be resized by the pointer (the user pressed
    /// on an edge it draws itself).
    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: ResizeEdge,
    ) {
        if self.drag.is_some() {
            return;
        }
        let Some(id) = self.desktop.id_of(&WindowSurface::X11(window)) else {
            return;
        };
        let (left, top) = match resize_edge {
            ResizeEdge::Left | ResizeEdge::BottomLeft => (true, false),
            ResizeEdge::Top | ResizeEdge::TopRight => (false, true),
            ResizeEdge::TopLeft => (true, true),
            _ => (false, false),
        };
        // The button the program reports is X11's (1 is the left one).
        let code = if button == 3 { 0x111 } else { 0x110 };
        self.begin_drag(id, Some((left, top)), code);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        if !self.desktop.primary_button_down || self.drag.is_some() {
            return;
        }
        if let Some(id) = self.desktop.id_of(&WindowSurface::X11(window)) {
            self.begin_drag(id, None, 0x110);
        }
    }
}

impl State {
    /// The client asked for a state itself: the desktop records it and the
    /// core hears about the window like after any change of its own.
    /// An X11 window covers a display of its own accord: it has no
    /// frame and is exactly as large as a display. That is how X11 games
    /// went fullscreen before there was a word for it, and many still do.
    fn x11_covers_a_display(&self, window: &X11Surface, width: i32, height: i32) -> bool {
        // (The library's word for "the window says it wants no frame
        // from the window manager" is that it is decorated by itself.)
        window.is_decorated()
            && !window.is_override_redirect()
            && width > 0
            && self.desktop.outputs.iter().any(|output| {
                self.desktop
                    .output_rectangle(output.id)
                    .is_some_and(|screen| screen.width == width && screen.height == height)
            })
    }

    /// A new X11 window that is to be fullscreen from the start: it said
    /// so before it was shown (`_NET_WM_STATE_FULLSCREEN` set on a window
    /// that is not mapped yet, for which no request comes), or it covers
    /// a display in the older way.
    pub(crate) fn x11_fullscreen_from_the_start(&mut self, id: gevrek_engine::WindowHandle) {
        let Some(record) = self.desktop.windows.get(&id) else {
            return;
        };
        let WindowSurface::X11(window) = &record.surface else {
            return;
        };
        if record.fullscreen {
            return;
        }
        let window = window.clone();
        let (width, height) = (record.rect.width, record.rect.height);
        let said_so = {
            let x11_window = window.window_id();
            let before = self.x11_start_fullscreen.len();
            self.x11_start_fullscreen
                .retain(|known| *known != x11_window);
            self.x11_start_fullscreen.len() != before
        };
        if said_so || window.is_fullscreen() || self.x11_covers_a_display(&window, width, height) {
            log::info!("An X11 window is fullscreen from the start ({width}x{height})");
            self.x11_state_request(
                &window,
                gevrek_engine::contract::StateChange::Fullscreen(true),
            );
        }
    }

    fn x11_state_request(
        &mut self,
        window: &X11Surface,
        change: gevrek_engine::contract::StateChange,
    ) {
        let Some(id) = self.desktop.id_of(&WindowSurface::X11(window.clone())) else {
            return;
        };
        // (No longer fullscreen: the size it had to itself is over.)
        if matches!(
            change,
            gevrek_engine::contract::StateChange::Fullscreen(false)
        ) {
            crate::surface::Emulated::set(window, None);
        }
        if let Err(error) = self
            .desktop
            .apply(&gevrek_engine::Effect::State { id, change })
        {
            log::debug!("X11 state request: {error}");
        }
        self.window_event(WindowEvent::MetadataChanged(id));
    }
}

/// Whether an X11 window has `_NET_WM_STATE_FULLSCREEN` among the states
/// it wrote on itself. Asked of the X server on a line of its own.
/// The X resources with what a scale means in them: `Xft.dpi` (96 at
/// a scale of one) and `Xcursor.size` (24 at one). Whatever else is
/// there stays; earlier lines of these two are replaced.
pub(crate) fn resources_at(existing: &str, scale: f64) -> String {
    let ours = ["Xft.dpi:", "Xcursor.size:"];
    let mut text: String = existing
        .lines()
        .filter(|line| !ours.iter().any(|name| line.trim_start().starts_with(name)))
        .map(|line| format!("{line}\n"))
        .collect();
    text.push_str(&format!("Xft.dpi:\t{}\n", (96.0 * scale).round() as i64));
    text.push_str(&format!(
        "Xcursor.size:\t{}\n",
        (24.0 * scale).round() as i64
    ));
    text
}

/// Write them where X11 programs read them: `RESOURCE_MANAGER` on the
/// root window (what `xrdb` writes).
fn write_x11_resources(display: u32, scale: f64) -> Option<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;
    let (connection, screen) = x11rb::connect(Some(&format!(":{display}"))).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let existing = connection
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            1 << 20,
        )
        .ok()?
        .reply()
        .ok()?;
    let text = resources_at(&String::from_utf8_lossy(&existing.value), scale);
    connection
        .change_property8(
            PropMode::REPLACE,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            text.as_bytes(),
        )
        .ok()?
        .check()
        .ok()?;
    connection.flush().ok()
}

/// The time on the clock that never goes back, as events carry it.
fn now_ms() -> u32 {
    crate::capture::monotonic_now().as_millis() as u32
}

/// Whether a size a window asks for (in points) is one of the
/// resolutions its program has to itself (in X's pixels, which are
/// `scale` to a point).
pub(crate) fn is_emulated(sizes: &[(i32, i32)], asked: (i32, i32), scale: f64) -> bool {
    let points = |pixels: i32| (f64::from(pixels) / scale.max(0.1)).round() as i32;
    sizes
        .iter()
        .any(|(width, height)| (points(*width), points(*height)) == asked)
}

/// The resolutions Xwayland gives a window's program for itself: what
/// it writes on the window (`_XWAYLAND_RANDR_EMU_MONITOR_RECTS`, four
/// numbers a display: where, and how large).
fn emulated_sizes(display: u32, window: u32) -> Option<Vec<(i32, i32)>> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let (connection, _) = x11rb::connect(Some(&format!(":{display}"))).ok()?;
    let atom = connection
        .intern_atom(false, b"_XWAYLAND_RANDR_EMU_MONITOR_RECTS")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = connection
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 64)
        .ok()?
        .reply()
        .ok()?;
    let numbers: Vec<u32> = reply.value32()?.collect();
    Some(
        numbers
            .as_chunks::<4>()
            .0
            .iter()
            .map(|rect| (rect[2] as i32, rect[3] as i32))
            .collect(),
    )
}

/// What `_NET_WM_BYPASS_COMPOSITOR` says: 2 is "never past the
/// compositor"; 0 (no wish), 1 (past it, please) and anything else are not.
pub(crate) fn never_past_the_compositor(hint: u32) -> bool {
    hint == 2
}

/// The kinds of window (`_NET_WM_WINDOW_TYPE_` and this name) there are,
/// as a window names its own.
const WINDOW_KINDS: [&str; 14] = [
    "DESKTOP",
    "DOCK",
    "TOOLBAR",
    "MENU",
    "UTILITY",
    "SPLASH",
    "DIALOG",
    "DROPDOWN_MENU",
    "POPUP_MENU",
    "TOOLTIP",
    "NOTIFICATION",
    "COMBO",
    "DND",
    "NORMAL",
];

/// Whether an X11 window of this kind is its program's own to place and
/// to show, like one that says no window manager is to touch it: a
/// tooltip, a notification, the menus that open under the pointer or
/// under a button, the list of a combo box, the picture of what is being
/// dragged. (A torn-off menu, a tool's window, a dialog and a splash
/// screen are windows of a workspace, and float.)
pub(crate) fn kind_places_itself(kind: &str) -> bool {
    matches!(
        kind,
        "TOOLTIP" | "NOTIFICATION" | "POPUP_MENU" | "DROPDOWN_MENU" | "COMBO" | "DND"
    )
}

/// The place and size an X11 window is to have when its program asks for
/// some of the four (the others stay as they are). No size under one.
pub(crate) fn asked_rect(
    now: gevrek_engine::Rect,
    (x, y): (Option<i32>, Option<i32>),
    (width, height): (Option<u32>, Option<u32>),
) -> gevrek_engine::Rect {
    let size = |asked: Option<u32>, now: i32| {
        asked.map_or(now, |asked| i32::try_from(asked).unwrap_or(i32::MAX).max(1))
    };
    gevrek_engine::Rect::new(
        x.unwrap_or(now.x),
        y.unwrap_or(now.y),
        size(width, now.width),
        size(height, now.height),
    )
}

/// Whether a window that places itself is in sight, by the windows of
/// workspaces that X11 programs have (each with its program and whether
/// its workspace is shown): with those of its own program, where it has
/// any and says which program it is of; else it is shown.
pub(crate) fn in_sight_with_its_program(
    program: Option<u32>,
    windows: &[(Option<u32>, bool)],
) -> bool {
    let Some(program) = program else {
        return true;
    };
    let mut its_own = windows
        .iter()
        .filter(|(other, _)| *other == Some(program))
        .map(|(_, shown)| *shown)
        .peekable();
    its_own.peek().is_none() || its_own.any(|shown| shown)
}

/// Whether enough of a window lies on a display to take hold of it.
pub(crate) fn reaches_a_display(
    window: gevrek_engine::Rect,
    displays: &[gevrek_engine::Rect],
) -> bool {
    displays.iter().any(|display| {
        let wide =
            (window.x + window.width).min(display.x + display.width) - window.x.max(display.x);
        let high =
            (window.y + window.height).min(display.y + display.height) - window.y.max(display.y);
        wide > 0 && high > 0 && wide >= window.width.min(32) && high >= window.height.min(16)
    })
}

/// What an X11 window wrote on itself before it is shown.
#[derive(Debug, Default)]
struct Said {
    /// `_NET_WM_STATE_FULLSCREEN` is among its states.
    fullscreen: bool,
    /// Its `_NET_WM_BYPASS_COMPOSITOR` (0 where it says none).
    bypass: u32,
    /// `_NET_WM_STATE_ABOVE` is among its states.
    above: bool,
    /// The kind of window it is: the first it names that there is (one
    /// of `WINDOW_KINDS`), or none where it names none.
    kind: Option<String>,
}

/// Read what an X11 window wrote on itself. Asked of the X server on a
/// line of its own.
fn said_of_itself(display: u32, window: u32) -> Option<Said> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let (connection, _) = x11rb::connect(Some(&format!(":{display}"))).ok()?;
    let atom = |name: &[u8]| -> Option<u32> {
        Some(connection.intern_atom(false, name).ok()?.reply().ok()?.atom)
    };
    let (state, fullscreen) = (atom(b"_NET_WM_STATE")?, atom(b"_NET_WM_STATE_FULLSCREEN")?);
    let reply = connection
        .get_property(false, window, state, AtomEnum::ATOM, 0, 64)
        .ok()?
        .reply()
        .ok()?;
    let states: Vec<u32> = reply.value32().map(Iterator::collect).unwrap_or_default();
    let is_fullscreen = states.contains(&fullscreen);
    let above = atom(b"_NET_WM_STATE_ABOVE").is_some_and(|above| states.contains(&above));
    // The kind: all the names are asked for at once, then heard.
    let named: Vec<_> = WINDOW_KINDS
        .iter()
        .filter_map(|kind| {
            let name = format!("_NET_WM_WINDOW_TYPE_{kind}");
            Some((*kind, connection.intern_atom(false, name.as_bytes()).ok()?))
        })
        .collect();
    let kinds: Vec<(&str, u32)> = named
        .into_iter()
        .filter_map(|(kind, asked)| Some((kind, asked.reply().ok()?.atom)))
        .collect();
    let kind = atom(b"_NET_WM_WINDOW_TYPE")
        .and_then(|kind| {
            connection
                .get_property(false, window, kind, AtomEnum::ATOM, 0, 16)
                .ok()?
                .reply()
                .ok()
        })
        .and_then(|reply| {
            reply.value32()?.find_map(|one| {
                kinds
                    .iter()
                    .find(|(_, known)| *known == one)
                    .map(|(kind, _)| (*kind).to_string())
            })
        });
    let bypass = atom(b"_NET_WM_BYPASS_COMPOSITOR")
        .and_then(|bypass| {
            connection
                .get_property(false, window, bypass, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()
        })
        .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
        .unwrap_or(0);
    Some(Said {
        fullscreen: is_fullscreen,
        bypass,
        above,
        kind,
    })
}

smithay::delegate_xwayland_shell!(State);

#[cfg(test)]
mod tests {
    use super::{
        asked_rect, in_sight_with_its_program, is_emulated, kind_places_itself,
        never_past_the_compositor, reaches_a_display, resources_at,
    };
    use gevrek_engine::Rect;

    #[test]
    fn a_tooltip_a_notification_and_a_menu_are_left_to_their_program() {
        for kind in [
            "TOOLTIP",
            "NOTIFICATION",
            "POPUP_MENU",
            "DROPDOWN_MENU",
            "COMBO",
            "DND",
        ] {
            assert!(kind_places_itself(kind), "{kind}");
        }
        // Windows of a workspace: tiled, or floating over the others.
        for kind in [
            "NORMAL", "DIALOG", "UTILITY", "SPLASH", "TOOLBAR", "MENU", "DOCK", "DESKTOP",
        ] {
            assert!(!kind_places_itself(kind), "{kind}");
        }
        // Every kind that is left to its program is one that is told apart.
        for kind in super::WINDOW_KINDS {
            assert_eq!(
                kind_places_itself(kind),
                [
                    "TOOLTIP",
                    "NOTIFICATION",
                    "POPUP_MENU",
                    "DROPDOWN_MENU",
                    "COMBO",
                    "DND"
                ]
                .contains(&kind)
            );
        }
    }

    #[test]
    fn a_program_that_asks_for_a_place_or_a_size_changes_only_what_it_names() {
        let now = Rect::new(100, 200, 640, 480);
        assert_eq!(asked_rect(now, (None, None), (None, None)), now);
        // Moved only (a notification that goes up as another leaves).
        assert_eq!(
            asked_rect(now, (Some(1500), Some(900)), (None, None)),
            Rect::new(1500, 900, 640, 480)
        );
        // Made larger only: it stays where it is.
        assert_eq!(
            asked_rect(now, (None, None), (Some(800), None)),
            Rect::new(100, 200, 800, 480)
        );
        // On a display to the left of the first, places are below zero.
        assert_eq!(
            asked_rect(now, (Some(-1900), None), (None, Some(300))),
            Rect::new(-1900, 200, 640, 300)
        );
        // No size of nothing.
        assert_eq!(
            asked_rect(now, (None, None), (Some(0), Some(0))),
            Rect::new(100, 200, 1, 1)
        );
    }

    #[test]
    fn what_places_itself_comes_and_goes_with_the_windows_of_its_program() {
        // The editor (program 7) is on a workspace that is not shown; a
        // terminal of another program is.
        let away = [(Some(7), false), (Some(7), false), (Some(9), true)];
        assert!(!in_sight_with_its_program(Some(7), &away));
        assert!(in_sight_with_its_program(Some(9), &away));
        // One of its windows is shown (a tool's window on this workspace).
        let partly = [(Some(7), false), (Some(7), true)];
        assert!(in_sight_with_its_program(Some(7), &partly));
        // A program with no window of a workspace, or one that does not
        // say which program it is of: shown.
        assert!(in_sight_with_its_program(Some(3), &away));
        assert!(in_sight_with_its_program(None, &away));
        assert!(in_sight_with_its_program(Some(3), &[]));
    }

    #[test]
    fn a_place_a_program_names_is_taken_only_where_the_window_can_be_reached() {
        let displays = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 2560, 1440)];
        assert!(reaches_a_display(Rect::new(100, 100, 400, 300), &displays));
        // On the second display, and across the two.
        assert!(reaches_a_display(
            Rect::new(4000, 1200, 400, 300),
            &displays
        ));
        assert!(reaches_a_display(Rect::new(1800, 500, 400, 300), &displays));
        // Beyond every display, or with a sliver left on one.
        assert!(!reaches_a_display(
            Rect::new(5000, 100, 400, 300),
            &displays
        ));
        assert!(!reaches_a_display(
            Rect::new(-390, 100, 400, 300),
            &displays
        ));
        assert!(!reaches_a_display(
            Rect::new(100, 1075, 400, 300),
            &displays
        ));
        // A window smaller than a hold is reached where all of it is.
        assert!(reaches_a_display(Rect::new(10, 10, 20, 10), &displays));
        assert!(!reaches_a_display(Rect::new(100, 100, 400, 300), &[]));
    }

    #[test]
    fn a_size_is_a_programs_own_resolution_when_xwayland_wrote_it_on_the_window() {
        let sizes = [(800, 600), (1024, 768)];
        assert!(is_emulated(&sizes, (800, 600), 1.0));
        assert!(!is_emulated(&sizes, (640, 480), 1.0));
        // On a display at twice the scale X's pixels are two to a point.
        assert!(is_emulated(&[(1600, 1200)], (800, 600), 2.0));
        assert!(!is_emulated(&[], (800, 600), 1.0));
    }

    #[test]
    fn only_a_window_that_says_never_is_kept_from_the_displays_plane() {
        assert!(never_past_the_compositor(2));
        assert!(!never_past_the_compositor(0) && !never_past_the_compositor(1));
        assert!(!never_past_the_compositor(7));
    }

    #[test]
    fn x11_programs_are_told_how_fine_the_display_is() {
        assert_eq!(resources_at("", 2.0), "Xft.dpi:\t192\nXcursor.size:\t48\n");
        assert_eq!(resources_at("", 1.5), "Xft.dpi:\t144\nXcursor.size:\t36\n");
        // What else is there stays; what was said of these before goes.
        let before = "XTerm*faceName: mono\nXft.dpi:\t96\nXcursor.size: 24\nXft.antialias: 1\n";
        assert_eq!(
            resources_at(before, 2.0),
            "XTerm*faceName: mono\nXft.antialias: 1\nXft.dpi:\t192\nXcursor.size:\t48\n"
        );
        // Back to one.
        assert_eq!(
            resources_at(&resources_at(before, 2.0), 1.0),
            "XTerm*faceName: mono\nXft.antialias: 1\nXft.dpi:\t96\nXcursor.size:\t24\n"
        );
    }
}
