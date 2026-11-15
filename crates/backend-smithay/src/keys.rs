//! Key names: from the keysym of a pressed key to the name bindings use
//! (the same names as on Windows, see `gevrek_model::keys`).

/// The binding name of a key, from its keysym without modifiers applied
/// (so Shift+1 is "1", not "exclam"). `None` for keys bindings cannot name.
pub fn key_name(keysym: u32) -> Option<String> {
    // Latin letters and digits are their own character codes.
    if let Some(character) = char::from_u32(keysym) {
        if character.is_ascii_alphanumeric() {
            return Some(character.to_ascii_uppercase().to_string());
        }
    }
    // F1 to F24 are consecutive from XK_F1.
    if (0xffbe..=0xffd5).contains(&keysym) {
        return Some(format!("F{}", keysym - 0xffbe + 1));
    }
    let name = match keysym {
        0x0020 => "SPACE",
        0xff0d | 0xff8d => "ENTER", // Return, KP_Enter
        0xff09 => "TAB",
        0xff08 => "BACKSPACE",
        0xff1b => "ESCAPE",
        0xffff => "DELETE",
        0xff63 => "INSERT",
        0xff50 => "HOME",
        0xff57 => "END",
        0xff55 => "PAGEUP",
        0xff56 => "PAGEDOWN",
        0xff52 => "UP",
        0xff54 => "DOWN",
        0xff51 => "LEFT",
        0xff53 => "RIGHT",
        0xff61 => "PRINTSCREEN",
        0xff13 => "PAUSE",
        0xffe5 => "CAPSLOCK",
        0xff7f => "NUMLOCK",
        0xff14 => "SCROLLLOCK",
        0x002d => "MINUS",
        0x003d => "EQUALS",
        0x002c => "COMMA",
        0x002e => "PERIOD",
        0x003b => "SEMICOLON",
        0x002f => "SLASH",
        0x005c => "BACKSLASH",
        0x0027 => "QUOTE",
        0x0060 => "BACKTICK",
        0x005b => "LEFTBRACKET",
        0x005d => "RIGHTBRACKET",
        0x1008_ff02 => "BRIGHTNESS_UP",
        0x1008_ff03 => "BRIGHTNESS_DOWN",
        0x1008_ffb2 => "MIC_MUTE",
        0x1008_ff31 => "MEDIA_PLAY_PAUSE", // XF86AudioPause
        0x1008_ff12 => "VOLUME_MUTE",
        0x1008_ff11 => "VOLUME_DOWN",
        0x1008_ff13 => "VOLUME_UP",
        0x1008_ff17 => "MEDIA_NEXT",
        0x1008_ff16 => "MEDIA_PREVIOUS",
        0x1008_ff15 => "MEDIA_STOP",
        0x1008_ff14 => "MEDIA_PLAY_PAUSE",
        _ => return None,
    };
    Some(name.to_string())
}

/// The one spelling of a key name: a binding may say "RETURN" or "ESC",
/// a pressed key is always "ENTER" or "ESCAPE".
pub fn canonical(name: &str) -> String {
    gevrek_model::keys::key_name_to_vk(name)
        .and_then(gevrek_model::keys::vk_to_key_name)
        .unwrap_or_else(|| name.trim().to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_keys_have_the_names_bindings_use() {
        assert_eq!(key_name(0x61).as_deref(), Some("A")); // XK_a
        assert_eq!(key_name(0x41).as_deref(), Some("A")); // XK_A
        assert_eq!(key_name(0x39).as_deref(), Some("9"));
        assert_eq!(key_name(0xffbe).as_deref(), Some("F1"));
        assert_eq!(key_name(0xffc6).as_deref(), Some("F9"));
        assert_eq!(key_name(0xffd5).as_deref(), Some("F24"));
        assert_eq!(key_name(0xff0d).as_deref(), Some("ENTER"));
        assert_eq!(key_name(0xff55).as_deref(), Some("PAGEUP"));
        assert_eq!(key_name(0x60).as_deref(), Some("BACKTICK"));
        assert_eq!(key_name(0xffe1), None, "a modifier is not a binding key");
    }

    #[test]
    fn every_name_a_key_can_have_is_one_the_model_knows() {
        let keysyms = (0x20..=0x7e)
            .chain(0xff00..=0xffff)
            .chain(0x1008_ff10..=0x1008_ff20);
        for keysym in keysyms {
            if let Some(name) = key_name(keysym) {
                assert!(
                    gevrek_model::keys::key_name_to_vk(&name).is_some(),
                    "{name} (keysym {keysym:#x})"
                );
                assert_eq!(canonical(&name), name);
            }
        }
    }

    #[test]
    fn aliases_in_bindings_match_the_pressed_key() {
        assert_eq!(canonical("return"), "ENTER");
        assert_eq!(canonical("Esc"), "ESCAPE");
        assert_eq!(canonical(" f9 "), "F9");
    }
}
