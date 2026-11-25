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
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// (By the key's one name: a binding written `RETURN` is the key the
    /// keyboard tells of as `ENTER`. Written so, it never fired.)
    pub fn normalized_key(&self) -> String {
        gevrek_model::keys::canonical_key_name(&self.key)
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// The bindings by the key each is on, so that a key that was pressed
/// is held against the bindings on that key and not against them all.
/// (Every key of every program comes by here, and each binding it was
/// held against had its key's name written out anew for it.)
#[derive(Debug, Clone, Default)]
pub struct ByKey {
    ids: HashMap<String, Vec<i32>>,
}

impl ByKey {
    pub fn of(registrations: &HashMap<i32, Hotkey>) -> Self {
        let mut ids: HashMap<String, Vec<i32>> = HashMap::new();
        for (id, hotkey) in registrations {
            ids.entry(hotkey.normalized_key()).or_default().push(*id);
        }
        Self { ids }
    }

    /// The bindings that `raw` is: those on its key, with its device and
    /// what is held with it. `registrations` are the ones this was made of.
    pub fn matching<'a>(
        &'a self,
        registrations: &'a HashMap<i32, Hotkey>,
        raw: &'a RawInputEvent,
    ) -> impl Iterator<Item = (i32, &'a Hotkey)> + 'a {
        self.ids
            .get(raw.key.as_str())
            .into_iter()
            .flatten()
            .filter_map(move |id| registrations.get(id).map(|hotkey| (*id, hotkey)))
            .filter(move |(_, hotkey)| hotkey.matches(raw))
    }

    /// As [`should_consume`].
    pub fn should_consume(
        &self,
        registrations: &HashMap<i32, Hotkey>,
        raw: &RawInputEvent,
    ) -> bool {
        self.matching(registrations, raw)
            .any(|(_, hotkey)| hotkey.consume)
    }

    /// As [`matching_registration_snapshot`].
    pub fn snapshot(
        &self,
        registrations: &HashMap<i32, Hotkey>,
        raw: &RawInputEvent,
    ) -> HashMap<i32, Hotkey> {
        self.matching(registrations, raw)
            .map(|(id, hotkey)| (id, hotkey.clone()))
            .collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bindings_by_their_key_are_the_ones_holding_against_all_finds() {
        let binding = |key: &str, ctrl: bool, device: BindingDevice, consume: bool| Hotkey {
            ctrl,
            device,
            consume,
            ..Hotkey::new(key.to_string(), false, false, false, false)
        };
        let registrations: HashMap<i32, Hotkey> = [
            binding("ENTER", false, BindingDevice::Keyboard, true),
            // (By its other name, and in small letters.)
            binding("return", true, BindingDevice::Keyboard, false),
            binding(" Esc ", false, BindingDevice::Keyboard, true),
            binding("A", true, BindingDevice::Keyboard, true),
            binding("a", false, BindingDevice::Keyboard, false),
            binding("X1", true, BindingDevice::Mouse, true),
            binding("F5", false, BindingDevice::Keyboard, true),
            binding("no such key", false, BindingDevice::Keyboard, true),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, hotkey)| (id as i32, hotkey))
        .collect();
        let by_key = ByKey::of(&registrations);
        for key in ["ENTER", "ESCAPE", "A", "B", "X1", "F5", "NO SUCH KEY", ""] {
            for ctrl in [false, true] {
                for device in [BindingDevice::Keyboard, BindingDevice::Mouse] {
                    let raw = RawInputEvent {
                        key: key.to_string(),
                        ctrl,
                        alt: false,
                        shift: false,
                        win: false,
                        device,
                        down: true,
                        momentary: false,
                    };
                    assert_eq!(
                        by_key.should_consume(&registrations, &raw),
                        should_consume(&registrations, &raw),
                        "{raw:?}"
                    );
                    assert_eq!(
                        by_key.snapshot(&registrations, &raw),
                        matching_registration_snapshot(&registrations, &raw),
                        "{raw:?}"
                    );
                }
            }
        }
        // And some of them are found at all: the test holds something.
        let enter = RawInputEvent {
            key: "ENTER".to_string(),
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            device: BindingDevice::Keyboard,
            down: true,
            momentary: false,
        };
        assert_eq!(by_key.snapshot(&registrations, &enter).len(), 1);
    }
}
