//! What a drag does when it is let go, as the keys held say it.
//!
//! A Wayland program that drags names what it allows (a copy, a move),
//! the one under the pointer what it would take and likes best, and the
//! compositor chooses between them. Which of them the person wants is
//! said with the keyboard, as on every desktop: Ctrl for a copy, Shift
//! for a move. Neither program is told of keys while a drag has the
//! pointer, so the compositor is where they count (as in Mutter): the
//! action the keys ask for is taken where both sides allow it, and where
//! they do not, or no key is held, the choice is as it was.
//!
//! The library's drag asks for the choice only when the program under
//! the pointer names what it takes. A key pressed while the pointer
//! rests would so change nothing until that program spoke again; it is
//! made to: the drag leaves the surface it is over and enters it anew,
//! which is what the protocol has for "ask again"
//! ([`State::drag_keys_changed`]).
//!
//! An X11 program's drag (`xdnd.rs`) brings the action its own toolkit
//! made of the keys; that one is the wish then.

use crate::state::State;
use crate::surface::WindowSurface;
use smithay::input::pointer::MotionEvent;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::utils::SERIAL_COUNTER;

/// The keys that say what a drag is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Keys {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What the keys held ask of a drag: Ctrl a copy, Shift a move; Ctrl
/// with Shift, or Alt, that the person be asked (where other desktops
/// make a link, which a Wayland drag does not know). No key: nothing is
/// asked, and the programs' own wishes decide.
pub fn asked_by_keys(keys: Keys) -> Option<DndAction> {
    match (keys.ctrl, keys.shift, keys.alt) {
        (false, false, false) => None,
        (true, false, false) => Some(DndAction::Copy),
        (false, true, false) => Some(DndAction::Move),
        _ => Some(DndAction::Ask),
    }
}

/// The action of a drag: what is asked for with the keys where both
/// programs allow it (`available`), else what the program under the
/// pointer likes best, else the first there is (asking, a copy, a move).
/// Nothing where nothing is allowed.
pub fn choose(available: DndAction, preferred: DndAction, asked: Option<DndAction>) -> DndAction {
    match asked {
        Some(asked) if !asked.is_empty() && available.contains(asked) => asked,
        _ => smithay::wayland::selection::data_device::default_action_chooser(available, preferred),
    }
}

impl State {
    /// The keys held now, as far as a drag reads them.
    pub(crate) fn drag_keys(&self) -> Keys {
        self.seat
            .get_keyboard()
            .map(|keyboard| keyboard.modifier_state())
            .map_or(Keys::default(), |held| Keys {
                ctrl: held.ctrl,
                shift: held.shift,
                alt: held.alt,
            })
    }

    /// What is asked of the drag that is on: by the X11 program whose
    /// drag it is, else by the keys.
    pub(crate) fn drag_wish(&self) -> Option<DndAction> {
        if self.x_drag_is_own() {
            self.x_drag_wish()
        } else {
            asked_by_keys(self.drag_keys())
        }
    }

    /// A key went down or up. While a Wayland program's drag is on and
    /// the keys now ask for another action than before, the program
    /// under the pointer is made to say again what it takes: the drag
    /// leaves its surface and enters it anew, and the choice is made
    /// with the keys as they are.
    pub(crate) fn drag_keys_changed(&mut self) {
        if !self.dnd_on {
            self.drag_asked = None;
            return;
        }
        let asked = asked_by_keys(self.drag_keys());
        if asked == self.drag_asked {
            return;
        }
        self.drag_asked = asked;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if !pointer.is_grabbed() {
            return;
        }
        let location = pointer.current_location();
        let time = self.start_time.elapsed().as_millis() as u32;
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
        self.pointer_moved(location, time);
    }

    /// Where the picture of what is dragged is drawn, by its corner, in
    /// the desktop's own measure: at the pointer, moved by as much as its
    /// program moved it (a picture held by its middle has its middle
    /// under the pointer). `None` while nothing with a picture is dragged.
    pub fn drag_picture_place(&self) -> Option<(i32, i32)> {
        self.drag_icon.as_ref()?;
        Some((
            self.desktop.pointer.0 + self.drag_icon_offset.0,
            self.desktop.pointer.1 + self.drag_icon_offset.1,
        ))
    }

    /// The X11 window that is topmost under the pointer, if what is
    /// topmost there is an X11 window: not one that lies under a surface
    /// of a shell (a panel, a launcher, zones raised over the windows)
    /// or under another program's menu. By what the pointer itself would
    /// find (`surface_under`), so that a drag is told to the same thing
    /// the pointer is over.
    pub(crate) fn x11_window_under_pointer(&self) -> Option<u32> {
        let (window, under) = self.surface_under(self.pointer_exact.into());
        match window {
            Some(id) => match &self.desktop.windows.get(&id)?.surface {
                WindowSurface::X11(window) => Some(window.window_id()),
                WindowSurface::Xdg(_) => None,
            },
            // No window of the desktop: an X11 window that places itself
            // (a menu, a list that dropped down) is X11's all the same.
            None => {
                let (surface, _) = under?;
                self.x11_overrides
                    .iter()
                    .find(|window| window.wl_surface().as_ref() == Some(&surface))
                    .map(|window| window.window_id())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(ctrl: bool, shift: bool, alt: bool) -> Keys {
        Keys { ctrl, shift, alt }
    }

    #[test]
    fn the_keys_ask_for_a_copy_a_move_or_to_be_asked() {
        let table = [
            (keys(false, false, false), None),
            (keys(true, false, false), Some(DndAction::Copy)),
            (keys(false, true, false), Some(DndAction::Move)),
            (keys(true, true, false), Some(DndAction::Ask)),
            (keys(false, false, true), Some(DndAction::Ask)),
            (keys(true, false, true), Some(DndAction::Ask)),
            (keys(false, true, true), Some(DndAction::Ask)),
            (keys(true, true, true), Some(DndAction::Ask)),
        ];
        for (held, asked) in table {
            assert_eq!(asked_by_keys(held), asked, "{held:?}");
        }
    }

    #[test]
    fn what_the_keys_ask_for_is_taken_where_both_sides_allow_it() {
        let both = DndAction::Copy | DndAction::Move;
        // The target likes a move; Ctrl makes it a copy, Shift leaves it.
        assert_eq!(
            choose(both, DndAction::Move, Some(DndAction::Copy)),
            DndAction::Copy
        );
        assert_eq!(
            choose(both, DndAction::Move, Some(DndAction::Move)),
            DndAction::Move
        );
        assert_eq!(
            choose(both, DndAction::Copy, Some(DndAction::Move)),
            DndAction::Move
        );
        // No key: the target's wish.
        assert_eq!(choose(both, DndAction::Move, None), DndAction::Move);
        assert_eq!(choose(both, DndAction::Copy, None), DndAction::Copy);
    }

    #[test]
    fn a_key_for_what_is_not_allowed_changes_nothing() {
        // Only a copy can be had: Shift does not make it a move.
        assert_eq!(
            choose(DndAction::Copy, DndAction::Copy, Some(DndAction::Move)),
            DndAction::Copy
        );
        // Nobody asks the person here: Alt leaves the target's wish.
        let both = DndAction::Copy | DndAction::Move;
        assert_eq!(
            choose(both, DndAction::Move, Some(DndAction::Ask)),
            DndAction::Move
        );
        // Asking is allowed: Alt gets it.
        assert_eq!(
            choose(both | DndAction::Ask, DndAction::Move, Some(DndAction::Ask)),
            DndAction::Ask
        );
        // Nothing allowed: nothing, whatever is held.
        assert_eq!(
            choose(DndAction::empty(), DndAction::Copy, Some(DndAction::Copy)),
            DndAction::empty()
        );
        assert_eq!(
            choose(DndAction::empty(), DndAction::None, None),
            DndAction::empty()
        );
    }

    #[test]
    fn without_a_wish_of_the_target_the_first_action_there_is_is_taken() {
        let both = DndAction::Copy | DndAction::Move;
        assert_eq!(choose(both, DndAction::None, None), DndAction::Copy);
        assert_eq!(
            choose(DndAction::Move, DndAction::None, None),
            DndAction::Move
        );
        // What the target likes is not on offer: what is, is taken.
        assert_eq!(
            choose(DndAction::Copy, DndAction::Move, None),
            DndAction::Copy
        );
        // One precise action comes out of every choice.
        let single = [
            DndAction::empty(),
            DndAction::Copy,
            DndAction::Move,
            DndAction::Ask,
        ];
        for bits in 0..8u32 {
            let available = DndAction::from_bits_truncate(bits);
            for preferred in single {
                for asked in [
                    None,
                    Some(DndAction::Copy),
                    Some(DndAction::Move),
                    Some(DndAction::Ask),
                ] {
                    let chosen = choose(available, preferred, asked);
                    assert!(
                        single.contains(&chosen),
                        "{available:?} {preferred:?} {asked:?}"
                    );
                    assert!(available.contains(chosen));
                }
            }
        }
    }
}
