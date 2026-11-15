//! A window picked by a press on it (`DispatchCommand::PickWindow`).
//!
//! What sets a rule up for "windows like that one" asks for it: the
//! pointer becomes a cross, and the next press of a mouse button is not
//! the program's under it but names that program's window to whoever
//! listens (`window.picked`, with its class and its title). Escape, or a
//! press where no window is, ends it with nothing picked.

use crate::state::State;
use gevrek_engine::executor::RuntimeEventKind;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};

impl State {
    /// Begin picking; asked for again while it is on, it ends with
    /// nothing picked.
    pub fn toggle_pick(&mut self) {
        if self.picking.is_some() {
            self.end_pick(None);
            return;
        }
        // (What the pointer looked like is put back afterwards.)
        self.picking = Some(std::mem::replace(
            &mut self.cursor_status,
            CursorImageStatus::Named(CursorIcon::Crosshair),
        ));
        self.desktop.needs_redraw = true;
    }

    /// A mouse button went down while a window is being picked: the
    /// window under the pointer is the one (none: nothing is picked).
    /// Neither the press nor its release is anybody else's.
    pub(crate) fn pick_press(&mut self, button: u32) {
        let (x, y) = self.desktop.pointer;
        self.pick_button = Some(button);
        let window = self.desktop.window_at(x, y);
        self.end_pick(window);
    }

    /// The picking is over: with this window, or with none.
    pub(crate) fn end_pick(&mut self, window: Option<gevrek_engine::WindowHandle>) {
        let Some(cursor) = self.picking.take() else {
            return;
        };
        self.cursor_status = cursor;
        self.desktop.needs_redraw = true;
        let fields = match window.and_then(|id| self.desktop.windows.get(&id)) {
            Some(known) => vec![
                ("class".to_string(), known.app_id.clone()),
                ("title".to_string(), known.title.clone()),
            ],
            None => vec![("cancelled".to_string(), "true".to_string())],
        };
        self.emit(RuntimeEventKind::WindowPicked, window, fields);
    }
}
