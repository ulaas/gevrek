//! Every key for gevrek while it runs as a window of another Wayland
//! session.
//!
//! The host's compositor takes its own shortcuts first (on KDE: Meta opens
//! the launcher, Meta+1 starts a program, Meta+Tab, ...), so gevrek's
//! bindings with the same keys would never be seen. A client can ask the
//! host to leave its shortcuts alone while one surface has the keyboard
//! (`keyboard-shortcuts-inhibit`, the protocol virtual machine viewers
//! use): gevrek asks that for its window. The bindings of the configuration
//! then work unchanged, with the same modifier as on the hardware.
//!
//! The window library (winit) owns the connection to the host and does not
//! speak that protocol, so this module talks on the same connection through
//! an event queue of its own.

use smithay::reexports::winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use smithay::reexports::winit::window::Window;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1,
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1 as Manager,
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1 as Inhibitor},
};

/// What the host said last: whether its shortcuts are off for the window.
#[derive(Default)]
struct Answer {
    active: Option<bool>,
}

/// The request, kept for as long as the window lives.
pub struct HostShortcuts {
    queue: EventQueue<Answer>,
    answer: Answer,
    /// What `poll` returned last.
    reported: Option<bool>,
    _inhibitor: Inhibitor,
}

impl HostShortcuts {
    /// Ask the host to send every key to this window while it has the
    /// keyboard. An error when the host is not Wayland or does not offer
    /// the protocol.
    pub fn inhibit(window: &Window) -> anyhow::Result<Self> {
        let display = match window.display_handle().map(|handle| handle.as_raw()) {
            Ok(RawDisplayHandle::Wayland(handle)) => handle.display.as_ptr(),
            _ => anyhow::bail!("the host session is not Wayland"),
        };
        let surface = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Wayland(handle)) => handle.surface.as_ptr(),
            _ => anyhow::bail!("the window is not a Wayland surface"),
        };
        // Safety: both pointers belong to the window, which the winit
        // backend keeps for as long as the compositor runs; the connection
        // made here does not close the display when it is dropped.
        let backend =
            unsafe { wayland_backend::client::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Answer>(&connection)?;
        let handle = queue.handle();
        let manager: Manager = globals.bind(&handle, 1..=1, ()).map_err(|error| {
            anyhow::anyhow!("the host has no keyboard-shortcuts-inhibit: {error}")
        })?;
        let seat: wl_seat::WlSeat = globals
            .bind(&handle, 1..=1, ())
            .map_err(|error| anyhow::anyhow!("the host has no seat: {error}"))?;
        let surface_id = unsafe {
            wayland_backend::client::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.cast(),
            )
        }?;
        let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;
        let inhibitor = manager.inhibit_shortcuts(&surface, &seat, &handle, ());
        let mut answer = Answer::default();
        queue.roundtrip(&mut answer)?;
        Ok(Self {
            queue,
            answer,
            reported: None,
            _inhibitor: inhibitor,
        })
    }

    /// Read what the host answered since the last call. Returns the new
    /// state when it changed.
    pub fn poll(&mut self) -> Option<bool> {
        if let Err(error) = self.queue.dispatch_pending(&mut self.answer) {
            log::debug!("host shortcuts: {error}");
        }
        if self.answer.active == self.reported {
            return None;
        }
        self.reported = self.answer.active;
        self.reported
    }
}

impl Dispatch<Inhibitor, ()> for Answer {
    fn event(
        answer: &mut Self,
        _: &Inhibitor,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => answer.active = Some(true),
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => answer.active = Some(false),
            _ => {}
        }
    }
}

impl Dispatch<Manager, ()> for Answer {
    fn event(
        _: &mut Self,
        _: &Manager,
        _: zwp_keyboard_shortcuts_inhibit_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Answer {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Answer {
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
