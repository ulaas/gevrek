//! Binding matching: which registered binding a raw key or button event
//! hits, and whether it is consumed. Pure; the Windows low-level hooks build
//! a `RawInputEvent` and ask.

use gevrek_model::settings::{BindingDevice, BindingTrigger};
use std::collections::HashMap;

/// A normalized input event emitted after the configured trigger semantics
/// (press, release, repeat, or long press) have been resolved.
#[derive(Debug, Clone)]
pub struct HotkeyEvent {
    pub registration_ids: Vec<i32>,
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub device: BindingDevice,
    pub trigger: BindingTrigger,
    pub held_ms: u32,
}

/// Registration consumed by the unified low-level keyboard/mouse input engine.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Hotkey {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub device: BindingDevice,
    pub trigger: BindingTrigger,
    pub consume: bool,
    pub long_press_ms: u32,
}

impl Hotkey {
    pub fn new(key: String, ctrl: bool, alt: bool, shift: bool, win: bool) -> Self {
        Self {
            key,
            ctrl,
            alt,
            shift,
            win,
            device: BindingDevice::Keyboard,
            trigger: BindingTrigger::Press,
            consume: true,
            long_press_ms: 500,
        }
    }

    pub fn normalized_key(&self) -> String {
        self.key.trim().to_ascii_uppercase()
    }

    pub fn matches(&self, raw: &RawInputEvent) -> bool {
        self.device == raw.device
            && self.normalized_key() == raw.key
            && self.ctrl == raw.ctrl
            && self.alt == raw.alt
            && self.shift == raw.shift
            && self.win == raw.win
    }
}

#[derive(Debug, Clone)]
pub struct RawInputEvent {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub device: BindingDevice,
    pub down: bool,
    pub momentary: bool,
}

pub fn matching_registration_snapshot(
    registrations: &HashMap<i32, Hotkey>,
    raw: &RawInputEvent,
) -> HashMap<i32, Hotkey> {
    registrations
        .iter()
        .filter(|(_, hotkey)| hotkey.matches(raw))
        .map(|(id, hotkey)| (*id, hotkey.clone()))
        .collect()
}

pub fn should_consume(registrations: &HashMap<i32, Hotkey>, raw: &RawInputEvent) -> bool {
    registrations
        .values()
        .any(|hotkey| hotkey.consume && hotkey.matches(raw))
}
