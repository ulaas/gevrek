//! Letters named as the keyboard's layout writes them.
//!
//! A binding names its key as on Windows, where a letter's virtual-key code
//! is the key that writes that letter: `Win+Z` is the key marked Z on a
//! German keyboard too (where it is at the US layout's Y), and `Win+A` the
//! key marked A on a French one. So on a Mac a letter is found by asking
//! the layout in use what each key writes (Carbon's `UCKeyTranslate`);
//! every other key (digits, punctuation, the arrows, the function keys) is
//! named by its place, as on Windows' own layouts. A key whose place is a
//! letter that another key now writes is no letter (`None`), so no two
//! keys answer to one name.
//!
//! The layout is asked again when the selected one changes (looked at with
//! each key that is pressed: asking is cheap, and it must be the main
//! thread's, which the tap is on).

use crate::ffi::{self, CFTypeRef};
use std::collections::HashMap;
use std::ffi::c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFTypeRef;
    static kTISPropertyInputSourceID: CFTypeRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISCreateInputSourceList(properties: CFTypeRef, include_all_installed: bool) -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        length: *mut usize,
        text: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
}

/// `kUCKeyActionDisplay`: what the key shows, without a dead key's wait.
const ACTION_DISPLAY: u16 = 3;
/// `kUCKeyTranslateNoDeadKeysBit`.
const NO_DEAD_KEYS: u32 = 1;

/// The keys of the block of letters, digits and punctuation.
const WRITING_KEYS: std::ops::RangeInclusive<u16> = 0x00..=0x32;

/// What the layout in use makes of the keys.
#[derive(Debug, Default)]
pub struct Layout {
    /// The layout's name (`com.apple.keylayout.German`).
    id: Option<String>,
    /// The keys named otherwise than by their place: by the letter they
    /// write, or not at all.
    keys: HashMap<u16, Option<u16>>,
}

impl Layout {
    /// Ask the layout again if another one was selected since.
    pub fn refresh(&mut self) {
        if let Some(source) = Source::current() {
            self.read(&source);
        }
    }

    /// An installed layout by its name (`com.apple.keylayout.German`),
    /// selected or not: to see what it would make of the keys.
    pub fn named(id: &str) -> Option<Self> {
        let mut layout = Self::default();
        layout.read(&Source::named(id)?);
        Some(layout)
    }

    fn read(&mut self, source: &Source) {
        let id = source.id();
        if id.is_some() && id == self.id {
            return;
        }
        let written: Vec<(u16, char)> = WRITING_KEYS
            .filter_map(|code| source.writes(code).map(|written| (code, written)))
            .collect();
        self.keys = letters(&written, crate::keys::place_key);
        log::debug!(
            "keyboard layout {}: {} keys named by what they write",
            id.as_deref().unwrap_or("?"),
            self.keys.len()
        );
        self.id = id;
    }

    /// The virtual-key code a binding knows the key with this macOS key
    /// code by.
    pub fn virtual_key(&self, code: u16) -> Option<u16> {
        match self.keys.get(&code) {
            Some(named) => *named,
            None => crate::keys::place_key(code),
        }
    }
}

/// The keys whose names are not their places', from what each key writes:
/// a key that writes a letter is that letter's; a key whose place is a
/// letter that another key writes is none; a key whose place is a letter
/// nobody else writes keeps it.
pub fn letters(
    written: &[(u16, char)],
    place: impl Fn(u16) -> Option<u16>,
) -> HashMap<u16, Option<u16>> {
    let mut keys = HashMap::new();
    for &(code, character) in written {
        if character.is_ascii_alphabetic() {
            keys.insert(code, Some(character.to_ascii_uppercase() as u16));
        }
    }
    let claimed: Vec<u16> = keys.values().flatten().copied().collect();
    for &(code, _) in written {
        if keys.contains_key(&code) {
            continue;
        }
        if let Some(letter) = place(code).filter(|key| (b'A' as u16..=b'Z' as u16).contains(key)) {
            if claimed.contains(&letter) {
                keys.insert(code, None);
            }
        }
    }
    // (A key that writes the letter of its own place is named as before.)
    keys.retain(|code, named| *named != place(*code));
    keys
}

/// The selected keyboard layout.
struct Source(CFTypeRef);

impl Source {
    fn named(id: &str) -> Option<Self> {
        let key = unsafe { kTISPropertyInputSourceID };
        let value = ffi::Cf::string(id);
        // SAFETY: a dictionary of one key, and the list it selects (owned,
        // released here); its first source retained for `Self`.
        unsafe {
            let properties = ffi::CFDictionaryCreate(
                std::ptr::null(),
                &key,
                &value.as_ptr(),
                1,
                &ffi::kCFTypeDictionaryKeyCallBacks,
                &ffi::kCFTypeDictionaryValueCallBacks,
            );
            let list = TISCreateInputSourceList(properties, true);
            ffi::CFRelease(properties);
            if list.is_null() {
                return None;
            }
            let first =
                (ffi::CFArrayGetCount(list) > 0).then(|| ffi::CFArrayGetValueAtIndex(list, 0));
            let source = first.map(|source| Self(ffi::CFRetain(source)));
            ffi::CFRelease(list);
            source
        }
    }

    fn current() -> Option<Self> {
        // SAFETY: an owned reference, released when dropped.
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        (!source.is_null()).then_some(Self(source))
    }

    fn id(&self) -> Option<String> {
        // SAFETY: a property the source holds (not owned).
        unsafe { ffi::string_of(TISGetInputSourceProperty(self.0, kTISPropertyInputSourceID)) }
    }

    /// The character a key writes with nothing held.
    fn writes(&self, code: u16) -> Option<char> {
        // SAFETY: the layout's data is held by the source while it lives;
        // the translation writes at most the room it is given.
        unsafe {
            let data = TISGetInputSourceProperty(self.0, kTISPropertyUnicodeKeyLayoutData);
            if data.is_null() {
                return None;
            }
            let layout = CFDataGetBytePtr(data) as *const c_void;
            let mut dead = 0u32;
            let mut text = [0u16; 4];
            let mut length = 0usize;
            let status = UCKeyTranslate(
                layout,
                code,
                ACTION_DISPLAY,
                0,
                u32::from(LMGetKbdType()),
                NO_DEAD_KEYS,
                &mut dead,
                text.len(),
                &mut length,
                text.as_mut_ptr(),
            );
            if status != 0 || length != 1 {
                return None;
            }
            char::from_u32(u32::from(text[0]))
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: the reference `current` took.
        unsafe { ffi::CFRelease(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The US layout's places of the keys these tests use.
    fn place(code: u16) -> Option<u16> {
        Some(match code {
            0x00 => b'A' as u16,
            0x0C => b'Q' as u16,
            0x06 => b'Z' as u16,
            0x10 => b'Y' as u16,
            0x22 => b'I' as u16,
            0x27 => 0xDE,
            0x29 => 0xBA,
            0x2E => b'M' as u16,
            _ => return None,
        })
    }

    #[test]
    fn the_us_layout_names_every_key_by_its_place() {
        let written = [
            (0x00, 'a'),
            (0x0C, 'q'),
            (0x06, 'z'),
            (0x10, 'y'),
            (0x29, ';'),
        ];
        assert!(letters(&written, place).is_empty());
    }

    #[test]
    fn a_german_keyboard_has_z_and_y_where_they_are_marked() {
        let written = [(0x06, 'y'), (0x10, 'z'), (0x00, 'a')];
        let keys = letters(&written, place);
        assert_eq!(keys.get(&0x06), Some(&Some(b'Y' as u16)));
        assert_eq!(keys.get(&0x10), Some(&Some(b'Z' as u16)));
        assert_eq!(keys.get(&0x00), None);
    }

    #[test]
    fn a_french_keyboard_has_m_beside_l_and_no_second_m() {
        let written = [(0x00, 'q'), (0x0C, 'a'), (0x29, 'm'), (0x2E, ',')];
        let keys = letters(&written, place);
        assert_eq!(keys.get(&0x00), Some(&Some(b'Q' as u16)));
        assert_eq!(keys.get(&0x0C), Some(&Some(b'A' as u16)));
        assert_eq!(keys.get(&0x29), Some(&Some(b'M' as u16)));
        // The key at the US layout's M writes a comma: it is no M.
        assert_eq!(keys.get(&0x2E), Some(&None));
    }

    #[test]
    fn a_letter_nobody_else_writes_stays_at_its_place() {
        // Turkish Q: the key at I writes ı, the key at ' writes i.
        let written = [(0x22, 'ı'), (0x27, 'i')];
        let keys = letters(&written, place);
        assert_eq!(keys.get(&0x27), Some(&Some(b'I' as u16)));
        assert_eq!(keys.get(&0x22), Some(&None));
    }
}
