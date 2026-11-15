//! Input methods: programs that turn what is typed into other text
//! (Chinese, Japanese, Korean and more, the emoji and compose helpers of
//! fcitx5 and IBus).
//!
//! Two protocols meet here. A program with a text field says so through
//! **text-input** (v3): where its cursor is, what stands around it. The
//! input method speaks **input-method** (v2): it is told when a field has
//! the keyboard, may take the keys for itself (its keyboard grab), and
//! hands back the text to put in, or a text that is still being made
//! (the pre-edit). smithay carries the one to the other with the
//! keyboard's focus; what is gevrek's own is the input method's little
//! window (the list of candidates), which is drawn at the text cursor of
//! the window that is typed into (`render.rs`).
//!
//! The input method is an ordinary program started with the session
//! (`desktop.startup`), and types through the virtual keyboard where it
//! must (`more_protocols.rs`).

use crate::state::State;
use smithay::desktop::{PopupKind, PopupManager};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface};

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(error) = self.popups.track_popup(PopupKind::from(surface)) {
            log::warn!("an input method's window is not shown: {error}");
        }
        self.desktop.needs_redraw = true;
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
        self.desktop.needs_redraw = true;
    }

    fn popup_repositioned(&mut self, _: PopupSurface) {
        self.desktop.needs_redraw = true;
    }

    /// (The text cursor's rectangle is told from the corner of the
    /// window's surface, and so is the place the window is drawn from:
    /// nothing is to be added.)
    fn parent_geometry(&self, _: &WlSurface) -> Rectangle<i32, Logical> {
        Rectangle::default()
    }
}

/// Where an input method's window goes, from the corner of the surface
/// that is typed into: under the text cursor, at its left.
pub fn under_the_cursor(cursor: Rectangle<i32, Logical>) -> (i32, i32) {
    (cursor.loc.x, cursor.loc.y + cursor.size.h)
}

smithay::delegate_text_input_manager!(State);

// What `smithay::delegate_input_method_manager!` hands to the library,
// but for the manager's own requests, which gevrek looks at first: a
// second input method that asks while one runs is told that it is not
// taken (`unavailable`, as the protocol has it) and the first goes on.
// (The library, 0.7, tells the one that was there instead, and serves
// neither.)
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::{
    zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
    zwp_input_method_manager_v2::{self, ZwpInputMethodManagerV2},
    zwp_input_method_v2::ZwpInputMethodV2,
    zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
};
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle};
use smithay::wayland::input_method::InputMethodManagerState;

smithay::reexports::wayland_server::delegate_global_dispatch!(State: [
    ZwpInputMethodManagerV2: smithay::wayland::input_method::InputMethodManagerGlobalData
] => InputMethodManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    ZwpInputMethodV2: smithay::wayland::input_method::InputMethodUserData<State>
] => InputMethodManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    ZwpInputMethodKeyboardGrabV2: smithay::wayland::input_method::InputMethodKeyboardUserData<State>
] => InputMethodManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    ZwpInputPopupSurfaceV2: smithay::wayland::input_method::InputMethodPopupSurfaceUserData
] => InputMethodManagerState);

/// An input method that was not taken: it is told so, and whatever it
/// asks afterwards is nothing.
pub struct NotTaken;

impl Dispatch<ZwpInputMethodV2, NotTaken> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputMethodV2,
        _: <ZwpInputMethodV2 as smithay::reexports::wayland_server::Resource>::Request,
        _: &NotTaken,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        manager: &ZwpInputMethodManagerV2,
        request: zwp_input_method_manager_v2::Request,
        data: &(),
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let request = match request {
            zwp_input_method_manager_v2::Request::GetInputMethod { seat, input_method } => {
                // Another program is the input method, and still there.
                let taken = state.input_method_of.as_ref().is_some_and(|other| {
                    *other != client.id()
                        && display
                            .backend_handle()
                            .get_client_data(other.clone())
                            .is_ok()
                });
                if taken {
                    log::info!(
                        "A second input method asks while one runs: it is told it is not taken"
                    );
                    init.init(input_method, NotTaken).unavailable();
                    return;
                }
                state.input_method_of = Some(client.id());
                zwp_input_method_manager_v2::Request::GetInputMethod { seat, input_method }
            }
            other => other,
        };
        <InputMethodManagerState as Dispatch<ZwpInputMethodManagerV2, (), Self>>::request(
            state, client, manager, request, data, display, init,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_candidates_stand_under_the_text_cursor() {
        let cursor = Rectangle::new((120, 40).into(), (2, 18).into());
        assert_eq!(under_the_cursor(cursor), (120, 58));
    }
}
