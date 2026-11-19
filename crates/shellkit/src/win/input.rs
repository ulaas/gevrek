//! What Windows says about the mouse and the keyboard, in egui's words.
//! Nothing here asks Windows anything: it is given the numbers of a
//! message and answers what they mean.

use windows::Win32::UI::Input::KeyboardAndMouse as keys;
use windows::Win32::UI::WindowsAndMessaging as messages;

/// The toolkit's name for a key, by the number Windows gives the key
/// (not the letter: that comes separately, as text).
pub fn key_of(key: u16) -> Option<egui::Key> {
    use egui::Key;
    let key = keys::VIRTUAL_KEY(key);
    Some(match key {
        keys::VK_RETURN => Key::Enter,
        keys::VK_ESCAPE => Key::Escape,
        keys::VK_BACK => Key::Backspace,
        keys::VK_TAB => Key::Tab,
        keys::VK_DELETE => Key::Delete,
        keys::VK_INSERT => Key::Insert,
        keys::VK_HOME => Key::Home,
        keys::VK_END => Key::End,
        keys::VK_LEFT => Key::ArrowLeft,
        keys::VK_RIGHT => Key::ArrowRight,
        keys::VK_UP => Key::ArrowUp,
        keys::VK_DOWN => Key::ArrowDown,
        keys::VK_PRIOR => Key::PageUp,
        keys::VK_NEXT => Key::PageDown,
        keys::VK_SPACE => Key::Space,
        keys::VK_OEM_MINUS | keys::VK_SUBTRACT => Key::Minus,
        keys::VK_OEM_PLUS => Key::Equals,
        keys::VK_ADD => Key::Plus,
        keys::VK_OEM_COMMA => Key::Comma,
        keys::VK_OEM_PERIOD | keys::VK_DECIMAL => Key::Period,
        other => {
            let number = other.0;
            let name = match number {
                // Letters and digits have the numbers of their characters.
                0x30..=0x39 | 0x41..=0x5A => char::from(number as u8).to_string(),
                // The digits of the number block.
                0x60..=0x69 => char::from(b'0' + (number - 0x60) as u8).to_string(),
                // F1 to F24.
                0x70..=0x87 => format!("F{}", number - 0x70 + 1),
                _ => return None,
            };
            Key::from_name(&name)?
        }
    })
}

/// Whether a key is the one that asks for a menu (between the right
/// Windows key and Ctrl on most keyboards).
pub fn is_menu_key(key: u16) -> bool {
    key == keys::VK_APPS.0
}

/// The keys held with a key or a click. Windows has no key of its own
/// for commands: Ctrl is it, as on Linux.
pub fn modifiers(shift: bool, ctrl: bool, alt: bool) -> egui::Modifiers {
    egui::Modifiers {
        alt,
        ctrl,
        shift,
        mac_cmd: false,
        command: ctrl,
    }
}

/// A message about a key says in its second number whether the key was
/// down already: held, it repeats.
pub fn is_repeat(lparam: isize) -> bool {
    lparam & (1 << 30) != 0
}

/// Where the pointer is on the surface, in egui's points, from a mouse
/// message's second number (two signed 16-bit numbers of pixels: on a
/// held button the pointer may be left of or above the surface).
pub fn point(lparam: isize, scale: f32) -> egui::Pos2 {
    let x = (lparam & 0xFFFF) as u16 as i16;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16;
    egui::pos2(f32::from(x) / scale, f32::from(y) / scale)
}

/// Where the pointer is on a surface, in egui's points, from a wheel
/// message's second number: there the two numbers are the pointer's place
/// on the desktop (in pixels, signed: a display left of or above the main
/// one), and `left` and `top` are where the surface's corner is on it.
pub fn point_on(lparam: isize, left: i32, top: i32, scale: f32) -> egui::Pos2 {
    let x = i32::from((lparam & 0xFFFF) as u16 as i16);
    let y = i32::from(((lparam >> 16) & 0xFFFF) as u16 as i16);
    egui::pos2((x - left) as f32 / scale, (y - top) as f32 / scale)
}

/// The button a mouse message is about, and whether it went down.
pub fn button(message: u32, wparam: usize) -> Option<(egui::PointerButton, bool)> {
    use egui::PointerButton;
    // The two buttons at the side share their messages: which one it was
    // is in the upper half of the first number.
    let side = if (wparam >> 16) & 0xFFFF == 2 {
        PointerButton::Extra2
    } else {
        PointerButton::Extra1
    };
    Some(match message {
        messages::WM_LBUTTONDOWN | messages::WM_LBUTTONDBLCLK => (PointerButton::Primary, true),
        messages::WM_LBUTTONUP => (PointerButton::Primary, false),
        messages::WM_RBUTTONDOWN | messages::WM_RBUTTONDBLCLK => (PointerButton::Secondary, true),
        messages::WM_RBUTTONUP => (PointerButton::Secondary, false),
        messages::WM_MBUTTONDOWN | messages::WM_MBUTTONDBLCLK => (PointerButton::Middle, true),
        messages::WM_MBUTTONUP => (PointerButton::Middle, false),
        messages::WM_XBUTTONDOWN | messages::WM_XBUTTONDBLCLK => (side, true),
        messages::WM_XBUTTONUP => (side, false),
        _ => return None,
    })
}

/// What egui counts as a double click, from what the person set in
/// Windows: the longest time between the two presses in seconds, and how
/// far apart they may be in points.
///
/// The toolkit's own values are a third of a second and six points,
/// whatever the system says; Windows' time is half a second unless the
/// person chose another, and a double press that Explorer takes must be
/// one on a surface of the shell too. `time_ms` is `GetDoubleClickTime`,
/// `across` and `down` the sides of Windows' rectangle around the first
/// press (`SM_CXDOUBLECLK`, `SM_CYDOUBLECLK`, in pixels), `scale` the
/// pixels to a point on the surface's display.
///
/// The distance is never less than the toolkit's own six points: the
/// toolkit uses the same number for how far a press may move before it
/// is a drag, and Windows' rectangle (two pixels to each side) would
/// make every unsteady click one.
pub fn double_click(time_ms: u32, across: i32, down: i32, scale: f32) -> (f64, f32) {
    // (A time of nothing, or of minutes, is no setting anyone made.)
    let delay = f64::from(time_ms.clamp(100, 5000)) / 1000.0;
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let half = across.max(down).max(0) as f32 / 2.0 / scale;
    (delay, half.max(6.0))
}

/// How far a wheel message scrolls, in lines: a notch of the wheel is
/// 120 of Windows' units, a finer wheel or a touchpad sends parts of it.
/// Away from the user is up; a wheel tilted to the right shows what is
/// to the right, which for egui is a movement to the left.
pub fn wheel(wparam: usize, horizontal: bool) -> egui::Vec2 {
    let turned = f32::from(((wparam >> 16) & 0xFFFF) as u16 as i16) / 120.0;
    if horizontal {
        egui::vec2(-turned, 0.0)
    } else {
        egui::vec2(0.0, turned)
    }
}

/// Puts together the characters Windows sends in two halves.
///
/// Text arrives in units of 16 bits; a character beyond the first 65536
/// (an emoji, a rare Han character) comes as two messages.
#[derive(Debug, Default)]
pub struct Utf16 {
    first_half: Option<u16>,
}

impl Utf16 {
    /// The character that is complete with this unit, if one is. A half
    /// without its other half is dropped: it means nothing.
    pub fn push(&mut self, unit: u16) -> Option<char> {
        match unit {
            0xD800..=0xDBFF => {
                self.first_half = Some(unit);
                None
            }
            0xDC00..=0xDFFF => {
                let first = self.first_half.take()?;
                char::decode_utf16([first, unit]).next()?.ok()
            }
            _ => {
                self.first_half = None;
                char::from_u32(u32::from(unit))
            }
        }
    }
}

/// Whether a character is text to write: Enter, Backspace, Escape and
/// the letters typed with Ctrl arrive as characters too, and are keys.
pub fn is_text(letter: char) -> bool {
    !letter.is_control()
}

/// The pointer Windows has for the one egui asks for, by the number of
/// the system's pointer (`IDC_ARROW` and the others); `None` hides it.
pub fn pointer_of(icon: egui::CursorIcon) -> Option<u16> {
    use egui::CursorIcon as Icon;
    const ARROW: u16 = 32512;
    const TEXT: u16 = 32513;
    const WAIT: u16 = 32514;
    const CROSS: u16 = 32515;
    const DIAGONAL_DOWN: u16 = 32642;
    const DIAGONAL_UP: u16 = 32643;
    const ACROSS: u16 = 32644;
    const UPRIGHT: u16 = 32645;
    const EVERY_WAY: u16 = 32646;
    const NO: u16 = 32648;
    const HAND: u16 = 32649;
    const WORKING: u16 = 32650;
    const HELP: u16 = 32651;
    Some(match icon {
        Icon::None => return None,
        Icon::Text | Icon::VerticalText => TEXT,
        Icon::Wait => WAIT,
        Icon::Progress => WORKING,
        Icon::Crosshair | Icon::Cell => CROSS,
        Icon::PointingHand | Icon::Grab | Icon::Grabbing => HAND,
        Icon::Help => HELP,
        Icon::NotAllowed | Icon::NoDrop => NO,
        Icon::Move | Icon::AllScroll => EVERY_WAY,
        Icon::ResizeHorizontal | Icon::ResizeEast | Icon::ResizeWest | Icon::ResizeColumn => ACROSS,
        Icon::ResizeVertical | Icon::ResizeNorth | Icon::ResizeSouth | Icon::ResizeRow => UPRIGHT,
        Icon::ResizeNwSe | Icon::ResizeNorthWest | Icon::ResizeSouthEast => DIAGONAL_DOWN,
        Icon::ResizeNeSw | Icon::ResizeNorthEast | Icon::ResizeSouthWest => DIAGONAL_UP,
        _ => ARROW,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_click_is_as_slow_as_windows_lets_it_be() {
        // As Windows comes: half a second, a rectangle of four pixels.
        assert_eq!(double_click(500, 4, 4, 1.0), (0.5, 6.0));
        // The person made it slower, or faster.
        assert_eq!(double_click(900, 4, 4, 1.0).0, 0.9);
        assert_eq!(double_click(200, 4, 4, 1.0).0, 0.2);
        // No setting at all, and one nobody made.
        assert_eq!(double_click(0, 4, 4, 1.0).0, 0.1);
        assert_eq!(double_click(u32::MAX, 4, 4, 1.0).0, 5.0);
    }

    #[test]
    fn the_two_presses_may_be_as_far_apart_as_windows_says_and_never_less_than_six_points() {
        // A wide rectangle (a setting for unsteady hands): half of its
        // longer side, in points of the display.
        assert_eq!(double_click(500, 40, 24, 1.0).1, 20.0);
        assert_eq!(double_click(500, 40, 24, 2.0).1, 10.0);
        assert_eq!(double_click(500, 30, 60, 1.5).1, 20.0);
        // Windows' own small one: the toolkit's six points stay.
        assert_eq!(double_click(500, 4, 4, 1.5).1, 6.0);
        assert_eq!(double_click(500, -4, 0, 1.0).1, 6.0);
        // A scale that is none counts as one.
        assert_eq!(double_click(500, 40, 40, 0.0).1, 20.0);
        assert_eq!(double_click(500, 40, 40, f32::NAN).1, 20.0);
    }

    #[test]
    fn keys_a_shell_surface_acts_on_have_the_toolkits_names() {
        assert_eq!(key_of(keys::VK_RETURN.0), Some(egui::Key::Enter));
        assert_eq!(key_of(keys::VK_ESCAPE.0), Some(egui::Key::Escape));
        assert_eq!(key_of(keys::VK_BACK.0), Some(egui::Key::Backspace));
        assert_eq!(key_of(keys::VK_DOWN.0), Some(egui::Key::ArrowDown));
        assert_eq!(key_of(keys::VK_PRIOR.0), Some(egui::Key::PageUp));
        assert_eq!(key_of(keys::VK_SPACE.0), Some(egui::Key::Space));
        assert_eq!(key_of(keys::VK_A.0), Some(egui::Key::A));
        assert_eq!(key_of(keys::VK_Z.0), Some(egui::Key::Z));
        assert_eq!(key_of(keys::VK_7.0), Some(egui::Key::Num7));
        assert_eq!(key_of(keys::VK_NUMPAD3.0), Some(egui::Key::Num3));
        assert_eq!(key_of(keys::VK_F1.0), Some(egui::Key::F1));
        assert_eq!(key_of(keys::VK_F12.0), Some(egui::Key::F12));
    }

    #[test]
    fn keys_that_only_change_others_are_no_keys_of_their_own() {
        assert_eq!(key_of(keys::VK_SHIFT.0), None);
        assert_eq!(key_of(keys::VK_CONTROL.0), None);
        assert_eq!(key_of(keys::VK_MENU.0), None);
        assert_eq!(key_of(keys::VK_LWIN.0), None);
    }

    #[test]
    fn the_key_for_a_menu_is_known_and_is_no_key_of_egui_s() {
        assert!(is_menu_key(keys::VK_APPS.0));
        assert!(!is_menu_key(keys::VK_F10.0));
        assert_eq!(key_of(keys::VK_APPS.0), None);
        assert_eq!(key_of(keys::VK_F10.0), Some(egui::Key::F10));
    }

    #[test]
    fn ctrl_is_the_key_for_commands() {
        let held = modifiers(false, true, false);
        assert!(held.ctrl && held.command && !held.shift && !held.alt && !held.mac_cmd);
        let held = modifiers(true, false, true);
        assert!(held.shift && held.alt && !held.ctrl && !held.command);
    }

    #[test]
    fn a_held_key_is_told_from_a_pressed_one() {
        assert!(!is_repeat(0x001E_0001));
        assert!(is_repeat(0x401E_0001));
    }

    #[test]
    fn the_pointers_place_is_in_points_of_the_scaled_surface() {
        let lparam = (30 << 16) | 150;
        assert_eq!(point(lparam, 1.0), egui::pos2(150.0, 30.0));
        assert_eq!(point(lparam, 1.5), egui::pos2(100.0, 20.0));
    }

    #[test]
    fn a_pointer_dragged_off_the_surface_is_left_of_or_above_it() {
        // -5 and -2 as Windows packs them.
        let lparam = ((0xFFFE_isize) << 16) | 0xFFFB;
        assert_eq!(point(lparam, 1.0), egui::pos2(-5.0, -2.0));
    }

    #[test]
    fn mouse_messages_name_their_button_and_its_direction() {
        use egui::PointerButton;
        assert_eq!(
            button(messages::WM_LBUTTONDOWN, 1),
            Some((PointerButton::Primary, true))
        );
        assert_eq!(
            button(messages::WM_LBUTTONUP, 0),
            Some((PointerButton::Primary, false))
        );
        assert_eq!(
            button(messages::WM_RBUTTONDOWN, 2),
            Some((PointerButton::Secondary, true))
        );
        assert_eq!(
            button(messages::WM_MBUTTONUP, 0),
            Some((PointerButton::Middle, false))
        );
        // The second press of a double click is a press like any other
        // (the toolkit counts double clicks itself, by `double_click`).
        assert_eq!(
            button(messages::WM_LBUTTONDBLCLK, 1),
            Some((PointerButton::Primary, true))
        );
        assert_eq!(
            button(messages::WM_XBUTTONDOWN, 1 << 16),
            Some((PointerButton::Extra1, true))
        );
        assert_eq!(
            button(messages::WM_XBUTTONUP, 2 << 16),
            Some((PointerButton::Extra2, false))
        );
        assert_eq!(button(messages::WM_MOUSEMOVE, 0), None);
    }

    #[test]
    fn a_wheel_message_says_where_the_pointer_is_on_the_surface() {
        let message = |x: i16, y: i16| {
            ((y as u16 as isize) << 16) | (x as u16 as isize)
        };
        // A surface at 2566, 6 of the desktop (a panel on a second display).
        assert_eq!(
            point_on(message(2600, 24), 2566, 6, 1.0),
            egui::pos2(34.0, 18.0)
        );
        // At twice the scale a point is two pixels.
        assert_eq!(
            point_on(message(2600, 24), 2566, 6, 2.0),
            egui::pos2(17.0, 9.0)
        );
        // A display to the left of the main one: the desktop's numbers
        // are below nothing there.
        assert_eq!(
            point_on(message(-1900, 30), -1920, 0, 1.0),
            egui::pos2(20.0, 30.0)
        );
    }

    #[test]
    fn a_notch_of_the_wheel_is_a_line() {
        let up = 120_usize << 16;
        assert_eq!(wheel(up, false), egui::vec2(0.0, 1.0));
        let down = ((-120_i16) as u16 as usize) << 16;
        assert_eq!(wheel(down, false), egui::vec2(0.0, -1.0));
        // A touchpad sends parts of a notch.
        let little = 30_usize << 16;
        assert_eq!(wheel(little, false), egui::vec2(0.0, 0.25));
    }

    #[test]
    fn a_wheel_tilted_right_moves_what_is_shown_to_the_left() {
        let right = 120_usize << 16;
        assert_eq!(wheel(right, true), egui::vec2(-1.0, 0.0));
        let left = ((-240_i16) as u16 as usize) << 16;
        assert_eq!(wheel(left, true), egui::vec2(2.0, 0.0));
    }

    #[test]
    fn a_character_in_two_halves_is_put_together() {
        let mut text = Utf16::default();
        assert_eq!(text.push(u16::from(b'a')), Some('a'));
        assert_eq!(text.push(0x011F), Some('ğ'));
        // U+1F96F, a bagel, as its two halves.
        let mut halves = [0u16; 2];
        '\u{1F96F}'.encode_utf16(&mut halves);
        assert_eq!(text.push(halves[0]), None);
        assert_eq!(text.push(halves[1]), Some('\u{1F96F}'));
    }

    #[test]
    fn half_a_character_alone_is_dropped() {
        let mut text = Utf16::default();
        // A second half with no first.
        assert_eq!(text.push(0xDD6F), None);
        // A first half followed by something whole: the whole one counts.
        assert_eq!(text.push(0xD83E), None);
        assert_eq!(text.push(u16::from(b'x')), Some('x'));
        assert_eq!(text.push(0xDD6F), None);
    }

    #[test]
    fn keys_that_arrive_as_characters_are_not_written() {
        assert!(is_text('a'));
        assert!(is_text('ş'));
        assert!(is_text(' '));
        assert!(!is_text('\r'));
        assert!(!is_text('\u{8}'));
        assert!(!is_text('\u{1b}'));
        // Ctrl+C.
        assert!(!is_text('\u{3}'));
    }

    #[test]
    fn egui_s_pointers_have_windows_own() {
        assert_eq!(pointer_of(egui::CursorIcon::Default), Some(32512));
        assert_eq!(pointer_of(egui::CursorIcon::Text), Some(32513));
        assert_eq!(pointer_of(egui::CursorIcon::PointingHand), Some(32649));
        assert_eq!(pointer_of(egui::CursorIcon::ResizeHorizontal), Some(32644));
        // The four ways a side or a corner is sized by.
        assert_eq!(pointer_of(egui::CursorIcon::ResizeVertical), Some(32645));
        assert_eq!(pointer_of(egui::CursorIcon::ResizeNwSe), Some(32642));
        assert_eq!(pointer_of(egui::CursorIcon::ResizeNeSw), Some(32643));
        assert_eq!(pointer_of(egui::CursorIcon::Move), Some(32646));
        assert_eq!(pointer_of(egui::CursorIcon::None), None);
    }
}
