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
//! The styles the configuration takes are drawn as on Windows: square,
//! rounded, double (two thin rings) and dashed. (The glow, the shadow and
//! the gradient are refused by the configuration on every system.)

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
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAShapeLayer};
use std::collections::HashMap;

/// The corner of a macOS window, in points, which a rounded border
/// follows (around it, so its own corner is this and its reach). Measured
/// on macOS 27.2 in captures of windows with and without a toolbar, with
/// a sidebar, and of gevrek's own: the same for all of them, a radius of
/// 34 pixels at twice the scale.
const WINDOW_CORNER: f64 = 17.0;

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
    /// What a style draws beside the plain ring: the inner ring of a
    /// double border, the dashes of a dashed one.
    extra: Option<Retained<CALayer>>,
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
                    extra: None,
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
        let resized = (border.around.width, border.around.height) != (around.width, around.height);
        border.around = around;
        place(mtm, border);
        // (A dashed border's dashes are laid along its size.)
        if resized && border.extra.is_some() {
            paint(border);
        }
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
/// corners, its style. A double border is two thin rings with a gap
/// between, a dashed one dashes four times as long as the band is wide
/// with gaps half that, both square: as Windows draws them.
fn paint(border: &mut Border) {
    let Some(layer) = border.window.contentView().and_then(|view| view.layer()) else {
        return;
    };
    if let Some(extra) = border.extra.take() {
        extra.removeFromSuperlayer();
    }
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
    let width = f64::from(look.width.max(1));
    let size = frame_of_size(border.around, look);
    match look.style {
        BorderStyle::Double => {
            let (ring, gap) = double_rings(look.width.max(1));
            layer.setBorderColor(Some(&color));
            layer.setBorderWidth(ring);
            layer.setCornerRadius(0.0);
            let inner = CALayer::layer();
            let inset = ring + gap;
            inner.setFrame(CGRect::new(
                CGPoint::new(inset, inset),
                CGSize::new(
                    (size.0 - 2.0 * inset).max(0.0),
                    (size.1 - 2.0 * inset).max(0.0),
                ),
            ));
            inner.setBorderColor(Some(&color));
            inner.setBorderWidth(ring);
            layer.addSublayer(&inner);
            border.extra = Some(inner);
        }
        BorderStyle::Dashed => {
            layer.setBorderWidth(0.0);
            layer.setCornerRadius(0.0);
            let dashes = CAShapeLayer::layer();
            dashes.setFrame(CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(size.0, size.1),
            ));
            // (A stroke lies on both sides of its path: the path runs down
            // the middle of the band.)
            let half = width / 2.0;
            let rect = CGRect::new(
                CGPoint::new(half, half),
                CGSize::new((size.0 - width).max(0.0), (size.1 - width).max(0.0)),
            );
            // SAFETY: a rectangle's path, without a transform.
            let path = unsafe { CGPath::with_rect(rect, std::ptr::null()) };
            dashes.setPath(Some(&path));
            dashes.setFillColor(None);
            dashes.setStrokeColor(Some(&color));
            dashes.setLineWidth(width);
            let (dash, gap) = dash_lengths(look.width.max(1));
            let pattern =
                NSArray::from_retained_slice(&[NSNumber::new_f64(dash), NSNumber::new_f64(gap)]);
            dashes.setLineDashPattern(Some(&pattern));
            layer.addSublayer(&dashes);
            border.extra = Some(Retained::into_super(dashes));
        }
        _ => {
            layer.setBorderColor(Some(&color));
            layer.setBorderWidth(width);
            let corner = match look.style {
                BorderStyle::Square => 0.0,
                _ => WINDOW_CORNER + f64::from(reach(look)),
            };
            layer.setCornerRadius(corner);
        }
    }
}

/// The size of a border's window, in points.
fn frame_of_size(around: Rect, look: &BorderLook) -> (f64, f64) {
    let reach = reach(look);
    (
        f64::from(around.width + 2 * reach),
        f64::from(around.height + 2 * reach),
    )
}

/// A double border's rings and the gap between them, in a band `width`
/// wide: each a third of it (at least one point), the gap the rest.
fn double_rings(width: i32) -> (f64, f64) {
    let ring = (width / 3).max(1);
    let gap = (width - 2 * ring).max(1);
    (f64::from(ring), f64::from(gap))
}

/// A dashed border's dashes and gaps along a band `width` wide.
fn dash_lengths(width: i32) -> (f64, f64) {
    (f64::from((width * 4).max(8)), f64::from((width * 2).max(4)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_border_is_two_rings_with_a_gap() {
        assert_eq!(double_rings(6), (2.0, 2.0));
        assert_eq!(double_rings(2), (1.0, 1.0));
        assert_eq!(double_rings(1), (1.0, 1.0));
    }

    #[test]
    fn dashes_grow_with_the_band() {
        assert_eq!(dash_lengths(1), (8.0, 4.0));
        assert_eq!(dash_lengths(3), (12.0, 6.0));
    }
}
