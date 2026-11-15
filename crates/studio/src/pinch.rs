//! Two fingers moved apart or together on the touchpad, for the studio
//! (Wayland).
//!
//! The toolkit does not hear a touchpad's pinch on Wayland (it does on
//! other systems, where the graph is zoomed by it already). The
//! compositor tells of one through `pointer-gestures`; this listens to
//! that on the toolkit's own connection, through an event queue of its
//! own that a thread waits at, and hands what it heard to the toolkit's
//! input as the zoom it would have been (`Studio::raw_input_hook`).

use eframe::egui_wgpu::wgpu::rwh::{HasDisplayHandle, RawDisplayHandle};
use std::sync::{Arc, Mutex};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1 as PinchGesture},
    zwp_pointer_gestures_v1::ZwpPointerGesturesV1 as Gestures,
};

/// How far apart the fingers went since it was last asked, as a factor
/// (1: not at all), kept between the thread that hears and the frame.
#[derive(Clone)]
pub struct Pinch {
    factor: Arc<Mutex<f64>>,
}

struct Heard {
    /// How far apart the fingers were when last told (1 as they began).
    scale: f64,
    factor: Arc<Mutex<f64>>,
    /// Called when there is something to take.
    wake: Box<dyn Fn() + Send>,
}

/// The factor after the fingers went from one distance to another.
pub fn grown(factor: f64, from: f64, to: f64) -> f64 {
    if from > 0.0 && to > 0.0 {
        factor * (to / from)
    } else {
        factor
    }
}

impl Pinch {
    /// Listen for pinches told to this window's program. An error when
    /// the session is not Wayland or its compositor does not tell of
    /// gestures. `wake` is called (on another thread) when one moved.
    pub fn listen(
        window: &impl HasDisplayHandle,
        wake: impl Fn() + Send + 'static,
    ) -> anyhow::Result<Self> {
        let display = match window.display_handle().map(|handle| handle.as_raw()) {
            Ok(RawDisplayHandle::Wayland(handle)) => handle.display.as_ptr(),
            _ => anyhow::bail!("the session is not Wayland"),
        };
        // Safety: the pointer belongs to the window, which the toolkit
        // keeps for as long as the studio runs; the connection made here
        // does not close the display when it is dropped.
        let backend =
            unsafe { wayland_backend::client::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Heard>(&connection)?;
        let handle = queue.handle();
        let gestures: Gestures = globals
            .bind(&handle, 1..=3, ())
            .map_err(|error| anyhow::anyhow!("the compositor tells of no gestures: {error}"))?;
        let seat: wl_seat::WlSeat = globals
            .bind(&handle, 1..=5, ())
            .map_err(|error| anyhow::anyhow!("there is no seat: {error}"))?;
        let pointer = seat.get_pointer(&handle, ());
        let _pinch = gestures.get_pinch_gesture(&pointer, &handle, ());
        let factor = Arc::new(Mutex::new(1.0));
        let mut heard = Heard {
            scale: 1.0,
            factor: Arc::clone(&factor),
            wake: Box::new(wake),
        };
        queue.roundtrip(&mut heard)?;
        std::thread::Builder::new()
            .name("pinch".to_string())
            .spawn(move || {
                // (Ends with the connection, which is the window's.)
                while queue.blocking_dispatch(&mut heard).is_ok() {}
            })?;
        Ok(Self { factor })
    }

    /// The factor since it was last asked; 1 when the fingers did not move.
    pub fn take(&self) -> f32 {
        let mut factor = self
            .factor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::replace(&mut *factor, 1.0) as f32
    }
}

impl Dispatch<PinchGesture, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &PinchGesture,
        event: zwp_pointer_gesture_pinch_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin { .. } => heard.scale = 1.0,
            zwp_pointer_gesture_pinch_v1::Event::Update { scale, .. } => {
                {
                    let mut factor = heard
                        .factor
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *factor = grown(*factor, heard.scale, scale);
                }
                if scale > 0.0 {
                    heard.scale = scale;
                }
                (heard.wake)();
            }
            _ => {}
        }
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
quiet!(Gestures, ());
quiet!(wl_seat::WlSeat, ());
quiet!(wl_pointer::WlPointer, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_follows_the_fingers_from_one_telling_to_the_next() {
        // Apart to one and a half, in two tellings: one and a half in all.
        let factor = grown(grown(1.0, 1.0, 1.2), 1.2, 1.5);
        assert!((factor - 1.5).abs() < 1e-9, "{factor}");
        // Together again: back to one.
        assert!((grown(factor, 1.5, 1.0) - 1.0).abs() < 1e-9);
        // Fingers that do not move, and a distance that means nothing.
        assert_eq!(grown(1.0, 1.0, 1.0), 1.0);
        assert_eq!(grown(1.3, 1.0, 0.0), 1.3);
    }
}
