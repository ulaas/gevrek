//! Input from any Smithay input backend (the nested window, libinput):
//! the pointer and the keyboard of the seat.

use crate::state::State;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::input::HotkeyEvent;
use gevrek_engine::WindowHandle;
use gevrek_model::settings::{BindingDevice, BindingTrigger};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
    GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::WindowSurfaceType;
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent as GesturePinchUpdateEventOut,
    GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent as GestureSwipeUpdateEventOut, MotionEvent,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::shell::wlr_layer::Layer;

/// `BTN_LEFT` of linux/input-event-codes.h.
const BUTTON_LEFT: u32 = 0x110;
/// `BTN_RIGHT`.
const BUTTON_RIGHT: u32 = 0x111;
/// A held key starts repeating its binding after this long, then at this rate.
const REPEAT_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
const REPEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);

/// The name bindings use for a mouse button (as on Windows).
fn button_name(button: u32) -> Option<&'static str> {
    Some(match button {
        0x110 => "LEFT",
        0x111 => "RIGHT",
        0x112 => "MIDDLE",
        0x113 => "X1",
        0x114 => "X2",
        _ => return None,
    })
}

/// A window is not dragged smaller than this.
const MIN_DRAG_SIZE: i32 = 80;

/// The modifiers that, held alone, make the left button move a window and
/// the right button resize it: Super, or what `--drag-modifier` names
/// (`GEVREK_WL_DRAG_MODIFIER`: `super`, `alt`, `ctrl`, several with commas).
/// Nested in a session that uses Super with the mouse for its own windows,
/// another one is needed.
pub fn drag_modifiers(text: &str) -> Option<Vec<&'static str>> {
    let mut names = Vec::new();
    for name in text.split(',') {
        names.push(match name.trim().to_ascii_lowercase().as_str() {
            "super" | "win" | "meta" | "logo" => "super",
            "alt" => "alt",
            "ctrl" | "control" => "ctrl",
            _ => return None,
        });
    }
    (!names.is_empty()).then_some(names)
}

/// The modifiers held with a key or a button: Ctrl, Alt, Shift, Super.
pub type Chord = (bool, bool, bool, bool);

/// The keys (or buttons) that are held, each with the modifiers that
/// were down when it went down and the bindings on its release that were
/// live then.
pub type Strokes = std::collections::HashMap<u32, (Chord, Vec<i32>)>;

/// What a key or button stroke is looked up with: its modifiers, and the
/// bindings its release runs. `now` and `on_release_now` are those of
/// this moment. Going down: they are remembered for the key. Coming up:
/// what was remembered when it went down is given back and the key is
/// forgotten, so the release is the release of the stroke that was
/// pressed, with the bindings that were live then (as on Windows,
/// `hotkey.rs`, which keeps the press and what it matched). Where the
/// press was never seen (the key was down before gevrek listened), this
/// moment's count.
pub fn stroke(
    held: &mut Strokes,
    code: u32,
    pressed: bool,
    now: Chord,
    on_release_now: Vec<i32>,
) -> (Chord, Vec<i32>) {
    if pressed {
        held.insert(code, (now, on_release_now.clone()));
        (now, on_release_now)
    } else {
        held.remove(&code).unwrap_or((now, on_release_now))
    }
}

fn drag_modifier_held(state: &smithay::input::keyboard::ModifiersState) -> bool {
    static WANTED: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    let wanted = WANTED.get_or_init(|| {
        std::env::var("GEVREK_WL_DRAG_MODIFIER")
            .ok()
            .and_then(|text| drag_modifiers(&text))
            .unwrap_or_else(|| vec!["super"])
    });
    // Exactly one of them, and nothing else: Ctrl+Alt+click stays the
    // client's.
    let held = (state.logo, state.alt, state.ctrl);
    !state.shift
        && wanted.iter().any(|name| match *name {
            "super" => held == (true, false, false),
            "alt" => held == (false, true, false),
            _ => held == (false, false, true),
        })
}

/// What is under a point: a surface, and where that surface is.
pub type Under = (WlSurface, Point<f64, Logical>);

impl State {
    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>) {
        self.activity();
        match event {
            InputEvent::Keyboard { event } => {
                self.key_event(event.key_code(), event.state(), Event::time_msec(&event));
            }
            InputEvent::PointerMotionAbsolute { event } => {
                // The device covers the first output (the nested window).
                let Some((output, transform)) = self
                    .desktop
                    .outputs
                    .first()
                    .map(|output| (output.rect, output.transform))
                else {
                    return;
                };
                // (The device says where on the display as it lies; the
                // picture may be turned on it.)
                let (across, down) = crate::desktop::turned_place(
                    transform,
                    (
                        event.x_transformed(1_000_000) / 1e6,
                        event.y_transformed(1_000_000) / 1e6,
                    ),
                );
                let x = across * f64::from(output.width) + f64::from(output.x);
                let y = down * f64::from(output.height) + f64::from(output.y);
                // The host may report a place on the window's frame, just
                // outside the output: the pointer stays on the outputs.
                let location = self.clamp_to_outputs((x, y).into());
                self.pointer_moved(location, Event::time_msec(&event));
            }
            InputEvent::PointerMotion { event } => {
                self.pointer_moved_by(event.delta(), event.delta_unaccel(), Event::time(&event));
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(
                    event.button_code(),
                    event.state() == ButtonState::Pressed,
                    Event::time_msec(&event),
                );
            }
            InputEvent::PointerAxis { event } => {
                // A binding on the wheel runs instead of scrolling.
                let turned = event
                    .amount(Axis::Vertical)
                    .or_else(|| event.amount_v120(Axis::Vertical))
                    .unwrap_or(0.0);
                // With the modifier of the mouse held, over a strip: its
                // view moves. Sideways before any binding (bindings are
                // on the wheel's own direction), else when none took it.
                let units = |axis| {
                    event
                        .amount(axis)
                        .or_else(|| event.amount_v120(axis).map(|steps| steps * 15.0 / 120.0))
                        .unwrap_or(0.0)
                };
                let (sideways, along) = (units(Axis::Horizontal), units(Axis::Vertical));
                let fingers = event.source() == AxisSource::Finger;
                let held = self.strip_modifier_held();
                if held && sideways.abs() > along.abs() && self.strip_wheel(sideways, 0.0, fingers)
                {
                    return;
                }
                if turned != 0.0 && self.pointer_wheel(turned < 0.0) {
                    return;
                }
                if held && self.strip_wheel(sideways, along, fingers) {
                    return;
                }
                let mut frame = AxisFrame::new(Event::time_msec(&event)).source(event.source());
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let steps = event.amount_v120(axis);
                    let amount = event
                        .amount(axis)
                        .unwrap_or_else(|| steps.unwrap_or(0.0) * 15.0 / 120.0);
                    if amount != 0.0 {
                        frame = frame
                            .relative_direction(axis, event.relative_direction(axis))
                            .value(axis, amount);
                        if let Some(steps) = steps {
                            frame = frame.v120(axis, steps as i32);
                        }
                    } else if event.source() == AxisSource::Finger
                        && event.amount(axis) == Some(0.0)
                    {
                        frame = frame.stop(axis);
                    }
                }
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.axis(self, frame);
                    pointer.frame(self);
                }
            }
            // Touchpad swipes (only a backend with real devices sends them).
            // A swipe with as many fingers as one of the configuration's
            // gestures asks for is gevrek's; any other is the program's
            // under the pointer (pointer-gestures), as pinches and holds
            // always are.
            // Fingers on a touch screen. (The screen is taken to be the
            // first output's: which screen a touch device belongs to is
            // not asked of the system yet.)
            InputEvent::TouchDown { event } => {
                use smithay::backend::input::{AbsolutePositionEvent, TouchEvent};
                if let Some(at) = self.touch_place(
                    event.x_transformed(1_000_000) / 1e6,
                    event.y_transformed(1_000_000) / 1e6,
                ) {
                    self.touch_down_slot(event.slot(), at, Event::time_msec(&event));
                }
            }
            InputEvent::TouchMotion { event } => {
                use smithay::backend::input::{AbsolutePositionEvent, TouchEvent};
                if let Some(at) = self.touch_place(
                    event.x_transformed(1_000_000) / 1e6,
                    event.y_transformed(1_000_000) / 1e6,
                ) {
                    self.touch_motion_slot(event.slot(), at, Event::time_msec(&event));
                }
            }
            InputEvent::TouchUp { event } => {
                use smithay::backend::input::TouchEvent;
                self.touch_up_slot(event.slot(), Event::time_msec(&event));
            }
            InputEvent::TouchFrame { .. } => self.touch_frame(),
            InputEvent::TouchCancel { .. } => self.touch_cancel(),
            InputEvent::GestureSwipeBegin { event } => {
                self.gestures_available = true;
                let fingers = GestureBeginEvent::fingers(&event);
                self.swipe_begin(fingers);
                self.swipe_forwarded = !self.swipe_is_ours(fingers);
                if let (true, Some(pointer)) = (self.swipe_forwarded, self.seat.get_pointer()) {
                    pointer.gesture_swipe_begin(
                        self,
                        &GestureSwipeBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            fingers,
                        },
                    );
                }
            }
            InputEvent::GestureSwipeUpdate { event } => {
                let (dx, dy) = (
                    GestureSwipeUpdateEvent::delta_x(&event),
                    GestureSwipeUpdateEvent::delta_y(&event),
                );
                self.swipe_update(dx, dy);
                if let (true, Some(pointer)) = (self.swipe_forwarded, self.seat.get_pointer()) {
                    pointer.gesture_swipe_update(
                        self,
                        &GestureSwipeUpdateEventOut {
                            time: Event::time_msec(&event),
                            delta: (dx, dy).into(),
                        },
                    );
                }
            }
            InputEvent::GestureSwipeEnd { event } => {
                let cancelled = GestureEndEvent::cancelled(&event);
                self.swipe_end(cancelled);
                if let (true, Some(pointer)) = (
                    std::mem::take(&mut self.swipe_forwarded),
                    self.seat.get_pointer(),
                ) {
                    pointer.gesture_swipe_end(
                        self,
                        &GestureSwipeEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            cancelled,
                        },
                    );
                }
            }
            InputEvent::GesturePinchBegin { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.gesture_pinch_begin(
                        self,
                        &GesturePinchBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            fingers: GestureBeginEvent::fingers(&event),
                        },
                    );
                }
            }
            InputEvent::GesturePinchUpdate { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.gesture_pinch_update(
                        self,
                        &GesturePinchUpdateEventOut {
                            time: Event::time_msec(&event),
                            delta: (
                                GesturePinchUpdateEvent::delta_x(&event),
                                GesturePinchUpdateEvent::delta_y(&event),
                            )
                                .into(),
                            scale: GesturePinchUpdateEvent::scale(&event),
                            rotation: GesturePinchUpdateEvent::rotation(&event),
                        },
                    );
                }
            }
            InputEvent::GesturePinchEnd { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.gesture_pinch_end(
                        self,
                        &GesturePinchEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            cancelled: GestureEndEvent::cancelled(&event),
                        },
                    );
                }
            }
            InputEvent::GestureHoldBegin { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.gesture_hold_begin(
                        self,
                        &GestureHoldBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            fingers: GestureBeginEvent::fingers(&event),
                        },
                    );
                }
            }
            InputEvent::GestureHoldEnd { event } => {
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.gesture_hold_end(
                        self,
                        &GestureHoldEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time_msec(&event),
                            cancelled: GestureEndEvent::cancelled(&event),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    /// A pointer button went down or up. With the Super key held, the left
    /// button moves the window under the pointer and the right button
    /// resizes it; otherwise a press focuses what is under the pointer and
    /// the button goes to the client.
    pub fn pointer_button(&mut self, button: u32, pressed: bool, time: u32) {
        // The end of a move or resize: the button that started it is not
        // the client's.
        if !pressed && self.drag.as_ref().is_some_and(|drag| drag.button == button) {
            if button == BUTTON_LEFT {
                self.desktop.primary_button_down = false;
            }
            self.end_drag();
            return;
        }
        // Locked: no bindings, no drags, no focus changes: the button is
        // the lock screen's.
        if self.locked {
            if pressed {
                self.focus_lock_surface();
            }
            if let Some(pointer) = self.seat.get_pointer() {
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                        button,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                    },
                );
                pointer.frame(self);
            }
            return;
        }
        // A window is being picked: the press names the window under the
        // pointer, and neither it nor its release reaches a program.
        if pressed && self.picking.is_some() {
            self.pick_press(button);
            return;
        }
        if !pressed && self.pick_button == Some(button) {
            self.pick_button = None;
            return;
        }
        // The overview of every workspace is up: a press chooses in it, and
        // neither it nor its release reaches a program.
        if pressed && self.workspace_overview.is_some() {
            let (x, y) = self.desktop.pointer;
            self.overview_button = Some(button);
            self.workspace_overview_press(x, y);
            return;
        }
        if !pressed && self.overview_button == Some(button) {
            self.overview_button = None;
            return;
        }
        // A menu is open: a press on anything of its own program is that
        // program's to deal with; a press anywhere else closes the menu and
        // goes no further.
        if let Some(mut grab) = self.popup_grab.take().filter(|grab| !grab.has_ended()) {
            use smithay::reexports::wayland_server::Resource;
            use smithay::wayland::seat::WaylandFocus;
            let (x, y) = self.desktop.pointer;
            let under = self
                .surface_under((f64::from(x), f64::from(y)).into())
                .1
                .map(|(surface, _)| surface);
            let menu = grab
                .current_grab()
                .and_then(|target| target.wl_surface().map(|surface| surface.into_owned()));
            let its_own = match (&under, &menu) {
                (Some(under), Some(menu)) => under.id().same_client_as(&menu.id()),
                _ => false,
            };
            if its_own || !pressed {
                self.popup_grab = Some(grab);
                if let Some(pointer) = self.seat.get_pointer() {
                    pointer.button(
                        self,
                        &ButtonEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time,
                            button,
                            state: if pressed {
                                ButtonState::Pressed
                            } else {
                                ButtonState::Released
                            },
                        },
                    );
                    pointer.frame(self);
                }
                return;
            }
            grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(pointer) = self.seat.get_pointer() {
                pointer.unset_grab(self, serial, time);
            }
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.unset_grab(self);
            }
            // The release of this press is not for anybody either, and the
            // keyboard goes back to the window that has the focus.
            self.suppressed_buttons.insert(button);
            self.desktop.focus_dirty = true;
            self.settle();
            return;
        }
        let logo = self
            .seat
            .get_keyboard()
            .is_some_and(|keyboard| drag_modifier_held(&keyboard.modifier_state()));
        if pressed
            && logo
            && (button == BUTTON_LEFT || button == BUTTON_RIGHT)
            && self.drag.is_none()
        {
            let (x, y) = self.desktop.pointer;
            let location = (f64::from(x), f64::from(y)).into();
            let on_layer = self
                .layer_surface_under(location, &[Layer::Overlay, Layer::Top])
                .is_some();
            if let (false, Some(id)) = (on_layer, self.desktop.window_at(x, y)) {
                if button == BUTTON_LEFT {
                    self.desktop.primary_button_down = true;
                }
                self.focus_window(id);
                // The right button resizes from the corner nearest to the
                // pointer.
                let edges = (button == BUTTON_RIGHT).then(|| {
                    let rect = self.desktop.windows[&id].rect;
                    (x < rect.x + rect.width / 2, y < rect.y + rect.height / 2)
                });
                self.begin_drag(id, edges, button);
                return;
            }
        }
        // A binding on a mouse button runs first; one that consumes the
        // button keeps press and release from the client.
        if let Some(name) = button_name(button) {
            // A button that comes up is the stroke it was when it went
            // down: its release runs the bindings that were live then,
            // for the modifiers held then (as a key's does).
            let now = self.modifiers();
            let (on_release_now, _) =
                self.matching_bindings(BindingDevice::Mouse, name, now, BindingTrigger::Release);
            let (chord, on_release) = stroke(
                &mut self.buttons_pressed_with,
                button,
                pressed,
                now,
                on_release_now,
            );
            if pressed {
                self.hold(button, BindingDevice::Mouse, name);
                let (ids, consume) =
                    self.matching_bindings(BindingDevice::Mouse, name, chord, BindingTrigger::Press);
                self.run_mouse_bindings(name, ids, chord, BindingTrigger::Press);
                if consume {
                    self.suppressed_buttons.insert(button);
                    return;
                }
            } else {
                self.release_held(button);
                let ids = if self.paused { Vec::new() } else { on_release };
                self.run_mouse_bindings(name, ids, chord, BindingTrigger::Release);
                // (Kept from the program as its press was.)
                if self.suppressed_buttons.remove(&button) {
                    return;
                }
            }
        }
        if button == BUTTON_LEFT {
            self.desktop.primary_button_down = pressed;
        }
        if pressed {
            // A click gives the window under the pointer the focus,
            // unless a bar or a launcher is in front of it.
            let (x, y) = self.desktop.pointer;
            let location = (f64::from(x), f64::from(y)).into();
            self.click_layer_focus(location);
            let on_layer = self
                .layer_surface_under(location, &[Layer::Overlay, Layer::Top])
                .is_some();
            // A click on a tab of a stackbar shows that window.
            let tab = if on_layer || button != BUTTON_LEFT {
                None
            } else {
                self.desktop.stackbar_tab_at(x, y)
            };
            if let Some((key, index)) = tab {
                if let Err(error) = self.core.executor.focus_stackbar_tab(
                    &mut self.desktop,
                    &mut self.core.monitor_manager,
                    key,
                    index,
                ) {
                    log::warn!("stackbar tab: {error}");
                }
                self.settle();
                return;
            }
            if !on_layer {
                // (The shell hears it: what it has open beside the
                // windows without holding the keyboard is put away.)
                self.emit(
                    gevrek_engine::executor::RuntimeEventKind::DesktopPressed,
                    None,
                    Vec::new(),
                );
                if self.x11_own_at(location) {
                    // A press on an X11 program's menu (or its tooltip,
                    // its notification): that program's alone. The window
                    // that lies under it is not given the keyboard: the
                    // program whose menu it is would see its window lose
                    // it, and close the menu before the press was told.
                } else if let Some(id) = self.surface_under(location).0 {
                    // (The window whose menu is pressed, where a menu
                    // reaches over another window.)
                    self.focus_window(id);
                } else {
                    // A press on the bare desktop of another display: the
                    // keyboard goes there, so that what is asked for next
                    // (a layout, a workspace, a new window's place) is that
                    // display's and not the one the last window was on.
                    let here = self.desktop.output_at(x, y);
                    if here.is_some() && here != self.core.executor.focused_monitor() {
                        if let Err(error) = self.run_command(
                            gevrek_model::command::DispatchCommand::FocusMonitorAtCursor,
                        ) {
                            log::warn!("focus the display under the pointer: {error}");
                        }
                    }
                }
            }
            self.settle();
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.button(
                self,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    button,
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                },
            );
            pointer.frame(self);
        }
        // (An X11 program's drag that was let go over a Wayland window:
        // the X11 program is told of the button now.)
        self.x_drag_release_owed();
    }

    /// The wheel turned one step. Returns whether a binding took it.
    /// Whether the modifier that makes the mouse gevrek's (moving and
    /// resizing windows with its buttons, moving a strip's view with its
    /// wheel) is held.
    pub fn strip_modifier_held(&self) -> bool {
        self.seat.get_keyboard().is_some_and(|keyboard| {
            // (Shift may be held with it: where the wheel with the
            // modifier alone is bound, as to the workspaces, the wheel
            // with Shift too is the strip's.)
            let mut state = keyboard.modifier_state();
            state.shift = false;
            drag_modifier_held(&state)
        })
    }

    pub fn pointer_wheel(&mut self, up: bool) -> bool {
        self.mouse_binding(
            if up { "WHEEL_UP" } else { "WHEEL_DOWN" },
            BindingTrigger::Press,
        )
    }

    /// One step of a wheel for the program under the pointer, as a mouse's
    /// wheel sends it (fifteen units, one notch): what a made-up step is
    /// when no binding took it.
    pub fn wheel_step_to_client(&mut self, up: bool, time: u32) {
        let (amount, notch) = if up { (-15.0, -120) } else { (15.0, 120) };
        let frame = AxisFrame::new(time)
            .source(AxisSource::Wheel)
            .value(Axis::Vertical, amount)
            .v120(Axis::Vertical, notch);
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }

    fn modifiers(&self) -> (bool, bool, bool, bool) {
        self.seat
            .get_keyboard()
            .map_or((false, false, false, false), |keyboard| {
                let state = keyboard.modifier_state();
                (state.ctrl, state.alt, state.shift, state.logo)
            })
    }

    /// Run the bindings on a mouse button or wheel step. Returns whether
    /// one of them keeps the input from the client.
    fn mouse_binding(&mut self, name: &str, trigger: BindingTrigger) -> bool {
        let modifiers = self.modifiers();
        let (ids, consume) = self.matching_bindings(BindingDevice::Mouse, name, modifiers, trigger);
        self.run_mouse_bindings(name, ids, modifiers, trigger);
        consume
    }

    /// Run these bindings of a mouse button or wheel step, as hit with
    /// these modifiers.
    fn run_mouse_bindings(
        &mut self,
        name: &str,
        ids: Vec<i32>,
        modifiers: Chord,
        trigger: BindingTrigger,
    ) {
        if !ids.is_empty() {
            self.bindings_triggered(
                ids.clone(),
                HotkeyEvent {
                    registration_ids: ids,
                    key: name.to_string(),
                    ctrl: modifiers.0,
                    alt: modifiers.1,
                    shift: modifiers.2,
                    win: modifiers.3,
                    device: BindingDevice::Mouse,
                    trigger,
                    held_ms: 0,
                },
            );
        }
    }

    /// Remember a key or button that went down, if a repeat or long-press
    /// binding is waiting on it.
    fn hold(&mut self, code: u32, device: BindingDevice, name: &str) {
        let modifiers = self.modifiers();
        let waits = self.bindings.iter().any(|(_, hotkey)| {
            hotkey.device == device
                && crate::keys::canonical(&hotkey.key) == name
                && (hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.win) == modifiers
                && matches!(
                    hotkey.trigger,
                    BindingTrigger::Repeat | BindingTrigger::LongPress
                )
        });
        if waits
            && !self
                .held
                .iter()
                .any(|held| held.code == code && held.device == device)
        {
            self.held.push(crate::state::Held {
                code,
                device,
                name: name.to_string(),
                modifiers,
                since: std::time::Instant::now(),
                last_repeat: None,
                long_fired: Vec::new(),
            });
            // Its repeat and long press are timed by the host's passes.
            self.host_soon();
        }
    }

    fn release_held(&mut self, code: u32) {
        self.held.retain(|held| held.code != code);
    }

    /// Run the repeat and long-press bindings of what is held down. Called
    /// from the host loop; `now` is a parameter so tests can move time.
    pub fn tick_held(&mut self, now: std::time::Instant) {
        if self.held.is_empty() || self.paused {
            return;
        }
        let mut due: Vec<(Vec<i32>, crate::state::Held, BindingTrigger, u32)> = Vec::new();
        for index in 0..self.held.len() {
            let held = self.held[index].clone();
            let down_for = now.saturating_duration_since(held.since);
            let matches = |hotkey: &gevrek_engine::input::Hotkey| {
                hotkey.device == held.device
                    && crate::keys::canonical(&hotkey.key) == held.name
                    && (hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.win) == held.modifiers
            };
            // Long press: once, when the key has been down long enough.
            let long: Vec<i32> = self
                .bindings
                .iter()
                .filter(|(id, hotkey)| {
                    matches(hotkey)
                        && hotkey.trigger == BindingTrigger::LongPress
                        && down_for.as_millis() >= u128::from(hotkey.long_press_ms)
                        && !held.long_fired.contains(id)
                })
                .map(|(id, _)| *id)
                .collect();
            if !long.is_empty() {
                self.held[index].long_fired.extend(long.iter().copied());
                due.push((
                    long,
                    held.clone(),
                    BindingTrigger::LongPress,
                    down_for.as_millis() as u32,
                ));
            }
            // Repeat: after a delay, then at a steady rate.
            let next = match held.last_repeat {
                None => held.since + REPEAT_DELAY,
                Some(last) => last + REPEAT_INTERVAL,
            };
            if now >= next {
                let repeat: Vec<i32> = self
                    .bindings
                    .iter()
                    .filter(|(_, hotkey)| {
                        matches(hotkey) && hotkey.trigger == BindingTrigger::Repeat
                    })
                    .map(|(id, _)| *id)
                    .collect();
                if !repeat.is_empty() {
                    self.held[index].last_repeat = Some(now);
                    due.push((
                        repeat,
                        held,
                        BindingTrigger::Repeat,
                        down_for.as_millis() as u32,
                    ));
                }
            }
        }
        for (ids, held, trigger, held_ms) in due {
            self.bindings_triggered(
                ids.clone(),
                HotkeyEvent {
                    registration_ids: ids,
                    key: held.name,
                    ctrl: held.modifiers.0,
                    alt: held.modifiers.1,
                    shift: held.modifiers.2,
                    win: held.modifiers.3,
                    device: held.device,
                    trigger,
                    held_ms,
                },
            );
        }
    }

    /// Start moving (or resizing) a window with the pointer.
    /// `resize` is `None` to move, or which edges move: (left, top).
    pub fn begin_drag(&mut self, id: WindowHandle, resize: Option<(bool, bool)>, button: u32) {
        let Some(rect) = self.desktop.windows.get(&id).map(|window| window.rect) else {
            return;
        };
        self.drag = Some(crate::state::Drag {
            id,
            resize: resize.is_some(),
            from_left: resize.is_some_and(|(left, _)| left),
            from_top: resize.is_some_and(|(_, top)| top),
            button,
            start_pointer: self.desktop.pointer,
            start_rect: rect,
        });
        // The pointer is gevrek's while the window is moved: the program
        // under it is told that the pointer left (as the protocol has it
        // for a move a program asks for), so that it does not go on
        // thinking a button held whose release it will never be told.
        if let Some(pointer) = self.seat.get_pointer() {
            let location = pointer.current_location();
            pointer.motion(
                self,
                None,
                &smithay::input::pointer::MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 0,
                },
            );
            pointer.frame(self);
        }
        self.window_event(WindowEvent::DragStarted(id));
    }

    fn end_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            // The size the window was dragged to last may be one its
            // client did not take, and no commit follows to say so when
            // the client's size did not change with it: the window is as
            // large as its client drew it, and the engine is told.
            let drawn = self.desktop.windows.get_mut(&drag.id).and_then(|window| {
                let drawn = window.committed?;
                ((window.rect.width, window.rect.height) != drawn && !window.overflows).then(|| {
                    window.rect.width = drawn.0;
                    window.rect.height = drawn.1;
                    drawn
                })
            });
            if let (true, Some((width, height))) = (drag.resize, drawn) {
                self.desktop.needs_redraw = true;
                self.window_event(WindowEvent::Resized(drag.id, width, height));
            }
            // The engine decides what the drop means: a floating window
            // stays, a tiled one swaps places or goes back.
            self.window_event(WindowEvent::DragEnded(drag.id));
            // The button that moved the window is up for the seat too (no
            // program is told: none has the pointer), and what is under
            // the pointer now has it again.
            if let Some(pointer) = self.seat.get_pointer() {
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: 0,
                        button: drag.button,
                        state: ButtonState::Released,
                    },
                );
                pointer.frame(self);
            }
            let (x, y) = self.pointer_exact;
            self.pointer_moved((x, y).into(), 0);
        }
    }

    /// The pointer moved during a move or resize: the window follows.
    fn drag_to(&mut self, x: i32, y: i32) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let (dx, dy) = (x - drag.start_pointer.0, y - drag.start_pointer.1);
        let start = drag.start_rect;
        if drag.resize {
            // The edge that moves follows the pointer; the opposite one stays.
            let width = if drag.from_left {
                start.width - dx
            } else {
                start.width + dx
            }
            .max(MIN_DRAG_SIZE);
            let height = if drag.from_top {
                start.height - dy
            } else {
                start.height + dy
            }
            .max(MIN_DRAG_SIZE);
            let x = if drag.from_left {
                start.x + start.width - width
            } else {
                start.x
            };
            let y = if drag.from_top {
                start.y + start.height - height
            } else {
                start.y
            };
            let rect = gevrek_engine::Rect::new(x, y, width, height);
            self.desktop.place_now(drag.id, rect);
            self.window_event(WindowEvent::Resized(drag.id, rect.width, rect.height));
        } else {
            let rect =
                gevrek_engine::Rect::new(start.x + dx, start.y + dy, start.width, start.height);
            self.desktop.place_now(drag.id, rect);
            self.window_event(WindowEvent::Moved(drag.id, rect.x, rect.y));
        }
    }

    /// A key went down or up: bindings see it first, then the focused
    /// client. `code` is the xkb keycode (the evdev code plus 8).
    pub fn key_event(&mut self, code: Keycode, key_state: KeyState, time: u32) {
        {
            {
                let serial = SERIAL_COUNTER.next_serial();
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };
                let pressed = key_state == KeyState::Pressed;
                // A program that asked for every key (a virtual machine, a
                // nested gevrek) gets them: bindings rest while it has the
                // keyboard. Asked before the keyboard is used below.
                // (The key before this one may have been the layout's
                // own switch: Alt+Shift. Its release is the next key.)
                self.refresh_keyboard_layout();
                let inhibited = self.shortcuts_inhibited();
                let asked_for_keys = self.shortcuts_inhibit_requested();
                let mut toggle_inhibit = false;
                let hit = keyboard.input(
                    self,
                    code,
                    key_state,
                    serial,
                    time,
                    |state, modifiers, keysym| {
                        // Ctrl+Alt+F<n> is the key for another virtual
                        // terminal (XF86Switch_VT_1 to _12), where gevrek
                        // owns one. Nested in a window it is an ordinary
                        // chord that a binding may use.
                        let switch = keysym.modified_sym().raw();
                        if state.vt_switching
                            && pressed
                            && (0x1008_fe01..=0x1008_fe0c).contains(&switch)
                        {
                            state.vt_request = Some((switch - 0x1008_fe00) as i32);
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        // The key without modifiers applied: Shift+1 is "1".
                        // By its place on the keyboard, not by the layout
                        // that is active: where a layout has no Latin
                        // letter on a key (Turkish, Greek, Cyrillic), the
                        // one another layout has there is taken, so
                        // Super+T is the same key whatever is typed with it.
                        let name = keysym
                            .raw_latin_sym_or_raw_current_sym()
                            .or_else(|| keysym.raw_syms().first().copied())
                            .and_then(|keysym| crate::keys::key_name(keysym.raw()));
                        let Some(name) = name else {
                            return FilterResult::Forward;
                        };
                        let chord = (
                            modifiers.ctrl,
                            modifiers.alt,
                            modifiers.shift,
                            modifiers.logo,
                        );
                        // Ctrl+Alt+Backspace twice within two seconds ends
                        // the session. It is not a binding and cannot be
                        // taken away: the way out when the configuration,
                        // or a program holding every key, leaves no other.
                        if name == "BACKSPACE" && chord == (true, true, false, false) {
                            if pressed {
                                let now = std::time::Instant::now();
                                let again = state.rescue_pressed.is_some_and(|before| {
                                    now.duration_since(before) < std::time::Duration::from_secs(2)
                                });
                                if again {
                                    log::warn!("Ctrl+Alt+Backspace twice: the session ends");
                                    state.running = false;
                                }
                                state.rescue_pressed = Some(now);
                            }
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        // Super+Escape is the way out of (and back into)
                        // handing every key to a program. It always works.
                        if asked_for_keys
                            && name == "ESCAPE"
                            && chord == (false, false, false, true)
                        {
                            if pressed {
                                toggle_inhibit = true;
                                state.suppressed_keys.insert(code.raw());
                            } else {
                                state.suppressed_keys.remove(&code.raw());
                            }
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        if inhibited || state.locked {
                            return FilterResult::Forward;
                        }
                        // A window is being picked: Escape ends that, and
                        // is nobody else's (nor is its release).
                        if name == "ESCAPE"
                            && chord == (false, false, false, false)
                            && (state.picking.is_some()
                                || (!pressed && state.pick_escape == Some(code.raw())))
                        {
                            if pressed {
                                state.pick_cancel_asked = true;
                                state.pick_escape = Some(code.raw());
                            } else {
                                state.pick_escape = None;
                            }
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        // The overview of one workspace's windows is up:
                        // its keys are its own (Escape, the arrows, Tab,
                        // Enter), and so is their release, though the
                        // overview may be gone by then.
                        if chord == (false, false, false, false)
                            && ((pressed
                                && crate::window_overview::is_its_key(&name)
                                && state.window_overview_is_up())
                                || (!pressed
                                    && state.window_overview.held.contains(&code.raw())))
                        {
                            if pressed {
                                state.window_overview.held.insert(code.raw());
                                state.window_overview.asked = Some(name.to_string());
                            } else {
                                state.window_overview.held.remove(&code.raw());
                            }
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        // The overview of every workspace is up: Escape
                        // puts it away, and is nobody else's (nor is its
                        // release, when the overview is gone by then).
                        if name == "ESCAPE"
                            && chord == (false, false, false, false)
                            && (state.workspace_overview.is_some()
                                || (!pressed && state.overview_escape == Some(code.raw())))
                        {
                            if pressed {
                                state.overview_leave_asked = true;
                                state.overview_escape = Some(code.raw());
                            } else {
                                state.overview_escape = None;
                            }
                            return FilterResult::Intercept((
                                Vec::new(),
                                String::new(),
                                *modifiers,
                                BindingTrigger::Press,
                            ));
                        }
                        if pressed {
                            state.pending_holds.push((code.raw(), name.clone()));
                        } else {
                            state.release_held(code.raw());
                        }
                        // A key that comes up is the stroke it was when it
                        // went down: its release is looked for with the
                        // modifiers held then, whichever of them were let
                        // go first (as on Windows, `hotkey.rs`). What runs
                        // is told those modifiers too.
                        // And with the bindings that were live then: a
                        // press that goes into another set of bindings
                        // (a submap) is let go in the one it was pressed
                        // in.
                        let (on_release_now, _) = state.matching_bindings(
                            BindingDevice::Keyboard,
                            &name,
                            chord,
                            BindingTrigger::Release,
                        );
                        let (chord, on_release) = stroke(
                            &mut state.pressed_with,
                            code.raw(),
                            pressed,
                            chord,
                            on_release_now,
                        );
                        let mut told = *modifiers;
                        (told.ctrl, told.alt, told.shift, told.logo) = chord;
                        let modifiers = &told;
                        if !pressed {
                            // (Bindings rest while gevrek is paused.)
                            let ids = if state.paused {
                                Vec::new()
                            } else {
                                on_release
                            };
                            // The release of a key whose press was kept
                            // is kept too; one whose press the program
                            // saw is seen by it.
                            if state.suppressed_keys.remove(&code.raw()) {
                                return FilterResult::Intercept((
                                    ids,
                                    name,
                                    *modifiers,
                                    BindingTrigger::Release,
                                ));
                            }
                            if !ids.is_empty() {
                                state.pending_bindings.push((
                                    ids,
                                    name,
                                    *modifiers,
                                    BindingTrigger::Release,
                                ));
                            }
                            return FilterResult::Forward;
                        }
                        let trigger = BindingTrigger::Press;
                        let (ids, consume) =
                            state.matching_bindings(BindingDevice::Keyboard, &name, chord, trigger);
                        if consume {
                            state.suppressed_keys.insert(code.raw());
                            FilterResult::Intercept((ids, name, *modifiers, trigger))
                        } else {
                            // A binding that lets the key through runs, and
                            // the client gets the key as well.
                            if !ids.is_empty() {
                                state
                                    .pending_bindings
                                    .push((ids, name, *modifiers, trigger));
                            }
                            FilterResult::Forward
                        }
                    },
                );
                // (Ctrl or Shift, while something is dragged: what the
                // drag does is another thing now.)
                self.drag_keys_changed();
                if std::mem::take(&mut self.pick_cancel_asked) {
                    self.end_pick(None);
                }
                if std::mem::take(&mut self.overview_leave_asked) {
                    self.leave_workspace_overview(None);
                }
                if let Some(key) = self.window_overview.asked.take() {
                    self.window_overview_key(&key);
                    self.sync_window_overview();
                }
                if toggle_inhibit && !self.toggle_shortcuts_inhibit() {
                    log::debug!("Super+Escape: no program has asked for every key");
                }
                for (code, name) in std::mem::take(&mut self.pending_holds) {
                    self.hold(code, BindingDevice::Keyboard, &name);
                }
                let mut triggered = std::mem::take(&mut self.pending_bindings);
                triggered.extend(hit);
                for (ids, key, modifiers, trigger) in triggered {
                    if ids.is_empty() {
                        continue;
                    }
                    self.bindings_triggered(
                        ids.clone(),
                        HotkeyEvent {
                            registration_ids: ids,
                            key,
                            ctrl: modifiers.ctrl,
                            alt: modifiers.alt,
                            shift: modifiers.shift,
                            win: modifiers.logo,
                            device: BindingDevice::Keyboard,
                            trigger,
                            held_ms: 0,
                        },
                    );
                }
            }
        }
    }

    pub fn clamp_to_outputs(&self, location: Point<f64, Logical>) -> Point<f64, Logical> {
        let Some(first) = self.desktop.outputs.first().map(|output| output.rect) else {
            return location;
        };
        let (mut left, mut top) = (first.x, first.y);
        let (mut right, mut bottom) = (first.x + first.width, first.y + first.height);
        for output in &self.desktop.outputs {
            left = left.min(output.rect.x);
            top = top.min(output.rect.y);
            right = right.max(output.rect.x + output.rect.width);
            bottom = bottom.max(output.rect.y + output.rect.height);
        }
        (
            location.x.clamp(f64::from(left), f64::from(right - 1)),
            location.y.clamp(f64::from(top), f64::from(bottom - 1)),
        )
            .into()
    }

    /// The pointer is at `location` now: the surface under it gets the
    /// pointer, and with focus-follows-mouse its window gets the focus.
    /// The pointer moved by a distance (a mouse, a touchpad). Programs that
    /// read the mouse as movement are told the movement; the pointer moves
    /// unless the program it is over has locked it, and stays inside that
    /// program's window if it is confined there.
    pub fn pointer_moved_by(
        &mut self,
        delta: Point<f64, Logical>,
        delta_unaccel: Point<f64, Logical>,
        time_microseconds: u64,
    ) {
        let time = (time_microseconds / 1000) as u32;
        let constraint = if self.drag.is_some() {
            None
        } else {
            self.pointer_constraint()
        };
        if let Some(pointer) = self.seat.get_pointer() {
            let focus = pointer.current_focus().map(|surface| {
                let at = self.surface_under(self.pointer_exact.into()).1;
                let origin = at
                    .filter(|(under, _)| *under == surface)
                    .map_or((0.0, 0.0).into(), |(_, origin)| origin);
                (surface, origin)
            });
            pointer.relative_motion(
                self,
                focus,
                &smithay::input::pointer::RelativeMotionEvent {
                    delta,
                    delta_unaccel,
                    utime: time_microseconds,
                },
            );
            pointer.frame(self);
        }
        if constraint == Some(true) {
            // Locked: only the movement is reported.
            return;
        }
        let mut location: Point<f64, Logical> = (
            self.pointer_exact.0 + delta.x,
            self.pointer_exact.1 + delta.y,
        )
            .into();
        if constraint == Some(false) {
            // Confined: the pointer stays on the window it is on.
            let (x, y) = self.desktop.pointer;
            if let Some(rect) = self
                .desktop
                .window_at(x, y)
                .and_then(|id| self.desktop.windows.get(&id))
                .map(|window| window.rect)
            {
                location.x = location
                    .x
                    .clamp(f64::from(rect.x), f64::from(rect.x + rect.width - 1));
                location.y = location
                    .y
                    .clamp(f64::from(rect.y), f64::from(rect.y + rect.height - 1));
            }
        }
        let location = self.clamp_to_outputs(location);
        self.pointer_moved(location, time);
    }

    pub fn pointer_moved(&mut self, location: Point<f64, Logical>, time: u32) {
        self.pointer_exact = (location.x, location.y);
        if self.drag.is_some() {
            self.desktop.pointer = (location.x as i32, location.y as i32);
            self.desktop.needs_redraw = true;
            self.tell_pointer_watches();
            self.drag_to(location.x as i32, location.y as i32);
            return;
        }
        let before = self
            .desktop
            .window_at(self.desktop.pointer.0, self.desktop.pointer.1);
        // Where gevrek draws the pointer itself, a move is something to draw.
        if self.draw_own_cursor && self.desktop.pointer != (location.x as i32, location.y as i32) {
            self.desktop.needs_redraw = true;
        }
        self.desktop.pointer = (location.x as i32, location.y as i32);
        // (A program that follows the pointer by itself is told at once,
        // whoever draws the pointer.)
        self.tell_pointer_watches();
        // (What a Wayland program drags is told to an X11 window it is over.)
        self.follow_drag_onto_x11();
        self.follow_x11_drag();
        let (window, focus) = self.surface_under(location);
        if let Some((surface, _)) = &focus {
            self.activate_pointer_constraint(surface);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            let grabbed = pointer.is_grabbed();
            pointer.motion(
                self,
                focus.clone(),
                &MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            // A menu's hold on the pointer that ended with its menu takes
            // this movement with it: said again, so that what the pointer
            // is over now has it (and the press that may follow).
            if grabbed && !pointer.is_grabbed() {
                pointer.motion(
                    self,
                    focus,
                    &MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
            }
            pointer.frame(self);
        }
        if self.config.desktop.focus.focus_follows_mouse && window != before {
            if let Some(id) = window {
                self.focus_window(id);
            }
        }
        // (Over a picture of the overview of the windows: it is the one
        // pointed at.)
        self.window_overview_pointed();
        // A hot corner: come into, or left. Its moment is looked for by
        // the host's passes, which are asked for while it lasts.
        self.tick_hot_corners(std::time::Instant::now());
        if self.hot_corners.waits() {
            self.host_soon();
        }
    }

    /// Hot corners (`desktop.hot_corners`): the pointer, left in a corner
    /// of a display for a moment, runs that corner's command. Called when
    /// the pointer moves and from the host's passes; `now` is a parameter
    /// so that tests can move time.
    pub fn tick_hot_corners(&mut self, now: std::time::Instant) {
        if self.paused || !self.config.desktop.hot_corners.watched() {
            return;
        }
        let displays: Vec<gevrek_model::geometry::Rect> = self
            .desktop
            .outputs
            .iter()
            .map(|output| output.rect)
            .collect();
        // (Something is dragged there, not pointed at.)
        let held = self.desktop.primary_button_down || self.drag.is_some();
        let Some(command) = self.hot_corners.step(
            &self.config.desktop.hot_corners,
            &displays,
            self.desktop.pointer,
            held,
            now,
        ) else {
            return;
        };
        // Not over a program that fills the display (a game, a film): a
        // pointer in its corner means something there.
        let fills = self.desktop.focused.is_some_and(|window| {
            gevrek_engine::WindowQueries::is_fullscreen(&self.desktop, window)
        });
        if fills {
            return;
        }
        log::info!("A hot corner runs: {}", command.display_name());
        if let Err(error) = self.run_command(command) {
            log::warn!("a hot corner's command could not be run: {error}");
        }
    }

    /// The window at a point, and the surface of it (or of one of its
    /// popups and subsurfaces) that is there, with where that surface is.
    /// The popup (menu, tooltip) of a window at a point, with its window.
    fn popup_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(Option<WindowHandle>, Option<Under>)> {
        use smithay::desktop::{PopupKind, PopupManager};
        for id in self.desktop.visible_in_order().into_iter().rev() {
            let window = &self.desktop.windows[&id];
            let Some(surface) = window.surface.wl_surface() else {
                continue;
            };
            let origin = (
                window.rect.x - window.geometry_offset.0,
                window.rect.y - window.geometry_offset.1,
            );
            // The last one opened is the one on top.
            let popups: Vec<_> = PopupManager::popups_for_surface(&surface).collect();
            for (popup, offset) in popups.into_iter().rev() {
                let PopupKind::Xdg(xdg) = &popup else {
                    continue;
                };
                let geometry = popup.geometry();
                let at = (
                    origin.0 + offset.x - geometry.loc.x,
                    origin.1 + offset.y - geometry.loc.y,
                );
                if let Some((surface, at)) =
                    under_from_surface_tree(xdg.wl_surface(), location, at, WindowSurfaceType::ALL)
                {
                    return Some((Some(id), Some((surface, at.to_f64()))));
                }
            }
        }
        None
    }

    /// Where on the desktop a touch screen's place is (`across` and
    /// `down` from 0 to 1 over the screen): on the first output, turned
    /// as its picture is.
    fn touch_place(&self, across: f64, down: f64) -> Option<Point<f64, Logical>> {
        let output = self.desktop.outputs.first()?;
        let (across, down) = crate::desktop::turned_place(output.transform, (across, down));
        Some(
            self.clamp_to_outputs(
                (
                    across * f64::from(output.rect.width) + f64::from(output.rect.x),
                    down * f64::from(output.rect.height) + f64::from(output.rect.y),
                )
                    .into(),
            ),
        )
    }

    /// A finger comes down at a place of the desktop: what is under it
    /// is told (a window is given the keyboard first, as by a click).
    pub fn touch_down(&mut self, finger: u32, location: Point<f64, Logical>, time: u32) {
        self.touch_down_slot(Some(finger).into(), location, time);
    }

    pub fn touch_motion(&mut self, finger: u32, location: Point<f64, Logical>, time: u32) {
        self.touch_motion_slot(Some(finger).into(), location, time);
    }

    pub fn touch_up(&mut self, finger: u32, time: u32) {
        self.touch_up_slot(Some(finger).into(), time);
    }

    fn touch_down_slot(
        &mut self,
        slot: smithay::backend::input::TouchSlot,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        let Some(touch) = self.seat.get_touch() else {
            return;
        };
        // (A touch is input: the screen stays on and unlocked-for.)
        self.activity();
        // The keyboard goes where the finger does, as with a press of
        // the mouse: to a surface of the shell that takes it, or to the
        // window under the finger.
        if !self.locked {
            self.click_layer_focus(location);
            let on_layer = self
                .layer_surface_under(location, &[Layer::Overlay, Layer::Top])
                .is_some();
            if !on_layer {
                self.emit(
                    gevrek_engine::executor::RuntimeEventKind::DesktopPressed,
                    None,
                    Vec::new(),
                );
                if let Some(id) = self.desktop.window_at(location.x as i32, location.y as i32) {
                    self.focus_window(id);
                }
            }
        }
        let (_, focus) = self.surface_under(location);
        let serial = SERIAL_COUNTER.next_serial();
        touch.down(
            self,
            focus,
            &smithay::input::touch::DownEvent {
                slot,
                location,
                serial,
                time,
            },
        );
        self.settle();
    }

    fn touch_motion_slot(
        &mut self,
        slot: smithay::backend::input::TouchSlot,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        let Some(touch) = self.seat.get_touch() else {
            return;
        };
        // (A finger that is down stays with the surface it came down
        // on, as the protocol has it: smithay keeps that focus.)
        let (_, focus) = self.surface_under(location);
        touch.motion(
            self,
            focus,
            &smithay::input::touch::MotionEvent {
                slot,
                location,
                time,
            },
        );
    }

    fn touch_up_slot(&mut self, slot: smithay::backend::input::TouchSlot, time: u32) {
        let Some(touch) = self.seat.get_touch() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        touch.up(self, &smithay::input::touch::UpEvent { slot, serial, time });
    }

    /// The fingers' news of one moment are complete.
    pub fn touch_frame(&mut self) {
        if let Some(touch) = self.seat.get_touch() {
            touch.frame(self);
        }
    }

    /// The touches are taken back (the screen says they were none).
    pub fn touch_cancel(&mut self) {
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
        }
    }

    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> (Option<WindowHandle>, Option<Under>) {
        // Locked: only the lock screen's surface of the output is there.
        if self.locked {
            let output = self.desktop.outputs.iter().find(|output| {
                let rect = output.rect;
                location.x >= f64::from(rect.x)
                    && location.x < f64::from(rect.x + rect.width)
                    && location.y >= f64::from(rect.y)
                    && location.y < f64::from(rect.y + rect.height)
            });
            let under = output.and_then(|output| {
                let lock = self.lock_surface(output.id)?;
                under_from_surface_tree(
                    lock.wl_surface(),
                    location,
                    (output.rect.x, output.rect.y),
                    WindowSurfaceType::ALL,
                )
                .map(|(surface, at)| (surface, at.to_f64()))
            });
            return (None, under);
        }
        // X11 menus and tooltips are above everything (while the windows
        // of their program are in sight).
        for window in self.x11_overrides.iter().rev() {
            if !self.x11_own_shown(window) {
                continue;
            }
            let geometry = window.geometry();
            let Some(surface) = window.wl_surface() else {
                continue;
            };
            if let Some((surface, at)) =
                under_from_surface_tree(&surface, location, geometry.loc, WindowSurfaceType::ALL)
            {
                return (None, Some((surface, at.to_f64())));
            }
        }
        // What is above everything comes first; a fullscreen window is
        // above bars and launchers, as it is drawn.
        if let Some(found) = self.layer_surface_under(location, &[Layer::Overlay]) {
            return (None, Some(found));
        }
        let fullscreen_here = self
            .desktop
            .window_at(location.x as i32, location.y as i32)
            .is_some_and(|id| {
                self.desktop.windows[&id].fullscreen || self.desktop.over_fullscreen(id)
            });
        if !fullscreen_here {
            if let Some(found) = self.layer_surface_under(location, &[Layer::Top]) {
                return (None, Some(found));
            }
        }
        // A window's menus are above every window and may reach beyond
        // their own.
        if let Some(found) = self.popup_under(location) {
            return found;
        }
        let Some(id) = self.desktop.window_at(location.x as i32, location.y as i32) else {
            // Nothing but the wallpaper, if there is one.
            let below = self.layer_surface_under(location, &[Layer::Bottom, Layer::Background]);
            return (None, below);
        };
        let window = &self.desktop.windows[&id];
        // In an overview a window is a picture of itself: a press chooses
        // it, and nothing of the pointer reaches its program (the places
        // on the picture are not the places in the window).
        if window.overview.is_some() {
            return (Some(id), None);
        }
        let origin = (
            window.rect.x - window.geometry_offset.0,
            window.rect.y - window.geometry_offset.1,
        );
        let under = window.surface.wl_surface().and_then(|surface| {
            under_from_surface_tree(&surface, location, origin, WindowSurfaceType::ALL)
                .map(|(surface, at)| (surface, at.to_f64()))
        });
        (Some(id), under)
    }
}
