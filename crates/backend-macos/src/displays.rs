//! The displays: where each is, what of it the menu bar and the Dock leave
//! free, its scale, and who it is.
//!
//! The window server gives the displays and who they are (the maker, model
//! and serial number of their EDID); AppKit gives the room left free and
//! the scale (the window server's own numbers for a scaled mode are in
//! points, not pixels, so a scale cannot be had from them). The two are
//! matched by where the display is. AppKit's screens may only be read on
//! the main thread, which is where the window manager's loop runs.

use crate::ffi::{self, CGDirectDisplayID};
use crate::space::{self, AppKitRect};
use gevrek_engine::outputs::MonitorInfo;
use gevrek_model::edid::{self, Edid};
use gevrek_model::geometry::Rect;
use gevrek_model::ids::OutputId;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

/// A display as this backend reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub id: CGDirectDisplayID,
    pub name: String,
    pub bounds: Rect,
    /// What the menu bar and the Dock leave free.
    pub work_area: Rect,
    /// Pixels to a point (2 on a Retina display).
    pub scale: f64,
    pub is_main: bool,
    pub is_builtin: bool,
    pub stable_id: String,
}

/// The window server's displays, each with what AppKit says of it.
pub fn displays(mtm: MainThreadMarker) -> Vec<Display> {
    let mut ids = [0 as CGDirectDisplayID; 32];
    let mut count = 0u32;
    // SAFETY: room for 32 ids, and a place for how many there are.
    if unsafe { ffi::CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    // SAFETY: no arguments.
    let main = unsafe { ffi::CGMainDisplayID() };

    // AppKit's screens, measured from the top-left like the rest.
    let screens = NSScreen::screens(mtm);
    let main_height = screens
        .iter()
        .next()
        .map_or(0.0, |screen| screen.frame().size.height);
    let screens: Vec<(Rect, Rect, f64, String)> = screens
        .iter()
        .map(|screen| {
            let frame = space::from_appkit(appkit(screen.frame()), main_height);
            let visible = space::from_appkit(appkit(screen.visibleFrame()), main_height);
            (
                frame,
                visible,
                screen.backingScaleFactor(),
                screen.localizedName().to_string(),
            )
        })
        .collect();

    ids[..count as usize]
        .iter()
        .map(|&id| {
            // SAFETY: an active display's id.
            let bounds = crate::ax::rect_of(unsafe { ffi::CGDisplayBounds(id) });
            let screen = screens.iter().find(|(frame, ..)| *frame == bounds);
            Display {
                id,
                name: screen.map_or_else(|| format!("Display {id}"), |(.., name)| name.clone()),
                bounds,
                work_area: screen.map_or(bounds, |(_, visible, ..)| *visible),
                scale: screen.map_or(1.0, |(_, _, scale, _)| *scale),
                is_main: id == main,
                // SAFETY: an active display's id.
                is_builtin: unsafe { ffi::CGDisplayIsBuiltin(id) } != 0,
                stable_id: stable_id(id),
            }
        })
        .collect()
}

fn appkit(rect: objc2_foundation::NSRect) -> AppKitRect {
    AppKitRect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

/// Who the display is, the same as on the other systems: made of the
/// maker, the model and the serial number its EDID gives, which the window
/// server hands out as numbers. (A display that writes its serial number
/// only as text, not as the number, is told apart by that text elsewhere;
/// here it is a display without a serial number.)
fn stable_id(id: CGDirectDisplayID) -> String {
    // SAFETY: an active display's id.
    let (vendor, model, serial) = unsafe {
        (
            ffi::CGDisplayVendorNumber(id),
            ffi::CGDisplayModelNumber(id),
            ffi::CGDisplaySerialNumber(id),
        )
    };
    // The maker's three letters, packed into 16 bits as in the EDID.
    let maker = u16::try_from(vendor)
        .ok()
        .and_then(|packed| edid::maker_of((packed >> 8) as u8, packed as u8));
    match maker {
        Some(maker) => Edid {
            maker,
            product: model as u16,
            serial,
            week: 0,
            year: 0,
            name: None,
            serial_text: None,
        }
        .stable_id(),
        None => edid::stable_id_of_place(&device_path(id)),
    }
}

/// What stands for a device path: the window server's id of the display,
/// which stays with a display (it is made of the same numbers).
fn device_path(id: CGDirectDisplayID) -> String {
    format!("display:{id}")
}

/// The displays as the engine keeps them.
pub fn monitors(mtm: MainThreadMarker) -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = displays(mtm)
        .into_iter()
        .map(|display| MonitorInfo {
            handle: OutputId(display.id as isize),
            name: display.name,
            device_path: device_path(display.id),
            stable_id: display.stable_id,
            is_primary: display.is_main,
            x: display.bounds.x,
            y: display.bounds.y,
            width: display.bounds.width,
            height: display.bounds.height,
            work_x: display.work_area.x,
            work_y: display.work_area.y,
            work_width: display.work_area.width,
            work_height: display.work_area.height,
            dpi_scale: display.scale as f32,
        })
        .collect();
    gevrek_engine::outputs::number_same_stable_ids(&mut monitors);
    monitors
}
