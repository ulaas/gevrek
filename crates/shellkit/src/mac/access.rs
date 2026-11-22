//! A surface as screen readers and other aids know it (VoiceOver, through
//! AccessKit): as on Windows and Linux, nothing is described until
//! somebody listens, and what they ask to be done is input for egui.
//!
//! AccessKit takes the surface's view over (a subclass made at run time,
//! which answers AppKit's accessibility questions); it does so before the
//! window is first shown.

use egui::accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use objc2_app_kit::NSView;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct Shared {
    /// Somebody listens: the surface is to be described.
    wanted: Cell<bool>,
    /// What was asked to be done since the last pass.
    asked: RefCell<Vec<ActionRequest>>,
}

/// AppKit's questions, on the main thread (within the loop's wait, which
/// returns once they are answered).
struct Hears(Rc<Shared>);

impl ActivationHandler for Hears {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // (The tree comes with the next frame.)
        self.0.wanted.set(true);
        None
    }
}

impl ActionHandler for Hears {
    fn do_action(&mut self, request: ActionRequest) {
        self.0.asked.borrow_mut().push(request);
    }
}

pub struct Access {
    adapter: accesskit_macos::SubclassingAdapter,
    shared: Rc<Shared>,
    /// egui was told to describe the surface.
    described: bool,
    /// The surface had the keyboard when it last drew.
    focused: bool,
}

impl Access {
    /// For a view whose window was not shown yet.
    pub fn new(view: &NSView) -> Self {
        let shared = Rc::new(Shared::default());
        // SAFETY: a live view, kept by its window (and retained by the
        // adapter), taken over before it is shown.
        let adapter = unsafe {
            accesskit_macos::SubclassingAdapter::new(
                view as *const NSView as *mut std::ffi::c_void,
                Hears(Rc::clone(&shared)),
                Hears(Rc::clone(&shared)),
            )
        };
        Self {
            adapter,
            shared,
            described: false,
            focused: false,
        }
    }

    /// What screen readers asked since the last pass: egui describes the
    /// surface from its next frame on once somebody listens, and what
    /// was asked to be done is input. Whether there is anything new.
    pub fn news(&mut self, context: &egui::Context, events: &mut Vec<egui::Event>) -> bool {
        let mut new = false;
        if self.shared.wanted.get() && !self.described {
            context.enable_accesskit();
            self.described = true;
            new = true;
        }
        for request in self.shared.asked.take() {
            events.push(egui::Event::AccessKitActionRequest(request));
            new = true;
        }
        new
    }

    /// What a frame shows, and whether the surface has the keyboard.
    pub fn update(&mut self, update: Option<TreeUpdate>, focused: bool) {
        if focused != self.focused {
            self.focused = focused;
            if let Some(events) = self.adapter.update_view_focus_state(focused) {
                events.raise();
            }
        }
        if let Some(update) = update {
            if let Some(events) = self.adapter.update_if_active(|| update) {
                events.raise();
            }
        }
    }
}
