//! The protocols beyond the window basics: layer-shell (bars, launchers,
//! wallpapers and the space they reserve), activation requests, the window
//! list for external bars, and the small ones clients expect to find.

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::{Rect, WindowHandle};
use smithay::desktop::{layer_map_for_output, LayerSurface, WindowSurfaceType};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::with_states;
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleHandler};
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState,
};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
    WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

/// An activation token older than this no longer gives the focus.
const ACTIVATION_TOKEN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(10);

impl State {
    fn output_for_layer(&self, surface: &WlSurface) -> Option<(MonitorHandle, Output)> {
        self.outputs
            .iter()
            .find(|(_, output)| {
                layer_map_for_output(output)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
    }

    /// A layer surface was committed: arrange the layers of its output,
    /// answer its first commit with a configure, and give the windows the
    /// space that is left.
    pub(crate) fn layer_committed(&mut self, surface: &WlSurface) -> bool {
        let Some((id, output)) = self.output_for_layer(surface) else {
            return false;
        };
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .map(|data| {
                    data.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .initial_configure_sent
                })
                .unwrap_or(true)
        });
        let wants_keyboard = {
            let mut map = layer_map_for_output(&output);
            map.arrange();
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned();
            if let Some(layer) = &layer {
                if !initial_configure_sent {
                    layer.layer_surface().send_configure();
                }
            }
            layer.is_some_and(|layer| {
                let state = layer.cached_state();
                matches!(state.layer, Layer::Top | Layer::Overlay)
                    && state.keyboard_interactivity == KeyboardInteractivity::Exclusive
            })
        };
        // A launcher or a lock prompt takes the keyboard while it is there.
        if wants_keyboard && self.layer_focus.as_ref() != Some(surface) {
            self.layer_focus = Some(surface.clone());
            self.desktop.focus_dirty = true;
        }
        self.update_work_area(id, &output);
        true
    }

    /// The space of an output that no bar reserves is where windows go.
    fn update_work_area(&mut self, id: MonitorHandle, output: &Output) {
        let free = layer_map_for_output(output).non_exclusive_zone();
        let Some(record) = self
            .desktop
            .outputs
            .iter_mut()
            .find(|record| record.id == id)
        else {
            return;
        };
        let work = Rect::new(
            record.rect.x + free.loc.x,
            record.rect.y + free.loc.y,
            free.size.w,
            free.size.h,
        );
        self.desktop.needs_redraw = true;
        if record.work == work {
            return;
        }
        record.work = work;
        self.publish_outputs();
        self.core.executor.mark_layout_dirty(id);
        self.settle();
    }

    /// The layer surface at a point of an output, on one of `layers`, with
    /// the surface (or popup, or subsurface) of it that is there.
    pub(crate) fn layer_surface_under(
        &self,
        location: Point<f64, Logical>,
        layers: &[Layer],
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        for (id, output) in &self.outputs {
            let Some(screen) = self.desktop.output_rectangle(*id) else {
                continue;
            };
            let local: Point<f64, Logical> = (
                location.x - f64::from(screen.x),
                location.y - f64::from(screen.y),
            )
                .into();
            let map = layer_map_for_output(output);
            for layer in layers {
                let Some(surface) = map.layer_under(*layer, local) else {
                    continue;
                };
                let Some(geometry) = map.layer_geometry(surface) else {
                    continue;
                };
                let inside = local - geometry.loc.to_f64();
                if let Some((found, at)) = surface.surface_under(inside, WindowSurfaceType::ALL) {
                    let global = (
                        f64::from(at.x + geometry.loc.x + screen.x),
                        f64::from(at.y + geometry.loc.y + screen.y),
                    )
                        .into();
                    return Some((found, global));
                }
            }
        }
        None
    }

    /// A click on a layer surface that takes the keyboard on demand gives
    /// it the keyboard; a click anywhere else takes it back.
    pub(crate) fn click_layer_focus(&mut self, location: Point<f64, Logical>) {
        let clicked = self
            .layer_surface_under(
                location,
                &[Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background],
            )
            .map(|(surface, _)| surface);
        let wanted = clicked.filter(|surface| {
            self.output_for_layer(surface).is_some_and(|(_, output)| {
                layer_map_for_output(&output)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some_and(|layer| layer.can_receive_keyboard_focus())
            })
        });
        let exclusive = self.layer_focus.as_ref().is_some_and(|surface| {
            self.output_for_layer(surface).is_some_and(|(_, output)| {
                layer_map_for_output(&output)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some_and(|layer| {
                        layer.cached_state().keyboard_interactivity
                            == KeyboardInteractivity::Exclusive
                    })
            })
        });
        // An exclusive surface keeps the keyboard until it goes away.
        if exclusive && wanted.is_none() {
            return;
        }
        if self.layer_focus != wanted {
            self.layer_focus = wanted;
            self.desktop.focus_dirty = true;
        }
    }

    /// Tell the window list (external bars) about a new window.
    pub(crate) fn announce_toplevel(&mut self, id: WindowHandle) {
        let Some(window) = self.desktop.windows.get(&id) else {
            return;
        };
        let handle = self
            .foreign_toplevels
            .new_toplevel::<State>(window.title.clone(), window.app_id.clone());
        handle.send_done();
        self.foreign_handles.insert(id, handle);
    }

    /// The title or application id of a window changed.
    pub(crate) fn update_toplevel(&mut self, id: WindowHandle) {
        let (Some(window), Some(handle)) =
            (self.desktop.windows.get(&id), self.foreign_handles.get(&id))
        else {
            return;
        };
        handle.send_title(&window.title);
        handle.send_app_id(&window.app_id);
        handle.send_done();
    }

    /// A window is gone.
    pub(crate) fn withdraw_toplevel(&mut self, id: WindowHandle) {
        if let Some(handle) = self.foreign_handles.remove(&id) {
            self.foreign_toplevels.remove_toplevel(&handle);
        }
    }
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // On the output the client named, else the one the pointer is on.
        let output = output.as_ref().and_then(Output::from_resource).or_else(|| {
            let at_pointer = gevrek_engine::Desktop::output_at_cursor(&self.desktop);
            self.outputs
                .iter()
                .find(|(id, _)| Some(*id) == at_pointer)
                .or(self.outputs.first())
                .map(|(_, output)| output.clone())
        });
        let Some(output) = output else {
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(error) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            log::warn!("layer surface: {error:?}");
        }
    }

    /// A popup was given a layer surface (a panel's menu): placed where
    /// its program asks, moved by the program's own rules so that all of
    /// it is on the surface's output. This is its first configure: until
    /// it has a parent there is nothing to place it against.
    fn new_popup(
        &mut self,
        parent: WlrLayerSurface,
        popup: smithay::wayland::shell::xdg::PopupSurface,
    ) {
        let positioner = popup.with_pending_state(|state| state.positioner);
        let target = self
            .output_for_layer(parent.wl_surface())
            .and_then(|(id, output)| {
                let screen = self.desktop.output_rectangle(id)?;
                let map = layer_map_for_output(&output);
                let layer =
                    map.layer_for_surface(parent.wl_surface(), WindowSurfaceType::TOPLEVEL)?;
                let at = map.layer_geometry(layer)?.loc;
                // The output as the layer surface sees it.
                Some(smithay::utils::Rectangle::new(
                    (-at.x, -at.y).into(),
                    (screen.width, screen.height).into(),
                ))
            });
        let geometry = match target {
            Some(target) => positioner.get_unconstrained_geometry(target),
            None => positioner.get_geometry(),
        };
        popup.with_pending_state(|state| state.geometry = geometry);
        if let Err(error) = popup.send_configure() {
            log::debug!("popup configure: {error:?}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let wl_surface = surface.wl_surface().clone();
        let found = self.outputs.iter().find_map(|(id, output)| {
            let mut map = layer_map_for_output(output);
            let layer = map
                .layers()
                .find(|layer| layer.layer_surface() == &surface)
                .cloned()?;
            map.unmap_layer(&layer);
            Some((*id, output.clone()))
        });
        if self.layer_focus.as_ref() == Some(&wl_surface) {
            self.layer_focus = None;
            self.desktop.focus_dirty = true;
        }
        if let Some((id, output)) = found {
            self.update_work_area(id, &output);
        }
        self.settle();
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    /// A client asks for one of its windows (or another client's, with a
    /// token it was handed) to get the focus. With `prevent_focus_steal`
    /// the request needs a token made from a user action; without it a
    /// fresh token is enough.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let fresh = token_data.timestamp.elapsed() < ACTIVATION_TOKEN_LIFETIME;
        let from_user_action = token_data.serial.is_some();
        self.activation_state.remove_token(&token);
        let Some(id) = self.desktop.id_of_surface(&surface) else {
            return;
        };
        self.activation_requested(id, fresh, from_user_action);
    }
}

impl State {
    /// A window asks for the focus with a token that is `fresh` or not and
    /// was made `from_user_action` or not. Whether it got it.
    ///
    /// The window rules come first: a window with `NoFocus` never gets the
    /// focus this way, and one with `NoInitialFocus` does not get it before
    /// it has had it once (a program started with a token of its launcher
    /// asks as it opens). Then the focus-steal policy: the token must be
    /// fresh, and made from a user action unless the program is on the
    /// whitelist.
    pub fn activation_requested(
        &mut self,
        id: gevrek_engine::WindowHandle,
        fresh: bool,
        from_user_action: bool,
    ) -> bool {
        let executor = &self.core.executor;
        let refused_by_rule = executor.no_focus_enabled(id)
            || (executor.no_initial_focus_enabled(id) && !self.desktop.had_focus.contains(&id));
        // A program on the whitelist may always take the focus.
        let program = gevrek_engine::WindowQueries::exe(&self.desktop, id);
        let allowed = !refused_by_rule
            && fresh
            && (from_user_action || self.core.focus_manager.can_steal_focus(&program));
        if !allowed {
            log::info!(
                "Window {id:?} asked for the focus and was refused{}",
                if refused_by_rule {
                    " by a window rule"
                } else {
                    ""
                }
            );
            return false;
        }
        if !self.desktop.shown(id) {
            return false;
        }
        self.focus_window(id);
        true
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevels
    }
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // Until the surface is known to be a window on some output
        // (`sync_surface_outputs`): the output the pointer is on, where a
        // new window opens.
        // (A surface of the shell is on the output it was made for.)
        let pointer = self.desktop.pointer;
        let preferred = match self.output_for_layer(&surface) {
            Some((_, output)) => output.current_scale().fractional_scale(),
            None => self
                .desktop
                .outputs
                .iter()
                .find(|output| output.rect.contains_point(pointer.0, pointer.1))
                .or_else(|| self.desktop.outputs.first())
                .map_or(1.0, |output| output.scale),
        };
        with_states(&surface, |states| {
            with_fractional_scale(states, |scale| scale.set_preferred_scale(preferred));
        });
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
        self.sync_idle_inhibit();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
        self.sync_idle_inhibit();
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl TabletSeatHandler for State {
    fn tablet_tool_image(
        &mut self,
        _tool: &smithay::backend::input::TabletToolDescriptor,
        _image: CursorImageStatus,
    ) {
    }
}

smithay::delegate_layer_shell!(State);
smithay::delegate_xdg_activation!(State);
smithay::delegate_foreign_toplevel_list!(State);
smithay::delegate_fractional_scale!(State);
smithay::delegate_idle_inhibit!(State);
smithay::delegate_primary_selection!(State);
smithay::delegate_viewporter!(State);
smithay::delegate_cursor_shape!(State);
