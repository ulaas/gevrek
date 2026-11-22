//! Stackbars: the strip of tabs over a container that holds several
//! windows, one tab a window, the one shown marked.
//!
//! As a border, a stackbar is a window of gevrek's own, put directly above
//! the container's shown window. Unlike a border it is clicked: a click on
//! a tab shows that tab's window. It is a panel that does not activate
//! gevrek when clicked (the keyboard stays where it was), and its clicks
//! are taken from AppKit's queue by the runtime (`tab_at`), as gevrek does
//! not run AppKit's own event loop.
//!
//! The titles are drawn by Core Animation's text layers, in the system's
//! font, cut short with an ellipsis where a tab is too narrow.

use crate::space;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::settings::StackbarProfile;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSScreen, NSWindow, NSWindowCollectionBehavior,
    NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{kCAAlignmentCenter, kCATruncationEnd, CALayer, CATextLayer};
use std::collections::HashMap;

/// Space between the edge of a tab and its title.
const PADDING: f64 = 10.0;
/// The height of a title's letters, as a part of the bar's height.
const TEXT_HEIGHT: f64 = 0.4;
/// Height of the line that marks the shown tab.
const ACCENT: f64 = 3.0;

/// Where each tab starts and how wide it is: the bar divided evenly, the
/// first tabs one point wider where it does not divide (as on the other
/// systems).
pub fn tab_spans(bar_width: i32, tab_count: usize) -> Vec<(i32, i32)> {
    if tab_count == 0 || bar_width <= 0 {
        return Vec::new();
    }
    let count = tab_count as i32;
    let (base, remainder) = (bar_width / count, bar_width % count);
    let mut offset = 0;
    (0..count)
        .map(|index| {
            let width = base + i32::from(index < remainder);
            let span = (offset, width);
            offset += width;
            span
        })
        .collect()
}

/// The tab at a distance from the bar's left edge.
pub fn tab_at(bar_width: i32, tab_count: usize, x: i32) -> Option<usize> {
    tab_spans(bar_width, tab_count)
        .iter()
        .position(|(start, width)| x >= *start && x < start + width)
}

/// A stackbar as the desktop wants it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    /// The container's shown window: the bar is known by it.
    pub key: WindowHandle,
    /// The bar's place: the top of the container.
    pub rect: Rect,
    pub tabs: Vec<WindowHandle>,
    pub titles: Vec<String>,
    pub focused_tab: usize,
}

struct Bar {
    window: Retained<NSWindow>,
    drawn: Wanted,
    colors: [(u8, u8, u8); 3],
}

/// The stackbars, by the window each is over.
pub struct Stackbars {
    mtm: MainThreadMarker,
    bars: HashMap<WindowHandle, Bar>,
    shown: bool,
}

impl Stackbars {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            bars: HashMap::new(),
            shown: true,
        }
    }

    /// Bring the bars to what is wanted: new ones made, changed ones drawn
    /// again, those no longer wanted taken away.
    pub fn sync(&mut self, wanted: Vec<Wanted>, profile: &StackbarProfile) {
        let colors = [
            profile.background_color,
            profile.active_text_color,
            profile.inactive_text_color,
        ];
        let keys: Vec<WindowHandle> = wanted.iter().map(|bar| bar.key).collect();
        // A container that shows another of its windows keeps its bar: the
        // bar is known by the window shown, and goes over to the new one.
        for bar in &wanted {
            if self.bars.contains_key(&bar.key) {
                continue;
            }
            let handed = self
                .bars
                .iter()
                .find(|(old_key, old)| !keys.contains(old_key) && old.drawn.tabs.contains(&bar.key))
                .map(|(old_key, _)| *old_key);
            if let Some(old_key) = handed {
                let old = self.bars.remove(&old_key).expect("just found");
                self.bars.insert(bar.key, old);
            }
        }
        let gone: Vec<WindowHandle> = self
            .bars
            .keys()
            .copied()
            .filter(|key| !keys.contains(key))
            .collect();
        for key in gone {
            self.remove(key);
        }
        if !profile.enabled {
            let all: Vec<WindowHandle> = self.bars.keys().copied().collect();
            for key in all {
                self.remove(key);
            }
            return;
        }
        for bar in wanted {
            if bar.tabs.len() < 2 || bar.rect.width <= 0 || bar.rect.height <= 0 {
                self.remove(bar.key);
                continue;
            }
            let key = bar.key;
            let unchanged = self
                .bars
                .get(&key)
                .is_some_and(|old| old.drawn == bar && old.colors == colors);
            if !unchanged {
                let mtm = self.mtm;
                let entry = self.bars.entry(key).or_insert_with(|| Bar {
                    window: make_panel(mtm),
                    drawn: bar.clone(),
                    colors,
                });
                entry.drawn = bar;
                entry.colors = colors;
                place(mtm, entry);
                paint(entry);
            }
            self.order(key);
        }
    }

    /// The bar over this window goes (it left its container, or the
    /// container is not shown).
    pub fn remove(&mut self, key: WindowHandle) {
        if let Some(bar) = self.bars.remove(&key) {
            bar.window.orderOut(None);
            bar.window.close();
        }
    }

    pub fn set_all_shown(&mut self, shown: bool) {
        self.shown = shown;
        let keys: Vec<WindowHandle> = self.bars.keys().copied().collect();
        for key in keys {
            self.order(key);
        }
    }

    /// Put the bar over this window above it again (the window came to
    /// the front after the bar was ordered).
    pub fn raise(&self, key: WindowHandle) {
        self.order(key);
    }

    fn order(&self, key: WindowHandle) {
        let Some(bar) = self.bars.get(&key) else {
            return;
        };
        if self.shown {
            bar.window
                .orderWindow_relativeTo(NSWindowOrderingMode::Above, key.0);
        } else {
            bar.window.orderOut(None);
        }
    }

    /// The tab a click at `x` points from the left of the window with this
    /// window number hit: the bar's key window and the tab's place. `None`
    /// for a click on no bar.
    pub fn tab_at(&self, window_number: isize, x: f64) -> Option<(WindowHandle, usize)> {
        self.bars.iter().find_map(|(key, bar)| {
            (bar.window.windowNumber() == window_number).then(|| {
                tab_at(bar.drawn.rect.width, bar.drawn.tabs.len(), x.floor() as i32)
                    .map(|index| (*key, index))
            })?
        })
    }
}

impl Drop for Stackbars {
    fn drop(&mut self) {
        let keys: Vec<WindowHandle> = self.bars.keys().copied().collect();
        for key in keys {
            self.remove(key);
        }
    }
}

/// A panel to draw a bar in: no frame or shadow, at the level of ordinary
/// windows, clicked without gevrek becoming the active program, and shown
/// though gevrek never is that.
fn make_panel(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(10.0, 10.0));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc::<NSPanel>(),
        rect,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    // (A panel is a window: the rest is the window's.)
    let window: Retained<NSWindow> = Retained::into_super(panel);
    // SAFETY: kept by `Bar`, so not to be released when closed.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setHidesOnDeactivate(false);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
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

fn place(mtm: MainThreadMarker, bar: &Bar) {
    let main_height = NSScreen::screens(mtm)
        .iter()
        .next()
        .map_or(0.0, |screen| screen.frame().size.height);
    let rect = space::to_appkit(bar.drawn.rect, main_height);
    bar.window.setFrame_display(
        NSRect::new(
            NSPoint::new(rect.x, rect.y),
            NSSize::new(rect.width, rect.height),
        ),
        true,
    );
}

fn color((red, green, blue): (u8, u8, u8)) -> objc2_core_foundation::CFRetained<CGColor> {
    CGColor::new_srgb(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
        1.0,
    )
}

/// Draw the bar: its background, a title a tab, the shown tab marked.
fn paint(bar: &Bar) {
    let Some(root) = bar.window.contentView().and_then(|view| view.layer()) else {
        return;
    };
    let [background, active, inactive] = bar.colors;
    let wanted = &bar.drawn;
    let scale = bar.window.backingScaleFactor();
    let height = f64::from(wanted.rect.height);
    // Measured from the top, as the engine measures.
    root.setGeometryFlipped(true);
    root.setBackgroundColor(Some(&color(background)));
    // SAFETY: no sublayers: the root's own are taken away to be drawn anew.
    unsafe { root.setSublayers(None) };
    let font = (height * TEXT_HEIGHT).max(9.0);
    let text_height = font * 1.3;
    for (index, (start, width)) in tab_spans(wanted.rect.width, wanted.tabs.len())
        .into_iter()
        .enumerate()
    {
        let shown = index == wanted.focused_tab;
        let text = CATextLayer::new();
        text.setFrame(CGRect::new(
            CGPoint::new(f64::from(start) + PADDING, (height - text_height) / 2.0),
            CGSize::new((f64::from(width) - 2.0 * PADDING).max(0.0), text_height),
        ));
        let title = NSString::from_str(wanted.titles.get(index).map_or("", String::as_str));
        // SAFETY: a text layer's string is an NSString.
        unsafe { text.setString(Some(&title)) };
        text.setFontSize(font);
        text.setForegroundColor(Some(&color(if shown { active } else { inactive })));
        // SAFETY: the system's constants.
        unsafe {
            text.setAlignmentMode(kCAAlignmentCenter);
            text.setTruncationMode(kCATruncationEnd);
        }
        text.setContentsScale(scale);
        root.addSublayer(&text);
        if shown {
            let accent = CALayer::new();
            accent.setFrame(CGRect::new(
                CGPoint::new(f64::from(start), height - ACCENT),
                CGSize::new(f64::from(width), ACCENT),
            ));
            accent.setBackgroundColor(Some(&color(active)));
            root.addSublayer(&accent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_divided_evenly_the_first_tabs_taking_what_is_left() {
        assert_eq!(tab_spans(300, 3), vec![(0, 100), (100, 100), (200, 100)]);
        assert_eq!(tab_spans(302, 3), vec![(0, 101), (101, 101), (202, 100)]);
        assert!(tab_spans(300, 0).is_empty());
        assert!(tab_spans(0, 3).is_empty());
    }

    #[test]
    fn a_click_finds_its_tab() {
        assert_eq!(tab_at(300, 3, 0), Some(0));
        assert_eq!(tab_at(300, 3, 150), Some(1));
        assert_eq!(tab_at(300, 3, 299), Some(2));
        assert_eq!(tab_at(300, 3, 300), None);
        assert_eq!(tab_at(300, 3, -1), None);
    }
}
