//! The view a surface's window holds: the container of what is drawn and
//! of the parts shown soft, and what AppKit asks of a surface beyond its
//! events.
//!
//! - Drags dropped on it and begun from it (answered in `drag`).
//! - Text typed through the system's input methods (AppKit's text input
//!   client): a key is handed to the input system, which says what it
//!   writes. That is how a dead key (Option-E, then E: é) and an input
//!   method for Chinese, Japanese or Korean compose their text: what is
//!   being composed is told to egui as it changes (`Ime::Preedit`), and
//!   the text when it is done (`Ime::Commit`). The list of an input
//!   method's choices opens by the text cursor egui says it has.

use super::drag;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSResponder, NSTextInputClient, NSView};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSObject, NSPoint, NSRange,
    NSRangePointer, NSRect, NSString,
};
use std::cell::{Cell, RefCell};

pub struct Ivars {
    /// The surface's number.
    id: u32,
    /// What is being composed (an input method's, or a dead key's).
    marked: RefCell<String>,
    /// What the input system made of the keys handed to it.
    typed: RefCell<Vec<egui::Event>>,
    /// Where the text cursor is, in the window's coordinates.
    caret: Cell<NSRect>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; the ivars are plain
    // values, dropped with the view.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GevrekSurfaceView"]
    #[ivars = Ivars]
    pub struct SurfaceView;

    impl SurfaceView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: &AnyObject) -> usize {
            drag::entered(self, self.ivars().id, info)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: &AnyObject) -> usize {
            drag::updated(self, self.ivars().id, info)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: Option<&AnyObject>) {
            drag::exited(self.ivars().id);
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare(&self, _info: &AnyObject) -> bool {
            true
        }

        #[unsafe(method(performDragOperation:))]
        fn perform(&self, info: &AnyObject) -> bool {
            drag::dropped(self, self.ivars().id, info)
        }

        // As the source of a drag out of the surface.

        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_mask(&self, _session: &AnyObject, _context: isize) -> usize {
            drag::source_mask(self.ivars().id)
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &AnyObject, _at: NSPoint, operation: usize) {
            drag::ended(self.ivars().id, operation);
        }
    }

    unsafe impl NSObjectProtocol for SurfaceView {}

    unsafe impl NSTextInputClient for SurfaceView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _replacement: NSRange) {
            let text = text_of(string);
            let composed = !self.ivars().marked.take().is_empty();
            let mut typed = self.ivars().typed.borrow_mut();
            if composed {
                typed.push(egui::Event::Ime(egui::ImeEvent::Commit(text)));
            } else if !text.is_empty() && !text.chars().all(char::is_control) {
                typed.push(egui::Event::Text(text));
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command(&self, _selector: Sel) {
            // (The keys that are commands, Return or an arrow, reach egui
            // as keys.)
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, _replacement: NSRange) {
            let text = text_of(string);
            let active = chars_of(&text, selected);
            *self.ivars().marked.borrow_mut() = text.clone();
            self.ivars()
                .typed
                .borrow_mut()
                .push(egui::Event::Ime(egui::ImeEvent::Preedit {
                    text,
                    active_range_chars: active,
                }));
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let text = self.ivars().marked.take();
            if !text.is_empty() {
                self.ivars()
                    .typed
                    .borrow_mut()
                    .push(egui::Event::Ime(egui::ImeEvent::Commit(text)));
            }
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as usize, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let marked = self.ivars().marked.borrow();
            if marked.is_empty() {
                NSRange::new(NSNotFound as usize, 0)
            } else {
                NSRange::new(0, marked.encode_utf16().count())
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked.borrow().is_empty()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(
            &self,
            _range: NSRange,
            _actual: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: NSRangePointer) -> NSRect {
            let caret = self.ivars().caret.get();
            self.window()
                .map_or(caret, |window| window.convertRectToScreen(caret))
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _point: NSPoint) -> usize {
            NSNotFound as usize
        }
    }
);

impl SurfaceView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect, id: u32) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars {
            id,
            marked: RefCell::new(String::new()),
            typed: RefCell::new(Vec::new()),
            caret: Cell::new(NSRect::ZERO),
        });
        // SAFETY: NSView's initializer.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Whether text is being composed (keys go to the input method then,
    /// not to egui).
    pub fn composing(&self) -> bool {
        !self.ivars().marked.borrow().is_empty()
    }

    /// Hand a key to the system's input system; what it writes, as egui
    /// takes it.
    pub fn typed(&self, event: &NSEvent) -> Vec<egui::Event> {
        self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        self.ivars().typed.take()
    }

    /// Where egui's text cursor is, in the surface's points (from its
    /// top): the input method's list opens by it.
    pub fn set_caret(&self, rect: egui::Rect) {
        let height = self.bounds().size.height;
        self.ivars().caret.set(NSRect::new(
            NSPoint::new(f64::from(rect.min.x), height - f64::from(rect.max.y)),
            objc2_foundation::NSSize::new(f64::from(rect.width()), f64::from(rect.height())),
        ));
    }
}

/// The text of what the input system gives: a string, or an attributed
/// one.
fn text_of(string: &AnyObject) -> String {
    // SAFETY: what is asked of each is its own (an attributed string has
    // its plain text as `string`).
    unsafe {
        if let Some(attributed) = string.downcast_ref::<NSAttributedString>() {
            attributed.string().to_string()
        } else if let Some(text) = string.downcast_ref::<NSString>() {
            text.to_string()
        } else {
            let text: Option<Retained<NSString>> = msg_send![string, description];
            text.map(|text| text.to_string()).unwrap_or_default()
        }
    }
}

/// A range of UTF-16 units of `text` as a range of its characters.
pub fn chars_of(text: &str, range: NSRange) -> Option<std::ops::Range<usize>> {
    if range.location == NSNotFound as usize {
        return None;
    }
    let mut units = 0;
    let mut start = None;
    let mut end = None;
    for (index, character) in text.chars().enumerate() {
        if units == range.location {
            start = Some(index);
        }
        if units == range.location + range.length {
            end = Some(index);
        }
        units += character.len_utf16();
    }
    let count = text.chars().count();
    if units == range.location {
        start = start.or(Some(count));
    }
    if units == range.location + range.length {
        end = end.or(Some(count));
    }
    Some(start?..end?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_of_units_is_a_range_of_characters() {
        assert_eq!(chars_of("にほん", NSRange::new(1, 2)), Some(1..3));
        assert_eq!(chars_of("abc", NSRange::new(3, 0)), Some(3..3));
        // (An emoji is two units.)
        assert_eq!(chars_of("😀a", NSRange::new(2, 1)), Some(1..2));
        assert_eq!(chars_of("abc", NSRange::new(NSNotFound as usize, 0)), None);
    }
}
