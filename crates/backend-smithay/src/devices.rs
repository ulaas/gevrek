//! Keyboards, mice and touchpads as the configuration wants them
//! (`desktop.input`): the keyboard's layouts, options and repeat, Num Lock,
//! and what libinput offers for each pointing device. Applied when the
//! session starts, when a device is plugged in, and when the
//! configuration changes.

use crate::state::State;
use gevrek_host::wm::published::Published as _;
use gevrek_model::input::{AccelProfile, InputConfig, PointerConfig, ScrollMethod, TouchpadConfig};
use smithay::input::keyboard::XkbConfig;
use smithay::reexports::input as libinput;

impl State {
    /// Bring the keyboard and every known device to the configuration.
    pub fn apply_input_config(&mut self) {
        let input = self.config.desktop.input.clone();
        self.apply_keyboard_config(&input);
        for device in &mut self.input_devices {
            configure_device(device, &input);
        }
    }

    fn apply_keyboard_config(&mut self, input: &InputConfig) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let (layout, variant) = input.keyboard.xkb_layouts();
        // (With the keys that switch the layout, where there are several.)
        let options = input.keyboard.xkb_options().join(",");
        // Nothing named: what the system says (XKB_DEFAULT_*), as before.
        let wanted = (
            layout.clone(),
            variant.clone(),
            options.clone(),
            input.keyboard.model.clone(),
        );
        if self.applied_keymap.as_ref() != Some(&wanted) {
            let config = XkbConfig {
                rules: "",
                model: &input.keyboard.model,
                layout: &layout,
                variant: &variant,
                options: (!options.is_empty()).then(|| options.clone()),
            };
            match keyboard.set_xkb_config(self, config) {
                Ok(()) => {
                    log::info!(
                        "Keyboard: layouts '{}' variants '{}' options '{}'",
                        if layout.is_empty() {
                            "(system)"
                        } else {
                            &layout
                        },
                        variant,
                        options
                    );
                    self.applied_keymap = Some(wanted);
                }
                // The keymap that was there stays.
                Err(error) => log::error!("The keyboard layout could not be set: {error:?}"),
            }
        }
        keyboard.change_repeat_info(
            input.keyboard.repeat_rate as i32,
            input.keyboard.repeat_delay_ms as i32,
        );
    }

    /// Switch among the layouts of the keyboard.
    pub fn switch_keyboard_layout(
        &mut self,
        target: gevrek_model::command::LayoutTarget,
    ) -> anyhow::Result<()> {
        use gevrek_model::command::LayoutTarget;
        use smithay::input::keyboard::Layout;
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or_else(|| anyhow::anyhow!("there is no keyboard"))?;
        let switched = keyboard.with_xkb_state(self, |mut context| {
            let count = context
                .xkb()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .layouts()
                .count() as u32;
            match target {
                LayoutTarget::Next => context.cycle_next_layout(),
                LayoutTarget::Previous => context.cycle_prev_layout(),
                LayoutTarget::Index(index) if index < count => context.set_layout(Layout(index)),
                LayoutTarget::Index(index) => {
                    return Err(anyhow::anyhow!(
                        "There is no keyboard layout {index}: {count} are configured"
                    ))
                }
            }
            let xkb = context
                .xkb()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Ok(xkb.layout_name(xkb.active_layout()).to_string())
        })?;
        log::info!("Keyboard layout: {switched}");
        // (The panel shows it.)
        self.refresh_keyboard_layout();
        Ok(())
    }

    /// Look at which keyboard layout is in use (a key of the layout's
    /// own may have changed it, not only a command), for the state
    /// gevrek publishes; a change is published at once.
    pub(crate) fn refresh_keyboard_layout(&mut self) {
        use gevrek_engine::overlay_data::KeyboardLayoutNow;
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let (configured, _) = self.config.desktop.input.keyboard.xkb_layouts();
        // (None in the configuration: the system's, which is what is used.)
        let configured = if configured.trim_matches(',').is_empty() {
            std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default()
        } else {
            configured
        };
        let now = keyboard.with_xkb_state(self, |context| {
            let xkb = context
                .xkb()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let active = xkb.active_layout();
            let name = xkb.layout_name(active).to_string();
            KeyboardLayoutNow {
                short: KeyboardLayoutNow::short_of(&configured, active.0, &name),
                name,
                index: active.0,
                count: xkb.layouts().count() as u32,
            }
        });
        if self.keyboard_layout.as_ref() != Some(&now) {
            // Told at once to whoever shows it (the panel), except the
            // first time, when it is only found out.
            let changed = self.keyboard_layout.is_some();
            let fields = vec![
                ("layout".to_string(), now.short.clone()),
                ("name".to_string(), now.name.clone()),
                ("index".to_string(), now.index.to_string()),
            ];
            self.keyboard_layout = Some(now);
            self.host.state_stale.set(true);
            // (The state is published before the event: who asks for it
            // on hearing the event gets the new layout.)
            self.publish_state(None);
            if changed {
                self.emit(
                    gevrek_engine::executor::RuntimeEventKind::KeyboardLayoutChanged,
                    None,
                    fields,
                );
            }
        }
    }

    /// Num Lock as the configuration wants it at the start of the session:
    /// the key is pressed once if the lock is not where it should be.
    pub fn apply_numlock(&mut self) {
        let wanted = self.config.desktop.input.keyboard.numlock;
        let on = self
            .seat
            .get_keyboard()
            .is_some_and(|keyboard| keyboard.modifier_state().num_lock);
        if wanted != on {
            let steps = [
                gevrek_host::input_injection::InjectedInput::Key {
                    key: "NUMLOCK".to_string(),
                    pressed: true,
                },
                gevrek_host::input_injection::InjectedInput::Key {
                    key: "NUMLOCK".to_string(),
                    pressed: false,
                },
            ];
            if let Err(error) = self.inject_input(&steps) {
                log::debug!("Num Lock: {error}");
            }
        }
    }

    /// A device was plugged in (or was there at the start).
    pub fn device_added(&mut self, mut device: libinput::Device) {
        configure_device(&mut device, &self.config.desktop.input);
        log::info!("Input device: {}", device.name());
        self.input_devices.push(device);
        self.update_keyboard_leds();
    }

    pub fn device_removed(&mut self, device: &libinput::Device) {
        log::debug!("the input device {} is gone", device.name());
        self.input_devices.retain(|known| known != device);
    }

    /// Show Num Lock, Caps Lock and Scroll Lock on the keyboards.
    pub fn update_keyboard_leds(&mut self) {
        let Some(state) = self
            .seat
            .get_keyboard()
            .map(|keyboard| keyboard.led_state())
        else {
            return;
        };
        let mut leds = libinput::Led::empty();
        leds.set(libinput::Led::NUMLOCK, state.num.unwrap_or(false));
        leds.set(libinput::Led::CAPSLOCK, state.caps.unwrap_or(false));
        leds.set(libinput::Led::SCROLLLOCK, state.scroll.unwrap_or(false));
        for device in &mut self.input_devices {
            if device.has_capability(libinput::DeviceCapability::Keyboard) {
                device.led_update(leds);
            }
        }
    }
}

/// Apply the settings a device supports. A setting the device does not
/// have is skipped: libinput says so and nothing is wrong.
fn configure_device(device: &mut libinput::Device, input: &InputConfig) {
    // A touchpad is the pointing device that can tap.
    let touchpad = device.config_tap_finger_count() > 0;
    if touchpad {
        configure_touchpad(device, &input.touchpad);
    } else if device.has_capability(libinput::DeviceCapability::Pointer) {
        configure_mouse(device, &input.mouse);
    }
}

fn accel(device: &mut libinput::Device, speed: f64, profile: AccelProfile) {
    if device.config_accel_is_available() {
        said(
            device.config_accel_set_speed(speed.clamp(-1.0, 1.0)),
            device,
            "accel set speed",
        );
        let wanted = match profile {
            AccelProfile::Default => device.config_accel_default_profile(),
            AccelProfile::Adaptive => Some(libinput::AccelProfile::Adaptive),
            AccelProfile::Flat => Some(libinput::AccelProfile::Flat),
        };
        if let Some(wanted) = wanted {
            said(
                device.config_accel_set_profile(wanted),
                device,
                "accel set profile",
            );
        }
    }
}

fn configure_mouse(device: &mut libinput::Device, mouse: &PointerConfig) {
    accel(device, mouse.speed, mouse.accel_profile);
    if device.config_scroll_has_natural_scroll() {
        said(
            device.config_scroll_set_natural_scroll_enabled(mouse.natural_scroll),
            device,
            "scroll set natural scroll enabled",
        );
    }
    if device.config_left_handed_is_available() {
        said(
            device.config_left_handed_set(mouse.left_handed),
            device,
            "left handed set",
        );
    }
}

fn configure_touchpad(device: &mut libinput::Device, touchpad: &TouchpadConfig) {
    said(
        device.config_tap_set_enabled(touchpad.tap),
        device,
        "tap set enabled",
    );
    said(
        device.config_tap_set_drag_enabled(touchpad.tap_drag),
        device,
        "tap set drag enabled",
    );
    accel(device, touchpad.speed, touchpad.accel_profile);
    if device.config_scroll_has_natural_scroll() {
        said(
            device.config_scroll_set_natural_scroll_enabled(touchpad.natural_scroll),
            device,
            "scroll set natural scroll enabled",
        );
    }
    if device.config_dwt_is_available() {
        said(
            device.config_dwt_set_enabled(touchpad.disable_while_typing),
            device,
            "dwt set enabled",
        );
    }
    if device.config_left_handed_is_available() {
        said(
            device.config_left_handed_set(touchpad.left_handed),
            device,
            "left handed set",
        );
    }
    if device.config_middle_emulation_is_available() {
        said(
            device.config_middle_emulation_set_enabled(touchpad.middle_emulation),
            device,
            "middle emulation set enabled",
        );
    }
    let method = match touchpad.scroll_method {
        ScrollMethod::Default => device.config_scroll_default_method(),
        ScrollMethod::TwoFinger => Some(libinput::ScrollMethod::TwoFinger),
        ScrollMethod::Edge => Some(libinput::ScrollMethod::Edge),
        ScrollMethod::None => Some(libinput::ScrollMethod::NoScroll),
    };
    if let Some(method) = method {
        said(
            device.config_scroll_set_method(method),
            device,
            "scroll set method",
        );
    }
}

/// What libinput answered to a setting: a device that has the setting
/// and still refused it is said at debug.
fn said(answer: Result<(), libinput::DeviceConfigError>, device: &libinput::Device, setting: &str) {
    if let Err(error) = answer {
        log::debug!(
            "the input device {} refused the setting {setting}: {error:?}",
            device.name()
        );
    }
}
