//! Every key for the studio while a binding's keys are being recorded.
//!
//! The window manager takes its own bindings before a window sees them:
//! pressed while recording, Super+2 would show the second workspace and
//! never reach the studio. On Wayland a window can ask for every key
//! while it has the keyboard (`keyboard-shortcuts-inhibit`, what virtual
//! machine viewers use), and gevrek gives them (Super+Escape takes them
//! back). The toolkit does not speak that protocol, so this asks on the
//! toolkit's own connection, through an event queue of its own.

use eframe::egui_wgpu::wgpu::rwh::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1 as Manager,
    zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1 as Inhibitor,
};

#[derive(Default)]
struct Heard;

/// The request, for as long as it is kept.
pub struct AllKeys {
    _queue: EventQueue<Heard>,
    inhibitor: Inhibitor,
    connection: Connection,
}

impl AllKeys {
    /// Ask for every key for this window. An error when the session is
    /// not Wayland or its compositor does not offer the protocol.
    pub fn take(window: &(impl HasDisplayHandle + HasWindowHandle)) -> anyhow::Result<Self> {
        let display = match window.display_handle().map(|handle| handle.as_raw()) {
            Ok(RawDisplayHandle::Wayland(handle)) => handle.display.as_ptr(),
            _ => anyhow::bail!("the session is not Wayland"),
        };
        let surface = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Wayland(handle)) => handle.surface.as_ptr(),
            _ => anyhow::bail!("the window is not a Wayland surface"),
        };
        // Safety: both pointers belong to the window, which the toolkit
        // keeps for as long as the studio runs; the connection made here
        // does not close the display when it is dropped.
        let backend =
            unsafe { wayland_backend::client::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Heard>(&connection)?;
        let handle = queue.handle();
        let manager: Manager = globals.bind(&handle, 1..=1, ()).map_err(|error| {
            anyhow::anyhow!("the compositor has no keyboard-shortcuts-inhibit: {error}")
        })?;
        let seat: wl_seat::WlSeat = globals
            .bind(&handle, 1..=5, ())
            .map_err(|error| anyhow::anyhow!("there is no seat: {error}"))?;
        let surface_id = unsafe {
            wayland_backend::client::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.cast(),
            )
        }?;
        let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;
        let inhibitor = manager.inhibit_shortcuts(&surface, &seat, &handle, ());
        queue.roundtrip(&mut Heard)?;
        Ok(Self {
            _queue: queue,
            inhibitor,
            connection,
        })
    }
}

impl Drop for AllKeys {
    fn drop(&mut self) {
        // The window manager has its bindings again.
        self.inhibitor.destroy();
        let _ = self.connection.flush();
    }
}

macro_rules! quiet {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Heard {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
quiet!(Inhibitor, ());
quiet!(Manager, ());
quiet!(wl_seat::WlSeat, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);
