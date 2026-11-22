//! The windows there are, from the window server, each matched to its
//! Accessibility element.
//!
//! The window server knows every window (its id, its program, its layer,
//! where it is) but cannot move one; the Accessibility API can, but knows
//! windows only program by program and by elements, not ids. A window is
//! matched by asking each element of its program which window id it is
//! (`ax::Element::window_id`). On the Mac the backend was first tried on,
//! every normal window was matched.
//!
//! Titles come from the Accessibility API: the window server gives them
//! only to a program that may record the screen.

use crate::ax::{AxError, Element};
use crate::ffi::{self, CGRect, CGWindowID, Cf};
use gevrek_model::geometry::Rect;
use std::collections::HashMap;

/// The layer of programs' ordinary windows. The menu bar, the Dock,
/// panels and the like stand on other layers.
pub const NORMAL_LAYER: i64 = 0;

/// A window as the window server lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerWindow {
    pub id: CGWindowID,
    pub pid: i32,
    /// The program's name, as the system shows it.
    pub owner: String,
    pub layer: i64,
    pub bounds: Rect,
    pub on_screen: bool,
}

/// The windows the window server lists, front to back: those on the screen
/// only, or all (those of other Spaces and minimized ones too).
pub fn server_windows(on_screen_only: bool) -> Vec<ServerWindow> {
    let options = ffi::kCGWindowListExcludeDesktopElements
        | if on_screen_only {
            ffi::kCGWindowListOptionOnScreenOnly
        } else {
            ffi::kCGWindowListOptionAll
        };
    // SAFETY: a Copy call; the array is owned.
    let Some(list) = (unsafe { Cf::owned(ffi::CGWindowListCopyWindowInfo(options, 0)) }) else {
        return Vec::new();
    };
    list.array_items().iter().filter_map(read_entry).collect()
}

fn read_entry(entry: &Cf) -> Option<ServerWindow> {
    // SAFETY: `entry` is one of the dictionaries of the window list; its
    // values are lent for as long as it lives and read at once.
    unsafe {
        let get = |key: ffi::CFStringRef| ffi::CFDictionaryGetValue(entry.as_ptr(), key);
        let mut bounds = CGRect::default();
        let raw_bounds = get(ffi::kCGWindowBounds);
        if !raw_bounds.is_null() {
            ffi::CGRectMakeWithDictionaryRepresentation(raw_bounds, &mut bounds);
        }
        let on_screen = get(ffi::kCGWindowIsOnscreen);
        Some(ServerWindow {
            id: ffi::number_of(get(ffi::kCGWindowNumber))? as CGWindowID,
            pid: ffi::number_of(get(ffi::kCGWindowOwnerPID))? as i32,
            owner: ffi::string_of(get(ffi::kCGWindowOwnerName)).unwrap_or_default(),
            layer: ffi::number_of(get(ffi::kCGWindowLayer)).unwrap_or(NORMAL_LAYER),
            bounds: crate::ax::rect_of(bounds),
            on_screen: !on_screen.is_null() && ffi::CFBooleanGetValue(on_screen),
        })
    }
}

/// The window elements of a program, by window id. Elements that are no
/// window the server knows (an id of 0) are left out.
pub fn elements_of(pid: i32) -> Result<HashMap<CGWindowID, Element>, AxError> {
    Ok(Element::application(pid)
        .windows()?
        .into_iter()
        .filter_map(|element| element.window_id().map(|id| (id, element)))
        .collect())
}

/// A window gevrek can manage: the window server's account of it and the
/// element it is moved by.
#[derive(Debug, Clone)]
pub struct Window {
    pub server: ServerWindow,
    pub element: Element,
}

/// Everything listing the windows found.
#[derive(Debug, Default)]
pub struct Listing {
    /// Normal windows matched to their elements.
    pub windows: Vec<Window>,
    /// Normal windows the window server lists without an element of theirs
    /// (the program would not answer, or does not list them).
    pub unmatched: Vec<ServerWindow>,
    /// Programs that would not list their windows, and why.
    pub refused: Vec<(i32, AxError)>,
}

/// The normal windows on the screen, matched to their elements, front to
/// back.
pub fn list() -> Listing {
    let server: Vec<ServerWindow> = server_windows(true)
        .into_iter()
        .filter(|window| window.layer == NORMAL_LAYER)
        .collect();
    let mut elements: HashMap<i32, HashMap<CGWindowID, Element>> = HashMap::new();
    let mut listing = Listing::default();
    for window in server {
        let found = elements.entry(window.pid).or_insert_with(|| {
            elements_of(window.pid).unwrap_or_else(|error| {
                listing.refused.push((window.pid, error));
                HashMap::new()
            })
        });
        match found.remove(&window.id) {
            Some(element) => listing.windows.push(Window {
                server: window,
                element,
            }),
            None => listing.unmatched.push(window),
        }
    }
    listing
}

/// One window by its id, with its element.
pub fn find(id: CGWindowID) -> Option<Window> {
    let server = server_windows(false)
        .into_iter()
        .find(|window| window.id == id)?;
    let element = elements_of(server.pid).ok()?.remove(&id)?;
    Some(Window { server, element })
}
