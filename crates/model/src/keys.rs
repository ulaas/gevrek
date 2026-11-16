//! Key names used in bindings, and their virtual-key codes.
//! (Phase 2 step 2 makes this backend-neutral.)

pub fn key_name_to_vk(key: &str) -> Option<u16> {
    let key = key.trim().to_ascii_uppercase();
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_alphanumeric() {
            return Some(byte as u16);
        }
    }
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
    {
        if (1..=24).contains(&number) {
            return Some(0x6F + number);
        }
    }
    match key.as_str() {
        "SPACE" => Some(0x20),
        "ENTER" | "RETURN" => Some(0x0D),
        "TAB" => Some(0x09),
        "BACKSPACE" => Some(0x08),
        "ESCAPE" | "ESC" => Some(0x1B),
        "DELETE" => Some(0x2E),
        "INSERT" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" => Some(0x21),
        "PAGEDOWN" => Some(0x22),
        "UP" => Some(0x26),
        "DOWN" => Some(0x28),
        "LEFT" => Some(0x25),
        "RIGHT" => Some(0x27),
        "PRINTSCREEN" => Some(0x2C),
        "PAUSE" => Some(0x13),
        "CAPSLOCK" => Some(0x14),
        "NUMLOCK" => Some(0x90),
        "SCROLLLOCK" => Some(0x91),
        "MINUS" | "OEM_MINUS" => Some(0xBD),
        "EQUALS" | "OEM_PLUS" => Some(0xBB),
        "COMMA" | "OEM_COMMA" => Some(0xBC),
        "PERIOD" | "OEM_PERIOD" => Some(0xBE),
        "SEMICOLON" | "OEM1" => Some(0xBA),
        "SLASH" | "OEM2" => Some(0xBF),
        "BACKSLASH" | "OEM5" => Some(0xDC),
        "QUOTE" | "OEM7" => Some(0xDE),
        "BACKTICK" | "OEM3" => Some(0xC0),
        "LEFTBRACKET" | "OEM4" => Some(0xDB),
        "RIGHTBRACKET" | "OEM6" => Some(0xDD),
        // Keys Windows has no virtual-key code for (its firmware or drivers
        // handle them): numbers above every real code, so a binding on one
        // is valid everywhere and simply never fires where the key does
        // not reach the window manager.
        "BRIGHTNESS_UP" => Some(0x101),
        "BRIGHTNESS_DOWN" => Some(0x102),
        "MIC_MUTE" => Some(0x103),
        "VOLUME_MUTE" => Some(0xAD),
        "VOLUME_DOWN" => Some(0xAE),
        "VOLUME_UP" => Some(0xAF),
        "MEDIA_NEXT" => Some(0xB0),
        "MEDIA_PREVIOUS" => Some(0xB1),
        "MEDIA_STOP" => Some(0xB2),
        "MEDIA_PLAY_PAUSE" => Some(0xB3),
        _ => None,
    }
}

/// The one name of a key, whichever of its names was written: `RETURN`
/// is `ENTER`, `ESC` is `ESCAPE`, `OEM_MINUS` is `MINUS`. Two bindings on
/// one key are then known to be on one key, and a binding written with a
/// key's other name is the key the keyboard tells of (which it tells of
/// by this name). A name that is no key's is given back in capitals.
pub fn canonical_key_name(key: &str) -> String {
    let written = key.trim().to_ascii_uppercase();
    key_name_to_vk(&written)
        .and_then(vk_to_key_name)
        .unwrap_or(written)
}

pub fn vk_to_key_name(vk: u16) -> Option<String> {
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from_u32(vk as u32).map(|value| value.to_string());
    }
    if (0x70..=0x87).contains(&vk) {
        return Some(format!("F{}", vk - 0x6F));
    }
    let name = match vk {
        0x20 => "SPACE",
        0x0D => "ENTER",
        0x09 => "TAB",
        0x08 => "BACKSPACE",
        0x1B => "ESCAPE",
        0x2E => "DELETE",
        0x2D => "INSERT",
        0x24 => "HOME",
        0x23 => "END",
        0x21 => "PAGEUP",
        0x22 => "PAGEDOWN",
        0x26 => "UP",
        0x28 => "DOWN",
        0x25 => "LEFT",
        0x27 => "RIGHT",
        0x2C => "PRINTSCREEN",
        0x13 => "PAUSE",
        0x14 => "CAPSLOCK",
        0x90 => "NUMLOCK",
        0x91 => "SCROLLLOCK",
        0xBD => "MINUS",
        0xBB => "EQUALS",
        0xBC => "COMMA",
        0xBE => "PERIOD",
        0xBA => "SEMICOLON",
        0xBF => "SLASH",
        0xDC => "BACKSLASH",
        0xDE => "QUOTE",
        0xC0 => "BACKTICK",
        0xDB => "LEFTBRACKET",
        0xDD => "RIGHTBRACKET",
        0x101 => "BRIGHTNESS_UP",
        0x102 => "BRIGHTNESS_DOWN",
        0x103 => "MIC_MUTE",
        0xAD => "VOLUME_MUTE",
        0xAE => "VOLUME_DOWN",
        0xAF => "VOLUME_UP",
        0xB0 => "MEDIA_NEXT",
        0xB1 => "MEDIA_PREVIOUS",
        0xB2 => "MEDIA_STOP",
        0xB3 => "MEDIA_PLAY_PAUSE",
        // Modifier keys are deliberately named for state tracking but are not
        // normally used as the primary key of a chord.
        0x10 | 0xA0 | 0xA1 => "SHIFT",
        0x11 | 0xA2 | 0xA3 => "CTRL",
        0x12 | 0xA4 | 0xA5 => "ALT",
        0x5B | 0x5C => "WIN",
        _ => return None,
    };
    Some(name.to_string())
}
