//! What AppKit says of the keyboard and the mouse, as egui hears it.

use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSEventModifierFlags};

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

/// The text a key writes, as egui takes it: none for a key that writes
/// nothing (the arrows and the function keys write characters of the
/// private range), nor for one held with Command or Control (a shortcut).
pub fn text_of(characters: &str, modifiers: egui::Modifiers) -> Option<String> {
    if modifiers.command || modifiers.ctrl {
        return None;
    }
    let text: String = characters
        .chars()
        .filter(|character| !character.is_control() && !('\u{F700}'..='\u{F8FF}').contains(character))
        .collect();
    (!text.is_empty()).then_some(text)
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
    fn a_key_held_with_command_writes_nothing_and_an_arrow_never_does() {
        let plain = egui::Modifiers::default();
        assert_eq!(text_of("a", plain).as_deref(), Some("a"));
        assert_eq!(text_of("ç", plain).as_deref(), Some("ç"));
        assert_eq!(text_of("\u{F702}", plain), None);
        assert_eq!(text_of("\r", plain), None);
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        };
        assert_eq!(text_of("c", command), None);
    }
}
