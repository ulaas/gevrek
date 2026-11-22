//! Drag and drop on macOS: a surface's view (`view::SurfaceView`) is where
//! other programs' drags are dropped (AppKit's dragging destination) and
//! where drags out of it begin (its dragging source); what it is asked is
//! answered here.
//!
//! AppKit asks the view while the drag is under way and wants its answer
//! at once: the answer is the one the surface last gave (or its standing
//! one), as on Windows, and what happened is kept for the surface's next
//! frame ([`take_events`]). The pasteboard of a drag can be read while it
//! is still over the surface, so what is offered carries its payload.
//!
//! Keys as on a Mac: Option copies, Command moves, both make a link (an
//! alias).

use crate::dnd::{
    self, DragAction, DragActions, DragEvent, DragOutcome, DragPayload, DragStart, DropAnswer,
    Held, Offered, PathStyle,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSView};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// AppKit's names for the kinds of data a drag carries.
const FILE_URL: &str = "public.file-url";
const URL: &str = "public.url";
const TEXT: &str = "public.utf8-plain-text";

// NSDragOperation's bits.
const OPERATION_COPY: usize = 1;
const OPERATION_LINK: usize = 2;
const OPERATION_GENERIC: usize = 4;
const OPERATION_MOVE: usize = 16;
const OPERATION_DELETE: usize = 32;

/// What may be done, from what a source offers (Generic: whatever the
/// target likes, which for files is a move within a disk).
pub fn actions_of(mask: usize) -> DragActions {
    DragActions {
        copy: mask & OPERATION_COPY != 0,
        moving: mask & (OPERATION_MOVE | OPERATION_GENERIC) != 0,
        link: mask & OPERATION_LINK != 0,
    }
}

/// What AppKit is told a target would do, or a source allows.
pub fn mask_of(actions: DragActions) -> usize {
    let mut mask = 0;
    if actions.copy {
        mask |= OPERATION_COPY;
    }
    if actions.moving {
        mask |= OPERATION_MOVE | OPERATION_GENERIC;
    }
    if actions.link {
        mask |= OPERATION_LINK;
    }
    mask
}

pub fn mask_of_action(action: Option<DragAction>) -> usize {
    match action {
        Some(DragAction::Copy) => OPERATION_COPY,
        Some(DragAction::Move) => OPERATION_MOVE,
        Some(DragAction::Link) => OPERATION_LINK,
        None => 0,
    }
}

/// The keys held, as the shared rules name them: Option asks for a copy
/// (Ctrl elsewhere), Command for a move (Shift elsewhere), and both for a
/// link.
pub fn held_of(flags: NSEventModifierFlags) -> Held {
    Held {
        ctrl: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Command),
        alt: false,
    }
}

/// How a drag out ended, by the operation the target took.
pub fn outcome_of(operation: usize) -> DragOutcome {
    let taken = |action| DragOutcome::Taken {
        action,
        remove: false,
    };
    if operation & OPERATION_MOVE != 0 || operation & (OPERATION_GENERIC | OPERATION_DELETE) != 0 {
        // (The Trash, and Finder, move files themselves.)
        taken(DragAction::Move)
    } else if operation & OPERATION_COPY != 0 {
        taken(DragAction::Copy)
    } else if operation & OPERATION_LINK != 0 {
        taken(DragAction::Link)
    } else {
        DragOutcome::Cancelled
    }
}

/// What a surface has to do with drags, kept where AppKit's calls reach.
#[derive(Default)]
struct Slot {
    /// Drops are taken: what of any drag, until the surface says more.
    standing: Option<DropAnswer>,
    accepting: bool,
    /// What it would do with the drag over it now.
    answer: DropAnswer,
    /// What the drag over it offers.
    offered: DragActions,
    /// What the source of a drag out of it allows.
    out: DragActions,
    events: Vec<DragEvent>,
}

thread_local! {
    static SLOTS: RefCell<HashMap<u32, Slot>> = RefCell::new(HashMap::new());
    /// Surfaces with something new since they last drew.
    static TOUCHED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// A drag out of a surface is under way.
    static OUT: Cell<bool> = const { Cell::new(false) };
}

fn with_slot<T>(id: u32, work: impl FnOnce(&mut Slot) -> T) -> T {
    SLOTS.with(|slots| work(slots.borrow_mut().entry(id).or_default()))
}

fn tell(id: u32, event: DragEvent) {
    with_slot(id, |slot| slot.events.push(event));
    TOUCHED.with(|touched| touched.borrow_mut().push(id));
}

/// Take drops on a surface, or no longer. Whether its view is to be
/// registered for them (or let go).
pub fn accept(id: u32, standing: Option<DropAnswer>) {
    with_slot(id, |slot| {
        slot.accepting = standing.is_some();
        slot.standing = standing;
        slot.answer = standing.unwrap_or(DropAnswer::REFUSE);
    });
}

pub fn answer(id: u32, answer: DropAnswer) {
    with_slot(id, |slot| slot.answer = answer);
}

pub fn take_events(id: u32) -> Vec<DragEvent> {
    with_slot(id, |slot| std::mem::take(&mut slot.events))
}

/// Whether a drag out of a surface is under way.
pub fn dragging_out() -> bool {
    OUT.with(Cell::get)
}

/// The surfaces that have news of drags since this was last asked.
pub fn touched() -> Vec<u32> {
    TOUCHED.with(|touched| std::mem::take(&mut *touched.borrow_mut()))
}

/// A surface that is gone.
pub fn forget(id: u32) {
    SLOTS.with(|slots| slots.borrow_mut().remove(&id));
}

/// Register a view for the kinds of data that are taken, or let it go.
pub fn register(view: &NSView, on: bool) {
    if on {
        let kinds: Vec<Retained<NSString>> = [FILE_URL, URL, TEXT]
            .iter()
            .map(|kind| NSString::from_str(kind))
            .collect();
        let kinds = NSArray::from_retained_slice(&kinds);
        // SAFETY: an array of pasteboard types (strings).
        let _: () = unsafe { msg_send![view, registerForDraggedTypes: &*kinds] };
    } else {
        view.unregisterDraggedTypes();
    }
}

fn text(object: Option<Retained<NSString>>) -> Option<String> {
    object.map(|text| text.to_string())
}

/// What a pasteboard carries: files, addresses, text.
fn payload_of(pasteboard: &AnyObject, allowed: DragActions) -> DragPayload {
    let mut payload = DragPayload {
        allowed,
        ..Default::default()
    };
    let (file_url, url, plain) = (
        NSString::from_str(FILE_URL),
        NSString::from_str(URL),
        NSString::from_str(TEXT),
    );
    // SAFETY: the pasteboard's items and their strings, read as they are.
    unsafe {
        let items: Option<Retained<NSArray<AnyObject>>> = msg_send![pasteboard, pasteboardItems];
        for item in items.iter().flat_map(|items| items.iter()) {
            if let Some(address) = text(msg_send![&*item, stringForType: &*file_url]) {
                match dnd::uri::to_path(&address, PathStyle::HERE, None) {
                    Some(path) => payload.paths.push(path),
                    None => payload.uris.push(address),
                }
            } else if let Some(address) = text(msg_send![&*item, stringForType: &*url]) {
                payload.uris.push(address);
            }
        }
        // (Finder puts the files' names as text beside them: the files are
        // what is dragged.)
        if payload.paths.is_empty() && payload.uris.is_empty() {
            payload.text = text(msg_send![pasteboard, stringForType: &*plain]);
        }
    }
    payload
}

fn kinds_of(pasteboard: &AnyObject) -> Vec<String> {
    // SAFETY: the pasteboard's types, strings.
    let types: Option<Retained<NSArray<NSString>>> = unsafe { msg_send![pasteboard, types] };
    types
        .map(|types| types.iter().map(|kind| kind.to_string()).collect())
        .unwrap_or_default()
}

// What AppKit asks of a surface's view (`view::SurfaceView`) about drags.

/// A drag came over the surface: what it would come to.
pub fn entered(view: &NSView, id: u32, info: &AnyObject) -> usize {
    if !with_slot(id, |slot| slot.accepting) {
        return 0;
    }
    // SAFETY: the dragging info's own properties.
    let (mask, pasteboard): (usize, Retained<AnyObject>) = unsafe {
        (
            msg_send![info, draggingSourceOperationMask],
            msg_send![info, draggingPasteboard],
        )
    };
    let allowed = actions_of(mask);
    let kinds = kinds_of(&pasteboard);
    let offered = Offered {
        files: kinds.iter().any(|kind| kind == FILE_URL),
        kinds,
        allowed,
        payload: Some(payload_of(&pasteboard, allowed)),
    };
    with_slot(id, |slot| {
        slot.offered = allowed;
        slot.answer = slot.standing.unwrap_or(DropAnswer::REFUSE);
    });
    tell(
        id,
        DragEvent::Entered {
            at: at(view, info),
            offered,
        },
    );
    operation(id)
}

/// The drag over the surface moved (or rests): what it would come to.
pub fn updated(view: &NSView, id: u32, info: &AnyObject) -> usize {
    if !with_slot(id, |slot| slot.accepting) {
        return 0;
    }
    let at = at(view, info);
    // (AppKit asks again and again while the pointer rests: only a move
    // is news.)
    let moved = with_slot(
        id,
        |slot| !matches!(slot.events.last(), Some(DragEvent::Moved { at: last }) if *last == at),
    );
    if moved {
        tell(id, DragEvent::Moved { at });
    }
    operation(id)
}

/// The drag went away.
pub fn exited(id: u32) {
    if with_slot(id, |slot| slot.accepting) {
        tell(id, DragEvent::Left);
    }
}

/// Something was let go on the surface: whether it is taken, and what it
/// is told of.
pub fn dropped(view: &NSView, id: u32, info: &AnyObject) -> bool {
    let (accepting, offered, answer) =
        with_slot(id, |slot| (slot.accepting, slot.offered, slot.answer));
    let Some(action) = accepting
        .then(|| dnd::negotiate(offered, held_of(NSEvent::modifierFlags_class()), answer))
        .flatten()
    else {
        return false;
    };
    // SAFETY: the dragging info's pasteboard.
    let pasteboard: Retained<AnyObject> = unsafe { msg_send![info, draggingPasteboard] };
    let payload = payload_of(&pasteboard, offered);
    tell(
        id,
        DragEvent::Dropped {
            at: at(view, info),
            payload,
            action,
        },
    );
    true
}

/// What the source of a drag out of the surface allows.
pub fn source_mask(id: u32) -> usize {
    mask_of(with_slot(id, |slot| slot.out))
}

/// The drag out of the surface is over.
pub fn ended(id: u32, operation: usize) {
    log::debug!("surface {id}: the drag out ended with operation {operation}");
    OUT.with(|out| out.set(false));
    tell(
        id,
        DragEvent::Ended {
            outcome: outcome_of(operation),
        },
    );
}

/// Where the drag is, in the surface's points (from its top).
fn at(view: &NSView, info: &AnyObject) -> egui::Pos2 {
    // SAFETY: the dragging info's place, in the window's coordinates.
    let point: NSPoint = unsafe { msg_send![info, draggingLocation] };
    let height = view.bounds().size.height;
    egui::pos2(point.x as f32, (height - point.y) as f32)
}

/// What the drag over the surface would come to now.
fn operation(id: u32) -> usize {
    let (offered, answer) = with_slot(id, |slot| (slot.offered, slot.answer));
    mask_of_action(dnd::negotiate(
        offered,
        held_of(NSEvent::modifierFlags_class()),
        answer,
    ))
}

/// Begin a drag of `drag` out of the surface, by the mouse event that
/// moved with the button held; the view is its source. Whether it began.
pub fn begin(view: &NSView, id: u32, drag: &DragStart, event: &NSEvent) -> bool {
    let items = items(drag, event);
    if items.is_empty() {
        return false;
    }
    with_slot(id, |slot| slot.out = drag.payload.allowed);
    let items = NSArray::from_retained_slice(&items);
    // SAFETY: dragging items, the event of the drag, and this view as
    // its source (it answers the source's two questions).
    let session: Option<Retained<AnyObject>> = unsafe {
        msg_send![view, beginDraggingSessionWithItems: &*items, event: event, source: view]
    };
    OUT.with(|out| out.set(session.is_some()));
    session.is_some()
}

/// One item for each file, address and text, the picture (if any) under
/// the pointer on the first.
fn items(drag: &DragStart, event: &NSEvent) -> Vec<Retained<AnyObject>> {
    let point = event.locationInWindow();
    let picture = drag
        .image
        .as_ref()
        .filter(|image| image.is_sound())
        .and_then(image_of);
    let (width, tall, hotspot) = match &drag.image {
        Some(image) if picture.is_some() => {
            let (width, tall) = image.logical_size();
            (f64::from(width), f64::from(tall), image.logical_hotspot())
        }
        _ => (32.0, 32.0, (16, 16)),
    };
    // (The view is measured from its bottom; the hotspot from the
    // picture's top.)
    let frame = NSRect::new(
        NSPoint::new(
            point.x - f64::from(hotspot.0),
            point.y - (tall - f64::from(hotspot.1)),
        ),
        NSSize::new(width, tall),
    );
    let mut writers: Vec<Retained<AnyObject>> = Vec::new();
    // SAFETY: NSURL and NSString are pasteboard writers.
    unsafe {
        for path in &drag.payload.paths {
            let path = NSString::from_str(path);
            let url: Option<Retained<AnyObject>> =
                msg_send![class!(NSURL), fileURLWithPath: &*path];
            writers.extend(url);
        }
        for address in &drag.payload.uris {
            let address = NSString::from_str(address);
            let url: Option<Retained<AnyObject>> =
                msg_send![class!(NSURL), URLWithString: &*address];
            writers.extend(url);
        }
        if let Some(text) = &drag.payload.text {
            writers.push(Retained::cast_unchecked(NSString::from_str(text)));
        }
    }
    writers
            .into_iter()
            .enumerate()
            .filter_map(|(index, writer)| {
                // SAFETY: an item of a pasteboard writer, with its frame and
                // (on the first) its picture.
                unsafe {
                    let item: Option<Retained<AnyObject>> =
                        msg_send![msg_send![class!(NSDraggingItem), alloc], initWithPasteboardWriter: &*writer];
                    let item = item?;
                    let contents: Option<&AnyObject> = if index == 0 { picture.as_deref() } else { None };
                    let _: () = msg_send![&*item, setDraggingFrame: frame, contents: contents];
                    Some(item)
                }
            })
            .collect()
}

/// A picture AppKit shows under the pointer: an NSImage of the pixels, at
/// their logical size.
fn image_of(image: &dnd::DragImage) -> Option<Retained<AnyObject>> {
    let (width, height) = (image.width as isize, image.height as isize);
    let space = NSString::from_str("NSDeviceRGBColorSpace");
    // SAFETY: a bitmap that owns its bytes, filled with as many as its
    // size says; an image of it.
    unsafe {
        let planes: *mut *mut u8 = std::ptr::null_mut();
        let bitmap: Option<Retained<AnyObject>> = msg_send![
            msg_send![class!(NSBitmapImageRep), alloc],
            initWithBitmapDataPlanes: planes,
            pixelsWide: width,
            pixelsHigh: height,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: &*space,
            bytesPerRow: width * 4,
            bitsPerPixel: 32isize
        ];
        let bitmap = bitmap?;
        let data: *mut u8 = msg_send![&*bitmap, bitmapData];
        if data.is_null() {
            return None;
        }
        // AppKit's bitmaps are red first with colours multiplied by how
        // solid they are.
        let bytes = std::slice::from_raw_parts_mut(data, image.rgba.len());
        for (pixel, out) in image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(bytes.as_chunks_mut::<4>().0.iter_mut())
        {
            let alpha = u32::from(pixel[3]);
            let scaled = |colour: u8| ((u32::from(colour) * alpha + 127) / 255) as u8;
            out.copy_from_slice(&[
                scaled(pixel[0]),
                scaled(pixel[1]),
                scaled(pixel[2]),
                pixel[3],
            ]);
        }
        let (logical_width, logical_height) = image.logical_size();
        let size = NSSize::new(f64::from(logical_width), f64::from(logical_height));
        let picture: Option<Retained<AnyObject>> =
            msg_send![msg_send![class!(NSImage), alloc], initWithSize: size];
        let picture = picture?;
        let _: () = msg_send![&*picture, addRepresentation: &*bitmap];
        Some(picture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_source_offers_is_read_with_generic_as_a_move() {
        assert_eq!(actions_of(OPERATION_COPY), DragActions::COPY);
        assert_eq!(
            actions_of(OPERATION_GENERIC | OPERATION_COPY),
            DragActions::COPY_OR_MOVE
        );
        assert_eq!(actions_of(usize::MAX), DragActions::ALL);
        assert_eq!(actions_of(0), DragActions::NONE);
        assert_eq!(actions_of(mask_of(DragActions::ALL)), DragActions::ALL);
    }

    #[test]
    fn option_copies_command_moves_and_both_link() {
        let held = |flags| held_of(flags).wanted();
        assert_eq!(held(NSEventModifierFlags::Option), Some(DragAction::Copy));
        assert_eq!(held(NSEventModifierFlags::Command), Some(DragAction::Move));
        assert_eq!(
            held(NSEventModifierFlags::Option | NSEventModifierFlags::Command),
            Some(DragAction::Link)
        );
        assert_eq!(held(NSEventModifierFlags::Shift), None);
    }

    #[test]
    fn a_drag_out_ends_as_the_target_took_it() {
        assert_eq!(outcome_of(0), DragOutcome::Cancelled);
        assert_eq!(
            outcome_of(OPERATION_COPY),
            DragOutcome::Taken {
                action: DragAction::Copy,
                remove: false
            }
        );
        assert_eq!(
            outcome_of(OPERATION_MOVE),
            DragOutcome::Taken {
                action: DragAction::Move,
                remove: false
            }
        );
        assert_eq!(
            outcome_of(OPERATION_DELETE),
            DragOutcome::Taken {
                action: DragAction::Move,
                remove: false
            }
        );
        assert_eq!(
            outcome_of(OPERATION_LINK),
            DragOutcome::Taken {
                action: DragAction::Link,
                remove: false
            }
        );
    }

    #[test]
    fn the_answer_given_is_what_appkit_is_told() {
        assert_eq!(mask_of_action(Some(DragAction::Move)), OPERATION_MOVE);
        assert_eq!(mask_of_action(None), 0);
    }
}
