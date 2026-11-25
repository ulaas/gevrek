//! A pointer moved by a program (wlr-virtual-pointer): `wlrctl`, `ydotool`
//! with its Wayland side, remote-desktop helpers and on-screen tools move
//! the pointer, press its buttons and turn its wheel as a mouse does.
//!
//! What such a program sends goes the way a mouse's events go: the
//! pointer moves on the displays, a press focuses and starts a drag where
//! a mouse's would, the wheel runs a binding or scrolls. (As with the
//! keyboard a program can make up, any program of the session may do
//! this: a session's programs are trusted with its input.)

use crate::state::State;
use smithay::backend::input::{Axis, AxisSource};
use smithay::input::pointer::AxisFrame;
use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::server::{
    zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
    zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
};
use smithay::reexports::wayland_server::protocol::wl_pointer;
use smithay::reexports::wayland_server::Resource as _;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum,
};
use std::sync::Mutex;

const VERSION: u32 = 2;

/// The wheel's turn that is being put together until the next `frame`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Turn {
    /// Sideways, then up and down: how far, and in notches of 120 where
    /// the program counts in steps.
    amount: [f64; 2],
    notches: [i32; 2],
    stopped: [bool; 2],
    source: Option<AxisSource>,
    time: u32,
}

impl Turn {
    fn is_empty(&self) -> bool {
        self.amount == [0.0; 2] && self.notches == [0; 2] && self.stopped == [false; 2]
    }
}

/// One pointer a program made.
pub struct Made {
    /// The display its absolute places are on; none: all of them.
    output: Option<String>,
    turn: Mutex<Turn>,
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ZwlrVirtualPointerManagerV1, ()>(VERSION, ());
}

/// Where a place given as a part of an extent is, on an area.
pub(crate) fn place_in(
    area: (i32, i32, i32, i32),
    at: (u32, u32),
    extent: (u32, u32),
) -> (f64, f64) {
    let part = |at: u32, extent: u32| f64::from(at.min(extent)) / f64::from(extent.max(1));
    (
        f64::from(area.0) + part(at.0, extent.0) * f64::from(area.2),
        f64::from(area.1) + part(at.1, extent.1) * f64::from(area.3),
    )
}

fn index_of(axis: WEnum<wl_pointer::Axis>) -> Option<usize> {
    match axis {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Some(0),
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Some(1),
        _ => None,
    }
}

impl State {
    /// The area a made-up pointer's absolute places are on: its display,
    /// or all the displays together.
    fn virtual_pointer_area(&self, output: Option<&str>) -> (i32, i32, i32, i32) {
        let outputs = &self.desktop.outputs;
        if let Some(found) = output.and_then(|name| outputs.iter().find(|known| known.name == name))
        {
            return (
                found.rect.x,
                found.rect.y,
                found.rect.width,
                found.rect.height,
            );
        }
        let left = outputs
            .iter()
            .map(|output| output.rect.x)
            .min()
            .unwrap_or(0);
        let top = outputs
            .iter()
            .map(|output| output.rect.y)
            .min()
            .unwrap_or(0);
        let right = outputs
            .iter()
            .map(|output| output.rect.x + output.rect.width)
            .max()
            .unwrap_or(1);
        let bottom = outputs
            .iter()
            .map(|output| output.rect.y + output.rect.height)
            .max()
            .unwrap_or(1);
        (left, top, right - left, bottom - top)
    }

    /// A wheel's turn a program made up, as a mouse's.
    fn virtual_pointer_turn(&mut self, turn: Turn) {
        if turn.is_empty() {
            return;
        }
        // A binding on the wheel runs instead of scrolling.
        let vertical = if turn.amount[1] != 0.0 {
            turn.amount[1]
        } else {
            f64::from(turn.notches[1])
        };
        if vertical != 0.0 && self.pointer_wheel(vertical < 0.0) {
            return;
        }
        let units = |index: usize| {
            if turn.amount[index] != 0.0 {
                turn.amount[index]
            } else {
                f64::from(turn.notches[index]) * 15.0 / 120.0
            }
        };
        if self.strip_modifier_held()
            && self.strip_wheel(units(0), units(1), turn.source == Some(AxisSource::Finger))
        {
            return;
        }
        let mut frame = AxisFrame::new(turn.time).source(turn.source.unwrap_or(AxisSource::Wheel));
        for (index, axis) in [Axis::Horizontal, Axis::Vertical].into_iter().enumerate() {
            let amount = if turn.amount[index] != 0.0 {
                turn.amount[index]
            } else {
                f64::from(turn.notches[index]) * 15.0 / 120.0
            };
            if amount != 0.0 {
                frame = frame.value(axis, amount);
                if turn.notches[index] != 0 {
                    frame = frame.v120(axis, turn.notches[index]);
                }
            } else if turn.stopped[index] {
                frame = frame.stop(axis);
            }
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }
}

impl GlobalDispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _display: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        log::debug!(
            "{} binds the virtual pointer manager (it may move the pointer and press its buttons)",
            crate::state::named_client(client)
        );
        init.init(resource, ());
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _client: &Client,
        _: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _: &(),
        _display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_virtual_pointer_manager_v1::Request;
        match request {
            Request::CreateVirtualPointer { id, .. } => {
                init.init(
                    id,
                    Made {
                        output: None,
                        turn: Default::default(),
                    },
                );
            }
            Request::CreateVirtualPointerWithOutput { output, id, .. } => {
                let output = output
                    .as_ref()
                    .and_then(smithay::output::Output::from_resource)
                    .map(|output| output.name());
                init.init(
                    id,
                    Made {
                        output,
                        turn: Default::default(),
                    },
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrVirtualPointerV1, Made> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        pointer: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        made: &Made,
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_virtual_pointer_v1::Request;
        let turning = |change: &dyn Fn(&mut Turn)| {
            change(
                &mut made
                    .turn
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
        };
        match request {
            Request::Motion { time, dx, dy } => {
                let by = (dx, dy).into();
                state.pointer_moved_by(by, by, u64::from(time) * 1000);
            }
            Request::MotionAbsolute {
                time,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                let area = state.virtual_pointer_area(made.output.as_deref());
                let place = place_in(area, (x, y), (x_extent, y_extent));
                state.pointer_moved(place.into(), time);
            }
            Request::Button {
                time,
                button,
                state: pressed,
            } => {
                let pressed = pressed == WEnum::Value(wl_pointer::ButtonState::Pressed);
                // Which buttons this pointer holds: let go of when it
                // goes (`destroyed`).
                let id = pointer.id();
                state
                    .virtual_buttons
                    .retain(|held| *held != (id.clone(), button));
                if pressed {
                    state.virtual_buttons.push((id, button));
                }
                state.pointer_button(button, pressed, time);
            }
            Request::Axis { time, axis, value } => {
                if let Some(index) = index_of(axis) {
                    turning(&|turn| {
                        turn.amount[index] += value;
                        turn.time = time;
                    });
                }
            }
            Request::AxisDiscrete {
                time,
                axis,
                value,
                discrete,
            } => {
                if let Some(index) = index_of(axis) {
                    turning(&|turn| {
                        turn.amount[index] += value;
                        // (Never past what a number holds: the program
                        // says how many.)
                        turn.notches[index] =
                            turn.notches[index].saturating_add(discrete.saturating_mul(120));
                        turn.time = time;
                    });
                }
            }
            Request::AxisStop { time, axis } => {
                if let Some(index) = index_of(axis) {
                    turning(&|turn| {
                        turn.stopped[index] = true;
                        turn.time = time;
                    });
                }
            }
            Request::AxisSource { axis_source } => {
                let source = match axis_source {
                    WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
                    WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
                    WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,
                    _ => AxisSource::Wheel,
                };
                turning(&|turn| turn.source = Some(source));
            }
            Request::Frame => {
                let turn = std::mem::take(
                    &mut *made
                        .turn
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                );
                state.virtual_pointer_turn(turn);
            }
            _ => {}
        }
    }

    /// The pointer is gone (its program ended, or let go of it): the
    /// buttons it held are let go. (They stayed held: the keyboard's
    /// window was pinned and a drag went on until a real button was
    /// pressed and let go.)
    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        pointer: &ZwlrVirtualPointerV1,
        _made: &Made,
    ) {
        let id = pointer.id();
        let held: Vec<u32> = state
            .virtual_buttons
            .iter()
            .filter(|(of, _)| *of == id)
            .map(|(_, button)| *button)
            .collect();
        state.virtual_buttons.retain(|(of, _)| *of != id);
        for button in held {
            state.pointer_button(button, false, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_place_is_a_part_of_the_area() {
        // The middle of a display that begins at 1920.
        assert_eq!(
            place_in((1920, 0, 1280, 720), (500, 500), (1000, 1000)),
            (2560.0, 360.0)
        );
        assert_eq!(place_in((0, 0, 800, 600), (0, 0), (100, 100)), (0.0, 0.0));
        // Beyond the extent is its edge; no extent is no division by nought.
        assert_eq!(
            place_in((0, 0, 800, 600), (300, 50), (100, 100)),
            (800.0, 300.0)
        );
        assert_eq!(place_in((10, 20, 800, 600), (5, 5), (0, 0)), (10.0, 20.0));
    }

    #[test]
    fn a_turn_with_nothing_in_it_is_empty() {
        assert!(Turn::default().is_empty());
        let turned = Turn {
            notches: [0, 120],
            ..Default::default()
        };
        assert!(!turned.is_empty());
    }
}
