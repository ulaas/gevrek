//! Protocols programs expect of a desktop beyond windows and input:
//!
//! - **relative-pointer** and **pointer-constraints**: a 3D viewport, a
//!   game or a remote desktop reads the mouse as movement, and keeps the
//!   pointer (locked in place, or confined to its window) while it does.
//! - **data-control** (wlr and ext): clipboard managers read and set the
//!   clipboard without a window of their own.
//! - **xdg-foreign**: a dialog of one program (a portal's file chooser) is
//!   shown as belonging to a window of another.
//! - **keyboard-shortcuts-inhibit**: a program (a virtual machine, a remote
//!   desktop, a gevrek nested in this one) asks for every key, also those
//!   gevrek's bindings use. Super+Escape gives the keys back to gevrek and
//!   hands them over again.
//! - **virtual-keyboard**: programs that type for the user (an on-screen
//!   keyboard, `wtype`).
//! - **single-pixel-buffer**, **content-type**: small things toolkits use.

use crate::state::State;
use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
};
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler};
use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};

impl PointerConstraintsHandler for State {
    /// A program asks to lock or confine the pointer on its surface: it
    /// gets that at once when the pointer is on the surface, otherwise
    /// when it comes there (`activate_pointer_constraint`).
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
    }
}

impl State {
    /// The pointer came onto a surface: a constraint its program asked for
    /// there starts now.
    pub(crate) fn activate_pointer_constraint(&mut self, surface: &WlSurface) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        with_pointer_constraint(surface, &pointer, |constraint| {
            if let Some(constraint) = constraint {
                if !constraint.is_active() {
                    constraint.activate();
                }
            }
        });
    }

    /// What the surface the pointer is on asks of the pointer now:
    /// `Some(true)` it is locked in place, `Some(false)` it is confined to
    /// the surface, `None` it is free.
    pub(crate) fn pointer_constraint(&self) -> Option<bool> {
        use smithay::wayland::pointer_constraints::PointerConstraint;
        let pointer = self.seat.get_pointer()?;
        let surface = pointer.current_focus()?;
        with_pointer_constraint(&surface, &pointer, |constraint| {
            let constraint = constraint?;
            if !constraint.is_active() {
                return None;
            }
            Some(matches!(&*constraint, PointerConstraint::Locked(_)))
        })
    }

    /// Whether the window that has the keyboard asked for every key and is
    /// given them now.
    pub(crate) fn shortcuts_inhibited(&self) -> bool {
        self.focused_inhibitor()
            .is_some_and(|inhibitor| inhibitor.is_active())
            || self.x11_has_the_keys()
    }

    /// Whether the window that has the keyboard asked for every key at
    /// all (it may have them now or not).
    pub(crate) fn shortcuts_inhibit_requested(&self) -> bool {
        self.focused_inhibitor().is_some() || self.x11_has_the_keys()
    }

    /// An X11 program holds the keyboard (XWayland's keyboard grab: what
    /// a game, a virtual machine or a remote desktop does with
    /// `XGrabKeyboard`), and the grab is still there.
    fn x11_has_the_keys(&self) -> bool {
        use smithay::reexports::wayland_server::Resource;
        self.x11_key_grab
            .as_ref()
            .is_some_and(|grab| grab.is_alive())
            && self
                .seat
                .get_keyboard()
                .is_some_and(|keyboard| keyboard.is_grabbed())
    }

    fn focused_inhibitor(&self) -> Option<KeyboardShortcutsInhibitor> {
        use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
        use smithay::wayland::seat::WaylandFocus;
        let focus = self.seat.get_keyboard()?.current_focus()?;
        let surface = focus.wl_surface()?;
        self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface)
    }

    /// Super+Escape: take the keys back from a program that asked for all
    /// of them, or hand them over again. Returns whether there was such a
    /// program.
    pub(crate) fn toggle_shortcuts_inhibit(&mut self) -> bool {
        // An X11 program that holds the keyboard: it is taken from it
        // (the program asks again when it wants it back).
        if self.x11_has_the_keys() {
            self.x11_key_grab = None;
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.unset_grab(self);
            }
            log::info!("gevrek's bindings are back (an X11 program held the keyboard)");
            return true;
        }
        let Some(inhibitor) = self.focused_inhibitor() else {
            return false;
        };
        if inhibitor.is_active() {
            inhibitor.inactivate();
            log::info!("gevrek's bindings are back (a program had asked for every key)");
        } else {
            inhibitor.activate();
            log::info!(
                "Every key goes to the program that asked for them; Super+Escape takes them back"
            );
        }
        true
    }
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.shortcuts_inhibit_state
    }

    /// A program asks for every key while its window has the keyboard: it
    /// gets them (the user asked for that program), and Super+Escape is the
    /// way out.
    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibitor.activate();
    }
}

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}

impl smithay::wayland::selection::wlr_data_control::DataControlHandler for State {
    fn data_control_state(
        &self,
    ) -> &smithay::wayland::selection::wlr_data_control::DataControlState {
        &self.wlr_data_control_state
    }
}

impl smithay::wayland::selection::ext_data_control::DataControlHandler for State {
    fn data_control_state(
        &self,
    ) -> &smithay::wayland::selection::ext_data_control::DataControlState {
        &self.ext_data_control_state
    }
}

smithay::delegate_relative_pointer!(State);
smithay::delegate_pointer_constraints!(State);
smithay::delegate_keyboard_shortcuts_inhibit!(State);

// XWayland's keyboard grab: an X11 program that grabs the keyboard (a
// game, a virtual machine, a remote desktop) gets every key, gevrek's
// bindings among them, until it lets go or Super+Escape takes them back.
impl smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabHandler for State {
    fn grab(
        &mut self,
        _surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        seat: smithay::input::Seat<Self>,
        grab: smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrab<Self>,
    ) {
        // Not under the lock screen.
        if self.locked {
            return;
        }
        self.x11_key_grab = Some(grab.grab().clone());
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_grab(self, grab, smithay::utils::SERIAL_COUNTER.next_serial());
        }
        log::info!("An X11 program holds the keyboard; Super+Escape takes it back");
    }

    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<crate::surface::KeyTarget> {
        use smithay::wayland::seat::WaylandFocus;
        // The X11 window this surface is of, among the managed ones and
        // those that place themselves.
        self.desktop
            .windows
            .values()
            .filter_map(|window| window.surface.key_target())
            .chain(
                self.x11_overrides
                    .iter()
                    .cloned()
                    .map(crate::surface::KeyTarget::X11),
            )
            .find(|target| {
                matches!(target, crate::surface::KeyTarget::X11(_))
                    && target.wl_surface().as_deref() == Some(surface)
            })
    }
}

smithay::delegate_xwayland_keyboard_grab!(State);

// Frames paced by the display (`wp-fifo`, `wp-commit-timing`): a program
// may hold a picture back until the one before was shown, or until a
// time. They are let go when a frame was drawn (`send_frame_callbacks`).
smithay::delegate_fifo!(State);
smithay::delegate_commit_timing!(State);
smithay::delegate_xdg_foreign!(State);
smithay::delegate_data_control!(State);
smithay::delegate_ext_data_control!(State);
smithay::delegate_single_pixel_buffer!(State);
smithay::delegate_content_type!(State);
smithay::delegate_virtual_keyboard_manager!(State);
