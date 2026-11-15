//! `gevrek-testclient --move-pointer STEPS [--extent WxH]`: what a program
//! that moves the pointer does (wlr-virtual-pointer). The steps, one
//! after another with `;` between them: `to=X,Y` (a place on all the
//! displays together, whose size `--extent` says), `by=DX,DY`, `press`,
//! `release`, `click`, `wheel=N` (notches; less than nought is up),
//! `wait=MS`.

use super::{say, word};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct Mover;

/// The left button, as the kernel numbers it.
const LEFT: u32 = 0x110;

pub fn run(connection: &Connection, steps: &str, extent: (u32, u32)) -> bool {
    let (globals, mut queue) = registry_queue_init::<Mover>(connection).expect("registry");
    let handle = queue.handle();
    let Ok(manager) = globals.bind::<ZwlrVirtualPointerManagerV1, _, _>(&handle, 1..=2, ()) else {
        say(
            "error",
            &[("message", word("the compositor has no virtual pointer"))],
        );
        return false;
    };
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&handle, 1..=5, ())
        .ok();
    let pointer = manager.create_virtual_pointer(seat.as_ref(), &handle, ());
    let started = std::time::Instant::now();
    let time = || started.elapsed().as_millis() as u32;
    let pair = |text: &str| -> Option<(f64, f64)> {
        let (x, y) = text.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    };
    for step in steps
        .split(';')
        .map(str::trim)
        .filter(|step| !step.is_empty())
    {
        let (what, value) = step.split_once('=').unwrap_or((step, ""));
        match what {
            "to" => {
                let Some((x, y)) = pair(value) else {
                    return false;
                };
                pointer.motion_absolute(time(), x as u32, y as u32, extent.0, extent.1);
            }
            "by" => {
                let Some((dx, dy)) = pair(value) else {
                    return false;
                };
                pointer.motion(time(), dx, dy);
            }
            "press" => pointer.button(time(), LEFT, wl_pointer::ButtonState::Pressed),
            "release" => pointer.button(time(), LEFT, wl_pointer::ButtonState::Released),
            "click" => {
                pointer.button(time(), LEFT, wl_pointer::ButtonState::Pressed);
                pointer.frame();
                pointer.button(time(), LEFT, wl_pointer::ButtonState::Released);
            }
            "wheel" => {
                let notches: i32 = value.parse().unwrap_or(1);
                pointer.axis_source(wl_pointer::AxisSource::Wheel);
                pointer.axis_discrete(
                    time(),
                    wl_pointer::Axis::VerticalScroll,
                    f64::from(notches) * 15.0,
                    notches,
                );
            }
            "wait" => {
                let _ = queue.roundtrip(&mut Mover);
                std::thread::sleep(std::time::Duration::from_millis(
                    value.parse().unwrap_or(100),
                ));
                continue;
            }
            _ => {
                say("error", &[("message", word("no such step"))]);
                return false;
            }
        }
        pointer.frame();
        if queue.roundtrip(&mut Mover).is_err() {
            return false;
        }
    }
    pointer.destroy();
    let done = queue.roundtrip(&mut Mover).is_ok();
    say("moved", &[]);
    done
}

macro_rules! silent {
    ($($interface:ty),*) => {$(
        impl Dispatch<$interface, ()> for Mover {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}
silent!(
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
    wl_seat::WlSeat
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Mover {
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
