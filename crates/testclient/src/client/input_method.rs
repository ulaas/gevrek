//! `gevrek-testclient --input-method TEXT [--candidates RRGGBB]`: what an
//! input method does (input-method v2). When a text field has the
//! keyboard it is told so (`activate`), and it hands the field `TEXT`,
//! first as a text that is still being made (the pre-edit) and then for
//! good. With `--candidates` it has a little window of that colour, as
//! the list of candidates is one, which the compositor places at the
//! field's text cursor (`rectangle` says where that is).
//!
//! Lines printed: `ready`, `unavailable` (another input method is there
//! already), `activate`, `deactivate`, `committed`, `rectangle`.

use super::{say, word};
use std::os::fd::AsFd;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
    zwp_input_popup_surface_v2::{self, ZwpInputPopupSurfaceV2},
};

/// The candidates' window: this wide and high.
const POPUP: (i32, i32) = (120, 30);

struct Method {
    text: String,
    /// A field has the keyboard, as far as was said since the last `done`.
    pending: bool,
    active: bool,
    /// How many times `done` was said: what a commit answers to.
    serial: u32,
    given: bool,
    gone: bool,
}

pub fn run(connection: &Connection, text: String, candidates: Option<[u8; 3]>) -> bool {
    let (globals, mut queue) = registry_queue_init::<Method>(connection).expect("registry");
    let handle = queue.handle();
    let (Ok(seat), Ok(manager)) = (
        globals.bind::<wl_seat::WlSeat, _, _>(&handle, 1..=5, ()),
        globals.bind::<ZwpInputMethodManagerV2, _, _>(&handle, 1..=1, ()),
    ) else {
        say(
            "error",
            &[("message", word("the compositor has no input methods"))],
        );
        return false;
    };
    let method = manager.get_input_method(&seat, &handle, ());
    let mut state = Method {
        text,
        pending: false,
        active: false,
        serial: 0,
        given: false,
        gone: false,
    };
    // The candidates' window: one colour, from shared memory.
    let _popup = candidates.and_then(|colour| {
        let compositor = globals
            .bind::<wl_compositor::WlCompositor, _, _>(&handle, 1..=4, ())
            .ok()?;
        let shm = globals
            .bind::<wl_shm::WlShm, _, _>(&handle, 1..=1, ())
            .ok()?;
        let surface = compositor.create_surface(&handle, ());
        let popup = method.get_input_popup_surface(&surface, &handle, ());
        let bytes = (POPUP.0 * POPUP.1 * 4) as usize;
        let file = memory(bytes)?;
        {
            use std::io::Write;
            let mut pixels = Vec::with_capacity(bytes);
            for _ in 0..POPUP.0 * POPUP.1 {
                pixels.extend_from_slice(&[colour[2], colour[1], colour[0], 255]);
            }
            (&file).write_all(&pixels).ok()?;
        }
        let pool = shm.create_pool(file.as_fd(), bytes as i32, &handle, ());
        let buffer = pool.create_buffer(
            0,
            POPUP.0,
            POPUP.1,
            POPUP.0 * 4,
            wl_shm::Format::Xrgb8888,
            &handle,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, POPUP.0, POPUP.1);
        surface.commit();
        Some((surface, popup, pool, buffer, file))
    });
    if queue.roundtrip(&mut state).is_err() {
        return false;
    }
    if state.gone {
        say("unavailable", &[]);
        return false;
    }
    say("ready", &[]);
    while !state.gone {
        if queue.blocking_dispatch(&mut state).is_err() {
            return false;
        }
        // A field took the keyboard: its text, once.
        if state.active && !state.given {
            state.given = true;
            method.set_preedit_string(state.text.clone(), 0, 0);
            method.commit(state.serial);
            method.commit_string(state.text.clone());
            method.commit(state.serial);
            say("committed", &[("text", word(&state.text))]);
        }
        if !state.active {
            state.given = false;
        }
    }
    true
}

/// A file in memory of this many bytes.
pub(super) fn memory(bytes: usize) -> Option<std::fs::File> {
    use std::os::fd::FromRawFd;
    // SAFETY: a name that ends in a zero byte; the descriptor that comes
    // back is this function's own.
    let descriptor =
        unsafe { libc::memfd_create(c"gevrek-testclient".as_ptr(), libc::MFD_CLOEXEC) };
    if descriptor < 0 {
        return None;
    }
    // SAFETY: just made, owned by nothing else.
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    file.set_len(bytes as u64).ok()?;
    Some(file)
}

impl Dispatch<ZwpInputMethodV2, ()> for Method {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.pending = true,
            zwp_input_method_v2::Event::Deactivate => state.pending = false,
            zwp_input_method_v2::Event::Done => {
                state.serial += 1;
                if state.pending != state.active {
                    state.active = state.pending;
                    say(
                        if state.active {
                            "activate"
                        } else {
                            "deactivate"
                        },
                        &[],
                    );
                }
            }
            zwp_input_method_v2::Event::Unavailable => state.gone = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputPopupSurfaceV2, ()> for Method {
    fn event(
        _: &mut Self,
        _: &ZwpInputPopupSurfaceV2,
        event: zwp_input_popup_surface_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_popup_surface_v2::Event::TextInputRectangle {
            x,
            y,
            width,
            height,
        } = event
        {
            say(
                "rectangle",
                &[
                    ("x", x.to_string()),
                    ("y", y.to_string()),
                    ("width", width.to_string()),
                    ("height", height.to_string()),
                ],
            );
        }
    }
}

macro_rules! quiet {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Method {
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
quiet!(ZwpInputMethodManagerV2, ());
quiet!(wl_seat::WlSeat, ());
quiet!(wl_compositor::WlCompositor, ());
quiet!(wl_surface::WlSurface, ());
quiet!(wl_shm::WlShm, ());
quiet!(wl_shm_pool::WlShmPool, ());
quiet!(wl_buffer::WlBuffer, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);
