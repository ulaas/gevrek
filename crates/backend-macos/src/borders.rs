//! Borders around the windows: windows of gevrek's own.
//!
//! macOS does not let a program draw on another's window, so a border is
//! a window of its own: borderless, transparent, never in the way of the
//! pointer, a little bigger than the window it goes around, with a ring
//! drawn by its layer. It is put directly above its window in the order of
//! windows (`orderWindow:relativeTo:` with the window server's id of the
//! other program's window), so a window that lies over that one lies over
//! its border too; and placed again whenever its window moves.
//!
//! A border follows its window as the window's moves are heard of (the
//! Accessibility API tells them as they happen): while a window is
//! dragged, the border trails it a little.
//!
//! Only the plain ring is drawn so far, square or rounded: the glow, the
//! shadow, the dashed, double and gradient styles are drawn as it.

use crate::space;
use gevrek_engine::workspace::BorderLook;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::graph::BorderStyle;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSScreen, NSWindow, NSWindowCollectionBehavior,
    NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_core_graphics::CGColor;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::collections::HashMap;

/// The corner of a macOS window, in points, which a rounded border
/// follows. (Windows of macOS 26 and later have rounder corners than
/// before; a toolbar makes them rounder still. This is a middle.)
const WINDOW_CORNER: f64 = 12.0;

/// One border.
struct Border {
    window: Retained<NSWindow>,
    look: BorderLook,
    focused: bool,
    /// Shown, as far as its window's state goes (not on a hidden
    /// workspace, not minimized).
    visible: bool,
    /// Where its window is.
    around: Rect,
}

/// The borders, by the window each goes around.
pub struct Borders {
    mtm: MainThreadMarker,
    borders: HashMap<WindowHandle, Border>,
    /// All of them shown, or all hidden (the overview of a workspace).
    shown: bool,
}

impl Borders {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            borders: HashMap::new(),
            shown: true,
        }
    }

    pub fn has(&self, id: WindowHandle) -> bool {
        self.borders.contains_key(&id)
    }

    /// Draw the border of `id` around `around`, as `look` has it: made
    /// where there is none, changed where there is one.
    pub fn draw(&mut self, id: WindowHandle, around: Rect, look: &BorderLook, focused: bool) {
        if !look.enabled {
            self.remove(id);
            return;
        }
        if !self.borders.contains_key(&id) {
            let window = make_window(self.mtm);
            self.borders.insert(
                id,
                Border {
                    window,
                    look: *look,
                    focused,
                    visible: true,
                    around,
                },
            );
        }
        let border = self.borders.get_mut(&id).expect("just made");
        border.look = *look;
        border.focused = focused;
        border.around = around;
        paint(border);
        place(self.mtm, border);
        self.order(id);
    }

    /// The window moved: its border goes with it.
    pub fn follow(&mut self, id: WindowHandle, around: Rect) {
        let mtm = self.mtm;
        let Some(border) = self.borders.get_mut(&id) else {
            return;
        };
        if border.around == around {
            return;
        }
        border.around = around;
        place(mtm, border);
        self.order(id);
    }

    pub fn set_focused(&mut self, id: WindowHandle, focused: bool) {
        let Some(border) = self.borders.get_mut(&id) else {
            return;
        };
        if border.focused != focused {
            border.focused = focused;
            paint(border);
        }
        // The focused window came to the front: its border with it.
        if focused {
            self.order(id);
        }
    }

    pub fn set_visible(&mut self, id: WindowHandle, visible: bool) {
        let Some(border) = self.borders.get_mut(&id) else {
            return;
        };
        border.visible = visible;
        self.order(id);
    }

    /// Show every border, or hide them all.
    pub fn set_all_shown(&mut self, shown: bool) {
        self.shown = shown;
        let ids: Vec<WindowHandle> = self.borders.keys().copied().collect();
        for id in ids {
            self.order(id);
        }
    }

    pub fn remove(&mut self, id: WindowHandle) {
        if let Some(border) = self.borders.remove(&id) {
            border.window.orderOut(None);
            border.window.close();
        }
    }

    /// Put the border above its window again (the window came to the front
    /// after the border was ordered).
    pub fn raise(&self, id: WindowHandle) {
        self.order(id);
    }

    /// Put the border directly above its window, or away where it is not
    /// to be seen.
    fn order(&self, id: WindowHandle) {
        let Some(border) = self.borders.get(&id) else {
            return;
        };
        if self.shown && border.visible {
            border
                .window
                .orderWindow_relativeTo(NSWindowOrderingMode::Above, id.0);
        } else {
            border.window.orderOut(None);
        }
    }
}

impl Drop for Borders {
    fn drop(&mut self) {
        let ids: Vec<WindowHandle> = self.borders.keys().copied().collect();
        for id in ids {
            self.remove(id);
        }
    }
}

/// A window to draw a border in: no frame, no shadow, clear, never taking
/// the pointer or the keyboard, at the level of ordinary windows (so that
/// it can stand between them), on no Space of its own.
fn make_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(10.0, 10.0));
    // SAFETY: a plain borderless window, made on the main thread; it is
    // kept by `Border` (not released when closed).
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            rect,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(0);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    if let Some(view) = window.contentView() {
        view.setWantsLayer(true);
    }
    window
}

/// How far the border reaches beyond its window on each side.
fn reach(look: &BorderLook) -> i32 {
    look.offset.max(0) + look.width.max(1)
}

/// The border's frame around `around`, as AppKit measures.
fn frame_of(mtm: MainThreadMarker, around: Rect, look: &BorderLook) -> NSRect {
    let reach = reach(look);
    let outer = Rect::new(
        around.x - reach,
        around.y - reach,
        around.width + 2 * reach,
        around.height + 2 * reach,
    );
    let main_height = NSScreen::screens(mtm)
        .iter()
        .next()
        .map_or(0.0, |screen| screen.frame().size.height);
    let rect = space::to_appkit(outer, main_height);
    NSRect::new(
        NSPoint::new(rect.x, rect.y),
        NSSize::new(rect.width, rect.height),
    )
}

fn place(mtm: MainThreadMarker, border: &Border) {
    border
        .window
        .setFrame_display(frame_of(mtm, border.around, &border.look), true);
}

/// Draw the ring: its colour (the focused or the other), its width, its
/// corners.
fn paint(border: &Border) {
    let Some(layer) = border.window.contentView().and_then(|view| view.layer()) else {
        return;
    };
    let look = &border.look;
    let ((red, green, blue), alpha) = if border.focused {
        (look.active_color, look.active_alpha)
    } else {
        (look.inactive_color, look.inactive_alpha)
    };
    let color = CGColor::new_srgb(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
        f64::from(alpha) / 255.0,
    );
    layer.setBorderColor(Some(&color));
    layer.setBorderWidth(f64::from(look.width.max(1)));
    let corner = match look.style {
        BorderStyle::Square => 0.0,
        _ => WINDOW_CORNER + f64::from(reach(look)),
    };
    layer.setCornerRadius(corner);
}
