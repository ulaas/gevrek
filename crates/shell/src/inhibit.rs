//! Keeping the screen awake for programs that ask through the portal.
//!
//! A program that plays a video asks its window's surface to keep the
//! screen awake, and the compositor follows that. A sandboxed program
//! asks the desktop instead (the Inhibit portal): then the shell holds an
//! inhibitor of its own with the compositor, on a surface that is never
//! shown, for as long as anyone asks. On a connection and a thread of its
//! own.

use smithay_client_toolkit::reexports::calloop::{self, EventLoop};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};

/// Whether the screen is to be kept awake, by who asks: each asker is
/// known by a name of its own, and the screen stays awake while there is
/// one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Askers(std::collections::BTreeSet<String>);

impl Askers {
    /// Someone asks, or stops asking. Whether that changed if the screen
    /// is kept awake, and to what.
    pub fn set(&mut self, asker: &str, asks: bool) -> Option<bool> {
        let before = !self.0.is_empty();
        if asks {
            self.0.insert(asker.to_string());
        } else {
            self.0.remove(asker);
        }
        let now = !self.0.is_empty();
        (now != before).then_some(now)
    }
}

/// The keeper, as the shell holds it.
pub struct Awake {
    askers: Askers,
    tell: calloop::channel::Sender<bool>,
}

impl Awake {
    /// Someone asks for the screen to be kept awake, or stops asking.
    pub fn set(&mut self, asker: &str, asks: bool) {
        if let Some(awake) = self.askers.set(asker, asks) {
            log::debug!(
                "the screen is {} awake ({asker} asked)",
                if awake { "kept" } else { "no longer kept" }
            );
            if let Err(error) = self.tell.send(awake) {
                log::debug!("the keeper of the screen's wakefulness is gone: {error}");
            }
        }
    }
}

struct Holder {
    manager: ZwpIdleInhibitManagerV1,
    surface: wl_surface::WlSurface,
    held: Option<ZwpIdleInhibitorV1>,
    handle: QueueHandle<Holder>,
}

/// Start the thread that holds the inhibitor. An error where the
/// compositor has no way to be asked.
pub fn start() -> anyhow::Result<Awake> {
    let connection = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<Holder>(&connection)?;
    let handle = queue.handle();
    let manager: ZwpIdleInhibitManagerV1 = globals
        .bind(&handle, 1..=1, ())
        .map_err(|_| anyhow::anyhow!("the compositor has no idle inhibitors"))?;
    let compositor: wl_compositor::WlCompositor = globals
        .bind(&handle, 1..=6, ())
        .map_err(|_| anyhow::anyhow!("there is no compositor to make a surface with"))?;
    let surface = compositor.create_surface(&handle, ());
    let (tell, told) = calloop::channel::channel::<bool>();
    std::thread::Builder::new()
        .name("gevrek-awake".to_string())
        .spawn(move || {
            let mut event_loop = match EventLoop::<Holder>::try_new() {
                Ok(event_loop) => event_loop,
                Err(error) => {
                    log::warn!("the screen cannot be kept awake for the portal: {error}");
                    return;
                }
            };
            let mut holder = Holder {
                manager,
                surface,
                held: None,
                handle,
            };
            if let Err(error) = WaylandSource::new(connection.clone(), queue).insert(event_loop.handle()) {
                log::warn!("the screen cannot be kept awake for the portal: {error}");
                return;
            }
            let _ = event_loop
                .handle()
                .insert_source(told, move |event, _, holder| {
                    if let calloop::channel::Event::Msg(awake) = event {
                        match (awake, holder.held.take()) {
                            (true, None) => {
                                holder.held = Some(holder.manager.create_inhibitor(
                                    &holder.surface,
                                    &holder.handle,
                                    (),
                                ));
                            }
                            (true, held) => holder.held = held,
                            (false, Some(held)) => held.destroy(),
                            (false, None) => {}
                        }
                        let _ = connection.flush();
                    }
                });
            if let Err(error) = event_loop.run(None, &mut holder, |_| {}) {
                log::warn!("the screen is no longer kept awake for the portal: {error}");
            }
        })?;
    Ok(Awake {
        askers: Askers::default(),
        tell,
    })
}

macro_rules! quiet {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Holder {
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
quiet!(ZwpIdleInhibitManagerV1, ());
quiet!(ZwpIdleInhibitorV1, ());
quiet!(wl_compositor::WlCompositor, ());
quiet!(wl_surface::WlSurface, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_is_kept_awake_while_anyone_asks() {
        let mut askers = Askers::default();
        assert_eq!(askers.set("/request/1", true), Some(true));
        // A second asker changes nothing; nor does the first asking twice.
        assert_eq!(askers.set("/request/2", true), None);
        assert_eq!(askers.set("/request/1", true), None);
        assert_eq!(askers.set("/request/1", false), None);
        // The last one stops asking.
        assert_eq!(askers.set("/request/2", false), Some(false));
        // One that never asked.
        assert_eq!(askers.set("/request/9", false), None);
    }
}
