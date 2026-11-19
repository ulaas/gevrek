//! The displays, and the room kept at their sides.
//!
//! A display is known to the zones by the id the window manager gives it
//! (`MonitorInfo::stable_id`: made of its EDID, the same on whichever
//! port it is plugged in). The shell is told the displays by the window
//! manager, ids and all, and hands them on (`Displays::tell`): that is
//! the source that counts, for it also knows which displays gevrek leaves
//! alone and how much room its own panel keeps.
//!
//! Until the window manager has told (and in a program that has none: an
//! example, a test) the displays are read from Windows here, and given
//! their ids by the very code the window manager uses
//! (`crate::display_ids`), so the two never disagree about who a display
//! is.
//!
//! The core counts in logical units; Windows counts in pixels, each
//! display with its own scale. A display's rectangle in logical units is
//! its rectangle in pixels divided by its scale, the corner as well as
//! the size, so that a point of a display in pixels is always its point
//! in logical units times the scale of that display.

use gevrek_model::geometry::{Point, Rect};
use gevrek_zones::geometry::{Display, Edge, ReservedArea};
use gevrek_zones::ids::DisplayId;
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

/// One display as it was found: who it is, and where it is in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub id: String,
    /// Windows' own name for it (`\\.\DISPLAY1`).
    pub name: String,
    /// The whole display in the pixels of the screen: x, y, width, height.
    pub pixels: (i32, i32, i32, i32),
    /// What panels and the taskbar leave of it, likewise.
    pub work: (i32, i32, i32, i32),
    /// How many pixels a logical unit is.
    pub scale: f64,
    pub primary: bool,
}

impl Found {
    fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }

    /// The display as the core knows it, in logical units.
    pub fn display(&self) -> Display {
        let logical = |pixels: i32| (f64::from(pixels) / self.scale()).round() as i32;
        let (x, y, width, height) = self.pixels;
        Display {
            id: DisplayId::new(self.id.clone()),
            rect: Rect::new(logical(x), logical(y), logical(width), logical(height)),
            scale: self.scale(),
            primary: self.primary,
        }
    }

    /// The room kept at its sides, in logical units: what lies between
    /// the display's edge and the edge of the room left for windows.
    pub fn reserved(&self) -> Vec<ReservedArea> {
        let (x, y, width, height) = self.pixels;
        let (work_x, work_y, work_width, work_height) = self.work;
        [
            (Edge::Left, work_x - x),
            (Edge::Top, work_y - y),
            (Edge::Right, (x + width) - (work_x + work_width)),
            (Edge::Bottom, (y + height) - (work_y + work_height)),
        ]
        .into_iter()
        .filter(|(_, kept)| *kept > 0)
        .map(|(edge, kept)| ReservedArea {
            display: DisplayId::new(self.id.clone()),
            edge,
            thickness: (f64::from(kept) / self.scale()).round() as i32,
        })
        .collect()
    }
}

/// A point of the desktop in logical units, as a point of the screen in
/// pixels: by the scale of the display it lies on.
///
/// Displays of different scales can cover the same logical units (a
/// display at 150 % beside one at 100 % begins, in its own units, where
/// the other has not ended yet). A point is asked about because the
/// pointer is there, so of the displays the point can lie on, the one the
/// pointer is on (`pointer`, in pixels) is taken; else the first. A point
/// on no display is taken to be on the nearest.
pub fn to_pixels(found: &[Found], at: Point, pointer: Option<(i32, i32)>) -> (i32, i32) {
    let far = |display: &Found| {
        let rect = display.display().rect;
        let beside = |value: i32, from: i32, length: i32| {
            (from - value).max(value - (from + length - 1)).max(0)
        };
        i64::from(beside(at.x, rect.x, rect.width)) + i64::from(beside(at.y, rect.y, rect.height))
    };
    let under_pointer = |display: &Found| {
        let (x, y, width, height) = display.pixels;
        pointer.is_some_and(|(px, py)| px >= x && px < x + width && py >= y && py < y + height)
    };
    let scale = found
        .iter()
        .min_by_key(|display| (far(display), !under_pointer(display)))
        .map_or(1.0, Found::scale);
    ((f64::from(at.x) * scale).round() as i32, (f64::from(at.y) * scale).round() as i32)
}

/// The displays the zones know, and who said so.
#[derive(Debug, Default)]
pub struct Displays {
    /// As Windows has them. Asked anew whenever they are needed until the
    /// window manager has told; afterwards only for where each display
    /// is, to the pixel.
    found: Vec<Found>,
    /// What the window manager told: these count from then on.
    told: Option<(Vec<Display>, Vec<ReservedArea>)>,
}

impl Displays {
    fn look(&mut self) {
        self.found = from_system();
    }

    /// The window manager's displays and the room kept on them: these
    /// count from now on. Returns whether they are others than were
    /// known.
    pub fn tell(&mut self, displays: Vec<Display>, reserved: Vec<ReservedArea>) -> bool {
        let told = Some((displays, reserved));
        let news = self.told != told;
        self.told = told;
        self.look();
        news
    }

    /// Windows says its displays changed. Returns whether what the zones
    /// know changed with it (it does not while the window manager tells:
    /// that one says so itself).
    pub fn system_changed(&mut self) -> bool {
        let before = std::mem::take(&mut self.found);
        self.look();
        self.told.is_none() && self.found != before
    }

    pub fn displays(&mut self) -> Vec<Display> {
        match &self.told {
            Some((displays, _)) => displays.clone(),
            None => {
                self.look();
                self.found.iter().map(Found::display).collect()
            }
        }
    }

    pub fn reserved(&mut self) -> Vec<ReservedArea> {
        match &self.told {
            Some((_, reserved)) => reserved.clone(),
            None => {
                self.look();
                self.found.iter().flat_map(Found::reserved).collect()
            }
        }
    }

    /// The displays with their pixels: as Windows has each one that the
    /// zones know (the window manager and Windows know a display by the
    /// same id); one Windows does not know is where its rectangle and
    /// scale say.
    fn with_pixels(&mut self) -> Vec<Found> {
        let Some((displays, _)) = &self.told else {
            self.look();
            return self.found.clone();
        };
        displays
            .iter()
            .map(|display| {
                let known = self.found.iter().find(|found| found.id == display.id.as_str());
                let scaled = |units: i32| (f64::from(units) * display.scale).round() as i32;
                let pixels = known.map_or(
                    (
                        scaled(display.rect.x),
                        scaled(display.rect.y),
                        scaled(display.rect.width),
                        scaled(display.rect.height),
                    ),
                    |known| known.pixels,
                );
                Found {
                    id: display.id.as_str().to_string(),
                    name: known.map(|known| known.name.clone()).unwrap_or_default(),
                    pixels,
                    work: pixels,
                    scale: display.scale,
                    primary: display.primary,
                }
            })
            .collect()
    }

    /// Where a display is on the screen, in pixels: x, y, width, height.
    pub fn pixels_of(&mut self, display: &DisplayId) -> Option<(i32, i32, i32, i32)> {
        self.with_pixels()
            .into_iter()
            .find(|found| found.id == display.as_str())
            .map(|found| found.pixels)
    }

    /// Windows' own name for a display (`\\.\DISPLAY1`): what a surface is
    /// placed on a display by. `None` for one Windows does not show now.
    pub fn name_of(&mut self, display: &DisplayId) -> Option<String> {
        self.with_pixels()
            .into_iter()
            .find(|found| found.id == display.as_str())
            .map(|found| found.name)
            .filter(|name| !name.is_empty())
    }

    /// See [`to_pixels`].
    pub fn to_pixels(&mut self, at: Point, pointer: Option<(i32, i32)>) -> (i32, i32) {
        to_pixels(&self.with_pixels(), at, pointer)
    }
}

/// What the window manager's state (the JSON the shell has from it) says
/// of the displays: each with the id that stays the same wherever it is
/// plugged in (`stable_id`, made of its EDID; `port:<connector>` where
/// the state has none), its rectangle and scale, and the room kept at
/// its sides (what its work area leaves of it).
///
/// The window manager counts in the pixels of the screen on Windows;
/// what comes out is in the core's logical units.
pub fn displays_of_state(state: &serde_json::Value) -> (Vec<Display>, Vec<ReservedArea>) {
    let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0).round() as i32;
    let mut displays = Vec::new();
    let mut reserved = Vec::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        let said = |key: &str| monitor[key].as_str().filter(|text| !text.is_empty());
        let Some(id) = said("stable_id")
            .map(str::to_string)
            .or_else(|| said("connector").or_else(|| said("name")).map(|port| format!("port:{port}")))
        else {
            continue;
        };
        let pixels = (
            number(&monitor["position"]["x"]),
            number(&monitor["position"]["y"]),
            number(&monitor["size"]["width"]),
            number(&monitor["size"]["height"]),
        );
        if pixels.2 <= 0 || pixels.3 <= 0 {
            continue;
        }
        // The work area where the state has one, else the sides it names.
        let work = (
            number(&monitor["work_position"]["x"]),
            number(&monitor["work_position"]["y"]),
            number(&monitor["work_size"]["width"]),
            number(&monitor["work_size"]["height"]),
        );
        let work = if work.2 > 0 && work.3 > 0 {
            work
        } else {
            let side = |key: &str| number(&monitor[key]).max(0);
            let (left, top) = (side("reserved_left"), side("reserved_top"));
            (
                pixels.0 + left,
                pixels.1 + top,
                pixels.2 - left - side("reserved_right"),
                pixels.3 - top - side("reserved_bottom"),
            )
        };
        let found = Found {
            id,
            name: said("name").unwrap_or_default().to_string(),
            pixels,
            work,
            scale: monitor["scale"].as_f64().filter(|scale| *scale > 0.0).unwrap_or(1.0),
            primary: monitor["is_primary"].as_bool().unwrap_or(false),
        };
        displays.push(found.display());
        reserved.extend(found.reserved());
    }
    (displays, reserved)
}

/// A display as Windows has it, before it has its id.
struct Seen {
    found: Found,
    /// The path of the display's device: what its id is read by.
    path: String,
    handle: isize,
}

/// The displays of the desktop, read from Windows, with the ids the
/// window manager gives them.
pub fn from_system() -> Vec<Found> {
    let mut seen: Vec<Seen> = Vec::new();
    // SAFETY: the list lives until the call returns, and is only touched
    // by the function it is handed to.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut seen as *mut Vec<Seen> as isize));
    }
    let places: Vec<(String, i32, i32, isize)> = seen
        .iter()
        .map(|display| (display.path.clone(), display.found.pixels.0, display.found.pixels.1, display.handle))
        .collect();
    let ids = crate::display_ids::stable_ids(&places);
    seen.into_iter()
        .zip(ids)
        .map(|(display, id)| Found { id, ..display.found })
        .collect()
}

unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, seen: LPARAM) -> BOOL {
    let seen = &mut *(seen.0 as *mut Vec<Seen>);
    let mut said = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    if !GetMonitorInfoW(monitor, &mut said.monitorInfo).as_bool() {
        return true.into();
    }
    let name = String::from_utf16_lossy(&said.szDevice)
        .trim_end_matches('\0')
        .to_string();
    // A display that is not in use, or is a copy another program makes
    // of the screen, is none (the window manager passes over the same).
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut device = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    if EnumDisplayDevicesW(windows::core::PCWSTR(wide.as_ptr()), 0, &mut device, 0).as_bool() {
        const ACTIVE: u32 = 0x1;
        const ATTACHED: u32 = 0x2;
        const MIRRORING: u32 = 0x8;
        let state = device.StateFlags;
        if state & ACTIVE == 0 || state & ATTACHED == 0 || state & MIRRORING != 0 {
            return true.into();
        }
    }
    let (whole, work) = (said.monitorInfo.rcMonitor, said.monitorInfo.rcWork);
    let (mut across, mut down) = (96u32, 96u32);
    let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut across, &mut down);
    let area = |rect: RECT| (rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top);
    seen.push(Seen {
        path: crate::display_ids::device_path_of(&name),
        handle: monitor.0 as isize,
        found: Found {
            id: String::new(),
            name,
            pixels: area(whole),
            work: area(work),
            // (As the window manager counts it: 96 dots to the inch is a
            // scale of one, kept as a small number and widened.)
            scale: f64::from(across.max(1) as f32 / 96.0),
            // The first of the flags: the main display.
            primary: said.monitorInfo.dwFlags & 1 != 0,
        },
    });
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(id: &str, pixels: (i32, i32, i32, i32), work: (i32, i32, i32, i32), scale: f64) -> Found {
        Found { id: id.to_string(), name: String::new(), pixels, work, scale, primary: false }
    }

    #[test]
    fn a_display_is_counted_in_logical_units_and_its_pixels_are_those_times_the_scale() {
        let plain = found("GSM-5B7F-0004E2A1", (0, 0, 1920, 1080), (0, 0, 1920, 1040), 1.0);
        assert_eq!(plain.display().rect, Rect::new(0, 0, 1920, 1080));
        assert_eq!(
            plain.reserved(),
            [ReservedArea { display: DisplayId::new("GSM-5B7F-0004E2A1"), edge: Edge::Bottom, thickness: 40 }]
        );
        // A display of 3840 by 2160 at 150 %, to the right of the first,
        // with a panel of 72 pixels at its top and a strip at its left.
        let fine = found("DEL-A0F1-noserial", (1920, 0, 3840, 2160), (1980, 72, 3780, 2088), 1.5);
        let display = fine.display();
        assert_eq!((display.rect, display.scale), (Rect::new(1280, 0, 2560, 1440), 1.5));
        assert_eq!(
            fine.reserved().iter().map(|kept| (kept.edge, kept.thickness)).collect::<Vec<_>>(),
            [(Edge::Left, 40), (Edge::Top, 48)]
        );
        // A scale that is none counts as one.
        assert_eq!(found("x", (0, 0, 800, 600), (0, 0, 800, 600), 0.0).display().scale, 1.0);

        let both = [plain, fine];
        assert_eq!(to_pixels(&both, Point { x: 100, y: 50 }, None), (100, 50));
        // Units both displays cover: the display the pointer is on.
        let at = Point { x: 1280 + 100, y: 50 };
        assert_eq!(to_pixels(&both, at, Some((1920 + 150, 75))), (1920 + 150, 75));
        assert_eq!(to_pixels(&both, at, Some((1380, 50))), (1380, 50));
        assert_eq!(to_pixels(&both, at, None), (1380, 50));
        // Units only the second covers.
        assert_eq!(to_pixels(&both, Point { x: 2000, y: 100 }, Some((5, 5))), (3000, 150));
        // Off every display: by the nearest one.
        assert_eq!(to_pixels(&both, Point { x: -10, y: 2000 }, None), (-10, 2000));
        assert_eq!(to_pixels(&[], Point { x: 7, y: 9 }, None), (7, 9));
    }

    #[test]
    fn what_the_window_manager_tells_counts_from_then_on() {
        let mut displays = Displays::default();
        let mut told = Display::new("no display of this machine", Rect::new(1280, 0, 2560, 1440));
        told.scale = 1.5;
        let kept = vec![ReservedArea { display: told.id.clone(), edge: Edge::Top, thickness: 48 }];
        assert!(displays.tell(vec![told.clone()], kept.clone()));
        assert!(!displays.tell(vec![told.clone()], kept.clone()));
        // Windows changing its mind is no news then.
        assert!(!displays.system_changed());
        assert_eq!(displays.displays(), [told.clone()]);
        assert_eq!(displays.reserved(), kept);
        // Windows does not know it: its pixels are its units at its scale.
        assert_eq!(displays.pixels_of(&told.id), Some((1920, 0, 3840, 2160)));
        assert_eq!(displays.to_pixels(Point { x: 1380, y: 50 }, None), (2070, 75));
        assert!(displays.tell(Vec::new(), Vec::new()));
        assert!(displays.displays().is_empty());
        assert_eq!(displays.pixels_of(&told.id), None);
    }

    #[test]
    fn the_window_managers_state_is_read_in_pixels_and_told_in_logical_units() {
        let state = serde_json::json!({
            "monitors": [
                {
                    "name": "LG ULTRAGEAR", "stable_id": "GSM-5B7F-0004E2A1", "connector": "\\\\?\\DISPLAY#GSM5B7F#1",
                    "is_primary": true, "scale": 1.5,
                    "position": { "x": 1920.0, "y": 0.0 }, "size": { "width": 3840, "height": 2160 },
                    "work_position": { "x": 1920.0, "y": 72.0 }, "work_size": { "width": 3840, "height": 2088 }
                },
                {
                    "name": "Old one", "stable_id": "", "connector": "HDMI-1",
                    "position": { "x": 0, "y": 0 }, "size": { "width": 1920, "height": 1080 },
                    "reserved_bottom": 40, "scale": 0.0
                },
                { "name": "No size", "stable_id": "X", "size": { "width": 0, "height": 0 } }
            ]
        });
        let (displays, reserved) = displays_of_state(&state);
        assert_eq!(displays.len(), 2);
        assert_eq!(
            (displays[0].id.as_str(), displays[0].rect, displays[0].scale, displays[0].primary),
            ("GSM-5B7F-0004E2A1", Rect::new(1280, 0, 2560, 1440), 1.5, true)
        );
        // No id of its own: its port. No scale said: one.
        assert_eq!(
            (displays[1].id.as_str(), displays[1].rect, displays[1].scale, displays[1].primary),
            ("port:HDMI-1", Rect::new(0, 0, 1920, 1080), 1.0, false)
        );
        assert_eq!(
            reserved.iter().map(|kept| (kept.display.as_str(), kept.edge, kept.thickness)).collect::<Vec<_>>(),
            [("GSM-5B7F-0004E2A1", Edge::Top, 48), ("port:HDMI-1", Edge::Bottom, 40)]
        );
        assert_eq!(displays_of_state(&serde_json::json!({})), (Vec::new(), Vec::new()));
    }

    #[test]
    fn the_displays_of_this_machine_have_ids_and_room() {
        let found = from_system();
        // (A machine that runs tests without a display has none.)
        for display in &found {
            assert!(!display.id.is_empty(), "{display:?}");
            assert!(display.pixels.2 > 0 && display.pixels.3 > 0, "{display:?}");
            assert!(display.scale >= 1.0, "{display:?}");
        }
        let mut ids: Vec<&str> = found.iter().map(|display| display.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), found.len(), "two displays have one id: {found:?}");
        assert!(found.iter().filter(|display| display.primary).count() <= 1);
    }
}
