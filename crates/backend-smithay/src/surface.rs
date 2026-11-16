//! What a window is made of: a Wayland toplevel or an X11 window (through
//! Xwayland), and what can have the keyboard.

use crate::state::State;
use gevrek_engine::Rect;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::input::Seat;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Rectangle, Serial};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::xwayland::X11Surface;
use std::borrow::Cow;

/// The client side of a window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowSurface {
    Xdg(ToplevelSurface),
    X11(X11Surface),
}

/// A state a window can be put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFlag {
    Maximized,
    Fullscreen,
    Activated,
}

/// The size an X11 window has to itself while its program has another
/// resolution than the display's (an older game changes the resolution
/// and covers that with its window; Xwayland gives it the resolution
/// for itself alone and shows the window large). Kept with the window:
/// while it is there, the window is told that size and not the
/// display's.
#[derive(Default)]
pub struct Emulated(std::sync::Mutex<Option<(i32, i32)>>);

impl Emulated {
    pub fn of(window: &smithay::xwayland::X11Surface) -> Option<(i32, i32)> {
        window.user_data().get::<Emulated>().and_then(|kept| {
            *kept
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        })
    }

    pub fn set(window: &smithay::xwayland::X11Surface, size: Option<(i32, i32)>) {
        let kept = window
            .user_data()
            .get_or_insert_threadsafe(Emulated::default);
        *kept
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = size;
    }
}

impl WindowSurface {
    /// The surface the window draws on. An X11 window has none until
    /// Xwayland has paired it with one.
    pub fn wl_surface(&self) -> Option<WlSurface> {
        match self {
            Self::Xdg(toplevel) => Some(toplevel.wl_surface().clone()),
            Self::X11(window) => window.wl_surface(),
        }
    }

    /// The client can be told sizes and states (a Wayland toplevel must
    /// first have been sent its initial configure).
    pub fn ready(&self) -> bool {
        match self {
            Self::Xdg(toplevel) => toplevel.is_initial_configure_sent(),
            Self::X11(window) => window.alive(),
        }
    }

    /// Ask the client to be this size at this place. A Wayland client only
    /// learns the size; an X11 client is told both.
    pub fn request_rect(&self, rect: Rect) {
        match self {
            Self::Xdg(toplevel) => {
                toplevel.with_pending_state(|state| {
                    state.size = Some((rect.width, rect.height).into());
                });
                toplevel.send_pending_configure();
            }
            Self::X11(window) => {
                // (A program that changed the resolution for itself keeps
                // the size it has to itself: see `Emulated`.)
                let (width, height) = Emulated::of(window).unwrap_or((rect.width, rect.height));
                let geometry = Rectangle::new((rect.x, rect.y).into(), (width, height).into());
                if let Err(error) = window.configure(geometry) {
                    log::debug!("X11 configure: {error}");
                }
            }
        }
    }

    /// Tell the client whether it is tiled (all four edges touch something:
    /// it then draws no shadow and no rounded corners of its own). Sent
    /// with the next configure.
    pub fn set_tiled(&self, tiled: bool) {
        if let Self::Xdg(toplevel) = self {
            toplevel.with_pending_state(|state| {
                for edge in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    if tiled {
                        state.states.set(edge);
                    } else {
                        state.states.unset(edge);
                    }
                }
            });
        }
    }

    /// Tell the client what was staged (`stage_state`, `set_tiled`), if it
    /// does not know yet.
    pub fn flush(&self) {
        if let Self::Xdg(toplevel) = self {
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
    }

    /// As `set_state`, but the client is told with the next configure
    /// (`request_rect`, `flush`), so that a new state and its new size
    /// arrive together.
    pub fn stage_state(&self, flag: StateFlag, on: bool) {
        match self {
            Self::Xdg(toplevel) => {
                let state_flag = match flag {
                    StateFlag::Maximized => xdg_toplevel::State::Maximized,
                    StateFlag::Fullscreen => xdg_toplevel::State::Fullscreen,
                    StateFlag::Activated => xdg_toplevel::State::Activated,
                };
                toplevel.with_pending_state(|state| {
                    if on {
                        state.states.set(state_flag);
                    } else {
                        state.states.unset(state_flag);
                    }
                });
            }
            Self::X11(_) => self.set_state(flag, on),
        }
    }

    pub fn set_state(&self, flag: StateFlag, on: bool) {
        match self {
            Self::Xdg(toplevel) => {
                let state_flag = match flag {
                    StateFlag::Maximized => xdg_toplevel::State::Maximized,
                    StateFlag::Fullscreen => xdg_toplevel::State::Fullscreen,
                    StateFlag::Activated => xdg_toplevel::State::Activated,
                };
                toplevel.with_pending_state(|state| {
                    if on {
                        state.states.set(state_flag);
                    } else {
                        state.states.unset(state_flag);
                    }
                });
                toplevel.send_pending_configure();
            }
            Self::X11(window) => {
                let result = match flag {
                    StateFlag::Maximized => window.set_maximized(on),
                    StateFlag::Fullscreen => window.set_fullscreen(on),
                    StateFlag::Activated => window.set_activated(on),
                };
                if let Err(error) = result {
                    log::debug!("X11 state: {error}");
                }
            }
        }
    }

    /// Ask the client to close the window.
    pub fn close(&self) {
        match self {
            Self::Xdg(toplevel) => toplevel.send_close(),
            Self::X11(window) => {
                if let Err(error) = window.close() {
                    log::debug!("X11 close: {error}");
                }
            }
        }
    }

    /// The smallest size the window's program lets it have: a width and
    /// a height (0 where it names none for that side), or nothing where
    /// it names none at all. A layout gives such a window room.
    pub fn smallest_size(&self) -> Option<(i32, i32)> {
        let named = match self {
            Self::Xdg(toplevel) => {
                smithay::wayland::compositor::with_states(toplevel.wl_surface(), |states| {
                    let mut cached = states
                        .cached_state
                        .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                    let least = cached.current().min_size;
                    (least.w, least.h)
                })
            }
            Self::X11(window) => window
                .min_size()
                .map(|least| (least.w, least.h))
                .unwrap_or((0, 0)),
        };
        (named.0 > 0 || named.1 > 0).then_some((named.0.max(0), named.1.max(0)))
    }

    /// The window is a dialog, by what its program says of it: it
    /// belongs to another window, it says it is to be answered before
    /// anything else (modal), it cannot be made larger or smaller (its
    /// smallest size is its largest), or, an X11 window, it is of a kind
    /// that is no ordinary window (a dialog, a tool's window, a splash
    /// screen). Such a window floats: a question for a password or for
    /// leave to do something is not a tile among the others.
    pub fn is_dialog(&self) -> bool {
        match self {
            Self::Xdg(toplevel) => {
                if toplevel.parent().is_some() {
                    return true;
                }
                smithay::wayland::compositor::with_states(toplevel.wl_surface(), |states| {
                    let modal = states
                        .data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .is_some_and(|data| {
                            data.lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .modal
                        });
                    let mut cached = states
                        .cached_state
                        .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                    let limits = cached.current();
                    let fixed = limits.min_size.w > 0
                        && limits.min_size.h > 0
                        && limits.min_size == limits.max_size;
                    modal || fixed
                })
            }
            Self::X11(window) => {
                use smithay::xwayland::xwm::WmWindowType;
                window.is_transient_for().is_some()
                    || matches!(
                        window.window_type(),
                        Some(
                            WmWindowType::Dialog
                                | WmWindowType::Utility
                                | WmWindowType::Splash
                                | WmWindowType::Toolbar
                                // (No windows to tile either, where one
                                // of them is not left to its program.)
                                | WmWindowType::Menu
                                | WmWindowType::DropdownMenu
                                | WmWindowType::PopupMenu
                                | WmWindowType::Tooltip
                                | WmWindowType::Notification
                        )
                    )
                    || window
                        .min_size()
                        .zip(window.max_size())
                        .is_some_and(|(least, most)| least.w > 0 && least.h > 0 && least == most)
            }
        }
    }

    /// The window belongs to another one (a dialog).
    pub fn has_parent(&self) -> bool {
        match self {
            Self::Xdg(toplevel) => toplevel.parent().is_some(),
            Self::X11(window) => window.is_transient_for().is_some(),
        }
    }

    /// What gets the keyboard when this window has the focus.
    pub fn key_target(&self) -> Option<KeyTarget> {
        match self {
            Self::Xdg(toplevel) => Some(KeyTarget::Wl(toplevel.wl_surface().clone())),
            Self::X11(window) => Some(KeyTarget::X11(window.clone())),
        }
    }
}

/// What can have the keyboard: a Wayland surface (a toplevel, a layer
/// surface), or an X11 window, which also needs the X input focus.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyTarget {
    Wl(WlSurface),
    X11(X11Surface),
}

impl IsAlive for KeyTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wl(surface) => surface.alive(),
            Self::X11(window) => window.alive(),
        }
    }
}

impl WaylandFocus for KeyTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wl(surface) => Some(Cow::Borrowed(surface)),
            Self::X11(window) => window.wl_surface().map(Cow::Owned),
        }
    }
}

impl KeyboardTarget<State> for KeyTarget {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Wl(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
            Self::X11(window) => KeyboardTarget::enter(window, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        match self {
            Self::Wl(surface) => KeyboardTarget::leave(surface, seat, data, serial),
            Self::X11(window) => KeyboardTarget::leave(window, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Self::Wl(surface) => KeyboardTarget::key(surface, seat, data, key, state, serial, time),
            Self::X11(window) => KeyboardTarget::key(window, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Wl(surface) => KeyboardTarget::modifiers(surface, seat, data, modifiers, serial),
            Self::X11(window) => KeyboardTarget::modifiers(window, seat, data, modifiers, serial),
        }
    }
}

// A menu (a popup with a grab) takes the keyboard while it is open, and the
// pointer goes to it and to the windows of its own program only. Smithay's
// popup grabs need these two conversions for that.
impl From<smithay::desktop::PopupKind> for KeyTarget {
    fn from(popup: smithay::desktop::PopupKind) -> Self {
        Self::Wl(popup.wl_surface().clone())
    }
}

impl From<KeyTarget> for WlSurface {
    fn from(target: KeyTarget) -> Self {
        match target {
            KeyTarget::Wl(surface) => surface,
            // Only a Wayland window can be the root of a popup grab; an X11
            // window that has the keyboard is mapped, so it has a surface.
            KeyTarget::X11(window) => window
                .wl_surface()
                .expect("an X11 window with the keyboard has a surface"),
        }
    }
}
