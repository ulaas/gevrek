//! What AppKit says of the keyboard and the mouse, as egui hears it.

use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// The egui key at a place on a Mac keyboard (`kVK_*` of Carbon's
/// Events.h): named by the place on a US keyboard, as gevrek's bindings
/// are on macOS.
pub fn key_of(code: u16) -> Option<egui::Key> {
    use egui::Key;
    Some(match code {
        0x00 => Key::A,
        0x0B => Key::B,
        0x08 => Key::C,
        0x02 => Key::D,
        0x0E => Key::E,
        0x03 => Key::F,
        0x05 => Key::G,
        0x04 => Key::H,
        0x22 => Key::I,
        0x26 => Key::J,
        0x28 => Key::K,
        0x25 => Key::L,
        0x2E => Key::M,
        0x2D => Key::N,
        0x1F => Key::O,
        0x23 => Key::P,
        0x0C => Key::Q,
        0x0F => Key::R,
        0x01 => Key::S,
        0x11 => Key::T,
        0x20 => Key::U,
        0x09 => Key::V,
        0x0D => Key::W,
        0x07 => Key::X,
        0x10 => Key::Y,
        0x06 => Key::Z,
        0x1D => Key::Num0,
        0x12 => Key::Num1,
        0x13 => Key::Num2,
        0x14 => Key::Num3,
        0x15 => Key::Num4,
        0x17 => Key::Num5,
        0x16 => Key::Num6,
        0x1A => Key::Num7,
        0x1C => Key::Num8,
        0x19 => Key::Num9,
        0x24 | 0x4C => Key::Enter,
        0x35 => Key::Escape,
        0x30 => Key::Tab,
        0x33 => Key::Backspace,
        0x75 => Key::Delete,
        0x31 => Key::Space,
        0x73 => Key::Home,
        0x77 => Key::End,
        0x74 => Key::PageUp,
        0x79 => Key::PageDown,
        0x7B => Key::ArrowLeft,
        0x7C => Key::ArrowRight,
        0x7D => Key::ArrowDown,
        0x7E => Key::ArrowUp,
        0x1B => Key::Minus,
        0x18 => Key::Equals,
        0x2B => Key::Comma,
        0x2F => Key::Period,
        0x2C => Key::Slash,
        0x2A => Key::Backslash,
        0x29 => Key::Semicolon,
        0x27 => Key::Quote,
        0x21 => Key::OpenBracket,
        0x1E => Key::CloseBracket,
        0x32 => Key::Backtick,
        0x7A => Key::F1,
        0x78 => Key::F2,
        0x63 => Key::F3,
        0x76 => Key::F4,
        0x60 => Key::F5,
        0x61 => Key::F6,
        0x62 => Key::F7,
        0x64 => Key::F8,
        0x65 => Key::F9,
        0x6D => Key::F10,
        0x67 => Key::F11,
        0x6F => Key::F12,
        _ => return None,
    })
}

/// The key as egui names it for shortcuts: by the letter it writes in
/// the keyboard's layout (`written`: what it writes with no modifier
/// held), as on the other systems; every other key by its place.
pub fn logical_key(code: u16, written: Option<&str>) -> Option<egui::Key> {
    let letter = written
        .and_then(|text| {
            let mut characters = text.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) if character.is_ascii_alphabetic() => Some(character),
                _ => None,
            }
        })
        .and_then(|letter| egui::Key::from_name(&letter.to_ascii_uppercase().to_string()));
    letter.or_else(|| key_of(code))
}

/// The modifiers held, as egui has them: Command is the key a Mac's
/// shortcuts are on (`command`).
pub fn modifiers_of(flags: NSEventModifierFlags) -> egui::Modifiers {
    let command = flags.contains(NSEventModifierFlags::Command);
    egui::Modifiers {
        alt: flags.contains(NSEventModifierFlags::Option),
        ctrl: flags.contains(NSEventModifierFlags::Control),
        shift: flags.contains(NSEventModifierFlags::Shift),
        mac_cmd: command,
        command,
    }
}

/// The text on the pasteboard, to paste.
pub fn pasted() -> Option<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    // SAFETY: the system's constant.
    let kind = unsafe { NSPasteboardTypeString };
    pasteboard
        .stringForType(kind)
        .map(|text| text.to_string())
        .filter(|text| !text.is_empty())
}

/// Put text on the pasteboard, as copied.
pub fn copy(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: the system's constant.
    let kind = unsafe { NSPasteboardTypeString };
    pasteboard.setString_forType(&NSString::from_str(text), kind);
}

/// The egui button of a mouse button, by AppKit's number of it.
pub fn button_of(number: isize) -> Option<egui::PointerButton> {
    Some(match number {
        0 => egui::PointerButton::Primary,
        1 => egui::PointerButton::Secondary,
        2 => egui::PointerButton::Middle,
        3 => egui::PointerButton::Extra1,
        4 => egui::PointerButton::Extra2,
        _ => return None,
    })
}

/// The pointer AppKit shows for one egui asks for.
#[allow(deprecated)] // (The resize pointers' newer names ask for a direction egui does not say.)
pub fn cursor_of(icon: egui::CursorIcon) -> Retained<NSCursor> {
    use egui::CursorIcon as Icon;
    match icon {
        Icon::Text | Icon::VerticalText => NSCursor::IBeamCursor(),
        Icon::PointingHand => NSCursor::pointingHandCursor(),
        Icon::Crosshair | Icon::Cell => NSCursor::crosshairCursor(),
        Icon::Grab => NSCursor::openHandCursor(),
        Icon::Grabbing => NSCursor::closedHandCursor(),
        Icon::NotAllowed | Icon::NoDrop => NSCursor::operationNotAllowedCursor(),
        Icon::ResizeHorizontal | Icon::ResizeColumn | Icon::ResizeEast | Icon::ResizeWest => {
            NSCursor::resizeLeftRightCursor()
        }
        Icon::ResizeVertical | Icon::ResizeRow | Icon::ResizeNorth | Icon::ResizeSouth => {
            NSCursor::resizeUpDownCursor()
        }
        _ => NSCursor::arrowCursor(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_by_their_place() {
        assert_eq!(key_of(0x00), Some(egui::Key::A));
        assert_eq!(key_of(0x24), Some(egui::Key::Enter));
        assert_eq!(key_of(0x35), Some(egui::Key::Escape));
        assert_eq!(key_of(0x7D), Some(egui::Key::ArrowDown));
        assert_eq!(key_of(0x3A), None);
    }

    #[test]
    fn a_letter_is_the_one_the_layout_writes() {
        // German: the key at the US layout's Y writes z.
        assert_eq!(logical_key(0x10, Some("z")), Some(egui::Key::Z));
        assert_eq!(logical_key(0x10, Some("y")), Some(egui::Key::Y));
        // What writes no letter is named by its place.
        assert_eq!(logical_key(0x12, Some("&")), Some(egui::Key::Num1));
        assert_eq!(logical_key(0x7B, Some("\u{F702}")), Some(egui::Key::ArrowLeft));
        assert_eq!(logical_key(0x00, None), Some(egui::Key::A));
    }
}
