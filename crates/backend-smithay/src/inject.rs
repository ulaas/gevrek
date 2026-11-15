//! Made-up input for automated checks (`gevrek_host::input_injection`):
//! each step goes through the same functions a real device's events do
//! (`key_event`, `pointer_moved`, `pointer_button`, `pointer_wheel`), so a
//! check exercises bindings, focus and drags as they really are.

use crate::state::State;
use gevrek_host::input_injection::{modifier_name, split_chord, InjectedInput};
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

/// Evdev codes of the mouse buttons.
fn button_code(name: &str) -> anyhow::Result<u32> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "left" => 0x110,
        "right" => 0x111,
        "middle" => 0x112,
        other => anyhow::bail!("unknown mouse button {other} (left, right, middle)"),
    })
}

/// Keysyms of the left-hand modifier keys.
fn modifier_keysym(modifier: &str) -> u32 {
    match modifier {
        "Super" => 0xffeb,
        "Ctrl" => 0xffe3,
        "Alt" => 0xffe9,
        _ => 0xffe1, // Shift
    }
}

impl State {
    fn injection_time(&mut self) -> u32 {
        // A clock of its own that only goes forward: the steps of one
        // request are a few milliseconds apart whatever the wall clock does.
        self.injected_time = self.injected_time.wrapping_add(8);
        self.injected_time
    }

    /// The key of the current keymap that carries `name`: a modifier, or a
    /// key by the name bindings use.
    fn keycode_named(&mut self, name: &str) -> anyhow::Result<Keycode> {
        let wanted_modifier = modifier_name(name).map(modifier_keysym);
        let wanted_key = crate::keys::canonical(name);
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| anyhow::anyhow!("there is no keyboard"))?;
        let found = keyboard.with_xkb_state(self, |context| {
            let xkb = context
                .xkb()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let layout = xkb.active_layout();
            (8u32..256).map(Keycode::new).find(|code| {
                let Some(keysym) = xkb
                    .raw_syms_for_key_in_layout(*code, layout)
                    .first()
                    .copied()
                else {
                    return false;
                };
                match wanted_modifier {
                    Some(modifier) => keysym.raw() == modifier,
                    None => {
                        crate::keys::key_name(keysym.raw()).as_deref() == Some(wanted_key.as_str())
                    }
                }
            })
        });
        found.ok_or_else(|| anyhow::anyhow!("no key of the keyboard layout is {name}"))
    }

    fn inject_key(&mut self, name: &str, pressed: bool) -> anyhow::Result<()> {
        let code = self.keycode_named(name)?;
        let time = self.injection_time();
        self.key_event(
            code,
            if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            time,
        );
        Ok(())
    }

    fn inject_move(&mut self, x: i32, y: i32) {
        let location = self.clamp_to_outputs((f64::from(x), f64::from(y)).into());
        let time = self.injection_time();
        self.pointer_moved(location, time);
    }

    fn inject_button(&mut self, button: &str, pressed: bool) -> anyhow::Result<()> {
        let code = button_code(button)?;
        let time = self.injection_time();
        self.pointer_button(code, pressed, time);
        Ok(())
    }

    fn inject_held(&mut self, hold: &[String], pressed: bool) -> anyhow::Result<()> {
        for name in hold {
            let modifier = modifier_name(name).ok_or_else(|| {
                anyhow::anyhow!("{name} is not a modifier (Super, Ctrl, Alt, Shift)")
            })?;
            self.inject_key(modifier, pressed)?;
        }
        Ok(())
    }

    /// Carry out the steps, in order. Stops at the first that cannot be.
    pub fn inject_input(&mut self, steps: &[InjectedInput]) -> anyhow::Result<()> {
        self.activity();
        for step in steps {
            match step {
                InjectedInput::Chord { chord } => {
                    let (modifiers, key) = split_chord(chord);
                    for modifier in &modifiers {
                        self.inject_key(modifier, true)?;
                    }
                    if let Some(key) = &key {
                        self.inject_key(key, true)?;
                        self.inject_key(key, false)?;
                    }
                    for modifier in modifiers.iter().rev() {
                        self.inject_key(modifier, false)?;
                    }
                }
                InjectedInput::Key { key, pressed } => self.inject_key(key, *pressed)?,
                InjectedInput::Keycode { code, hold } => {
                    self.inject_held(hold, true)?;
                    for state in [KeyState::Pressed, KeyState::Released] {
                        let time = self.injection_time();
                        self.key_event(Keycode::new(*code), state, time);
                    }
                    self.inject_held(hold, false)?;
                }
                InjectedInput::Move { x, y } => self.inject_move(*x, *y),
                InjectedInput::Button { button, pressed } => {
                    self.inject_button(button, *pressed)?
                }
                InjectedInput::Click { x, y, button } => {
                    self.inject_move(*x, *y);
                    self.inject_button(button, true)?;
                    self.inject_button(button, false)?;
                }
                InjectedInput::Drag {
                    from,
                    to,
                    button,
                    hold,
                } => {
                    self.inject_held(hold, true)?;
                    self.inject_move(from.0, from.1);
                    self.inject_button(button, true)?;
                    const STEPS: i32 = 12;
                    for step in 1..=STEPS {
                        self.inject_move(
                            from.0 + (to.0 - from.0) * step / STEPS,
                            from.1 + (to.1 - from.1) * step / STEPS,
                        );
                    }
                    self.inject_button(button, false)?;
                    self.inject_held(hold, false)?;
                }
                InjectedInput::Wheel { up, hold } => {
                    self.inject_held(hold, true)?;
                    // As with a device: a binding on the wheel runs
                    // instead of scrolling.
                    let turn = if *up { -15.0 } else { 15.0 };
                    if !self.pointer_wheel(*up)
                        && !(self.strip_modifier_held() && self.strip_wheel(0.0, turn, false))
                    {
                        let time = self.injection_time();
                        self.wheel_step_to_client(*up, time);
                    }
                    self.inject_held(hold, false)?;
                }
                InjectedInput::PlugOutput { width, height } => {
                    anyhow::ensure!(
                        self.headless,
                        "outputs can only be plugged into a window manager without a display (--backend headless)"
                    );
                    anyhow::ensure!(*width > 0 && *height > 0, "an output needs a size");
                    let id = self.add_virtual_output(*width, *height);
                    self.output_added(id);
                }
                InjectedInput::UnplugOutput { name } => {
                    anyhow::ensure!(
                        self.headless,
                        "outputs can only be unplugged from a window manager without a display (--backend headless)"
                    );
                    let id = self
                        .desktop
                        .outputs
                        .iter()
                        .find(|output| &output.name == name)
                        .map(|output| output.id)
                        .ok_or_else(|| anyhow::anyhow!("there is no output {name}"))?;
                    self.remove_output(id);
                }
                InjectedInput::Touch {
                    finger,
                    phase,
                    x,
                    y,
                } => {
                    let time = self.injection_time();
                    let at = (f64::from(*x), f64::from(*y)).into();
                    match phase.as_str() {
                        "down" => self.touch_down(*finger, at, time),
                        "move" => self.touch_motion(*finger, at, time),
                        "up" => self.touch_up(*finger, time),
                        "cancel" => self.touch_cancel(),
                        other => anyhow::bail!("touch {other}: down, move, up or cancel"),
                    }
                    self.touch_frame();
                }
                InjectedInput::Pinch { phase, scale } => {
                    use smithay::input::pointer::{
                        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
                    };
                    let time = self.injection_time();
                    let Some(pointer) = self.seat.get_pointer() else {
                        anyhow::bail!("there is no pointer");
                    };
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    match phase.as_str() {
                        "begin" => pointer.gesture_pinch_begin(
                            self,
                            &GesturePinchBeginEvent {
                                serial,
                                time,
                                fingers: 2,
                            },
                        ),
                        "update" => pointer.gesture_pinch_update(
                            self,
                            &GesturePinchUpdateEvent {
                                time,
                                delta: (0.0, 0.0).into(),
                                scale: *scale,
                                rotation: 0.0,
                            },
                        ),
                        "end" => pointer.gesture_pinch_end(
                            self,
                            &GesturePinchEndEvent {
                                serial,
                                time,
                                cancelled: false,
                            },
                        ),
                        other => anyhow::bail!("pinch {other}: begin, update or end"),
                    }
                }
                InjectedInput::Wait { ms } => {
                    // The loop is held meanwhile; long waits belong in the
                    // check, between requests.
                    std::thread::sleep(std::time::Duration::from_millis((*ms).min(500)));
                }
            }
            self.settle();
        }
        Ok(())
    }
}
