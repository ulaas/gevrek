use super::{
    animations as animation_styles, rules::WindowInfo as RuleWindowInfo, DwmAnimationType,
    RuleEngine, RuntimeAnimationScheduler, TransitionScope,
};
use crate::node_editor::BorderStyle;
use crate::settings::{
    AnimationProfile, RuleColor, ShadowPolicy, StackbarProfile, WindowTransitionStyle,
};
use anyhow::Result;
pub use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Make `hwnd` the foreground window.
///
/// Windows lets a process do that only when it "received the last input
/// event" (or is the foreground process already). The window manager does
/// when a key binding of its own ran, but not when the command came from
/// somewhere else: the command line, the settings program, the panel, the
/// automation graph on a timer or a window event. `SetForegroundWindow`
/// then returns without changing anything, and the window only flashes in
/// the taskbar. An input event of our own (a mouse movement of nothing)
/// makes this process the one that received the last input, after which
/// the call is honoured.
unsafe fn take_foreground(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_MOUSE};
    // (A window of a program that runs as an administrator is given the
    // keyboard by the helper that runs so too.)
    if super::reach::foreground_by_helper(hwnd) {
        return;
    }
    if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
        return;
    }
    let nothing = INPUT {
        r#type: INPUT_MOUSE,
        ..Default::default()
    };
    SendInput(&[nothing], std::mem::size_of::<INPUT>() as i32);
    if !SetForegroundWindow(hwnd).as_bool() {
        log::debug!("Windows did not give {hwnd:?} the foreground");
    }
}

/// Windows are identified by the model's `WindowId` (decision D2, stage 1:
/// it carries the HWND value). The name stays for the existing code.
pub use gevrek_model::ids::WindowId as WindowHandle;

/// Why gevrek leaves a window alone (`WindowManager::ignore_reason`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreReason {
    /// One of gevrek's own windows, by its class.
    Own(String),
    /// It is on a display the configuration switches off.
    UnmanagedDisplay,
    /// What it is: a menu, a tooltip, a dialog of another window.
    Kind(crate::window_kind::Kind),
    /// Its program runs as an administrator, and no helper does.
    Elevated,
    /// Out of sight (another virtual desktop, a program not yet shown),
    /// and a rule leaves it alone so.
    Cloaked(String),
    /// A rule leaves it alone.
    Rule(String),
    /// It is gone.
    Gone,
    /// No rules are there yet, and its title says so.
    Fallback(&'static str),
}

impl IgnoreReason {
    /// Whether this comes for what passes by as the pointer does (a menu,
    /// a tooltip): said at trace only.
    pub fn passes_by(&self) -> bool {
        matches!(self, IgnoreReason::Kind(_) | IgnoreReason::Gone)
    }
}

impl std::fmt::Display for IgnoreReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IgnoreReason::Own(class) => write!(f, "it is one of gevrek's own (class {class})"),
            IgnoreReason::UnmanagedDisplay => {
                write!(f, "it is on a display the configuration leaves alone")
            }
            IgnoreReason::Kind(kind) => write!(f, "it is no window of its own ({kind:?})"),
            IgnoreReason::Elevated => write!(
                f,
                "its program runs as an administrator, and no helper for such windows answers"
            ),
            IgnoreReason::Cloaked(rule) => write!(f, "it is cloaked, and {rule} leaves it alone"),
            IgnoreReason::Rule(rule) => write!(f, "{rule} leaves it alone"),
            IgnoreReason::Gone => write!(f, "it is gone"),
            IgnoreReason::Fallback(why) => write!(f, "{why}"),
        }
    }
}

/// HWND conversions. Windows-only, so they live here and never in the model
/// or the engine.
/// From where a workspace slides in so that it is seen on no display but
/// its own: the offset asked for where the windows, set aside by it, lie
/// on no other display; else the same to the other side, then from below,
/// then from above (by the height of its own display). `None`: there is a
/// display on every side, and it does not slide.
///
/// `displays` and `targets` are rectangles (x, y, width, height); the
/// workspace's own display is the one its first window's middle is on.
pub fn slide_clear_of_displays(
    displays: &[(i32, i32, i32, i32)],
    targets: &[(i32, i32, i32, i32)],
    asked: (i32, i32),
) -> Option<(i32, i32)> {
    let holds = |display: &(i32, i32, i32, i32), x: i32, y: i32| {
        x >= display.0 && x < display.0 + display.2 && y >= display.1 && y < display.1 + display.3
    };
    let first = targets.first()?;
    let middle = (first.0 + first.2 / 2, first.1 + first.3 / 2);
    let own = displays
        .iter()
        .position(|display| holds(display, middle.0, middle.1));
    let overlaps = |a: &(i32, i32, i32, i32), b: &(i32, i32, i32, i32)| {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    };
    let clear = |offset: (i32, i32)| {
        targets.iter().all(|target| {
            let aside = (target.0 + offset.0, target.1 + offset.1, target.2, target.3);
            displays
                .iter()
                .enumerate()
                .all(|(index, display)| Some(index) == own || !overlaps(&aside, display))
        })
    };
    let height = own.map_or(first.3, |index| displays[index].3);
    let across = if asked.0 != 0 { asked.0 } else { asked.1 };
    [
        asked,
        (-asked.0, -asked.1),
        (0, height.max(1) * if across < 0 { -1 } else { 1 }),
        (0, -height.max(1) * if across < 0 { -1 } else { 1 }),
    ]
    .into_iter()
    .find(|offset| *offset != (0, 0) && clear(*offset))
}

/// Whether the scrolling layout has to show whole columns only: where a
/// column that reaches past its display's left or right edge would be
/// seen on another display (two displays whose heights overlap, so that
/// one lies beside the other). With one display, or displays one above
/// the other, a column may lie partly off the desktop, where nobody
/// sees it, and the strip scrolls freely.
pub fn columns_must_be_whole(displays: &[(i32, i32, i32, i32)]) -> bool {
    displays.iter().enumerate().any(|(at, one)| {
        displays
            .iter()
            .skip(at + 1)
            .any(|other| one.1 < other.1 + other.3 && other.1 < one.1 + one.3)
    })
}

/// The part of a window that lies on its display, in the window's own
/// coordinates (what a window's shape is given in): `None` where all of
/// it does, and the window needs no shape. Rectangles are (left, top,
/// right, bottom). A window that lies wholly outside has an empty part.
pub fn part_on_display(
    window: (i32, i32, i32, i32),
    display: (i32, i32, i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let inside = (
        window.0.max(display.0),
        window.1.max(display.1),
        window.2.min(display.2),
        window.3.min(display.3),
    );
    if inside == window {
        return None;
    }
    let (right, bottom) = (inside.2.max(inside.0), inside.3.max(inside.1));
    Some((
        inside.0 - window.0,
        inside.1 - window.1,
        right - window.0,
        bottom - window.1,
    ))
}

/// How one workspace gives way to another on a desktop whose windows are
/// seen wherever they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceWays {
    /// From where the new one's windows come (set aside by this much
    /// first); `None`: they are in their places at once.
    pub come: Option<(i32, i32)>,
    /// Where to the old one's windows go before they are hidden; `None`:
    /// they are hidden at once.
    pub go: Option<(i32, i32)>,
    /// The old one fades as the new one appears; nothing moves.
    pub fade: bool,
}

/// The ways for a change of workspace (`desktop.animations.workspace_style`):
/// sideways, up and down (by the height of the workspace's display, the
/// way the sideways offset points), or a fade. The new one comes from a
/// side where no other display is (`slide_clear_of_displays`); the old one
/// goes out the opposite way, and only where that way is clear of other
/// displays too (else it is hidden at once, as it always was).
pub fn workspace_ways(
    style: gevrek_model::settings::WorkspaceTransitionStyle,
    displays: &[(i32, i32, i32, i32)],
    incoming: &[(i32, i32, i32, i32)],
    outgoing: &[(i32, i32, i32, i32)],
    asked: (i32, i32),
) -> WorkspaceWays {
    use gevrek_model::settings::WorkspaceTransitionStyle as Style;
    if style == Style::Fade {
        return WorkspaceWays {
            come: None,
            go: None,
            fade: true,
        };
    }
    let asked = match style {
        Style::SlideVertical => {
            let first = incoming.first().or(outgoing.first());
            let height = first.map_or(0, |rect| {
                let middle = (rect.0 + rect.2 / 2, rect.1 + rect.3 / 2);
                displays
                    .iter()
                    .find(|display| {
                        middle.0 >= display.0
                            && middle.0 < display.0 + display.2
                            && middle.1 >= display.1
                            && middle.1 < display.1 + display.3
                    })
                    .map_or(rect.3, |display| display.3)
            });
            let across = if asked.0 != 0 { asked.0 } else { asked.1 };
            (0, height * across.signum())
        }
        _ => asked,
    };
    let come = slide_clear_of_displays(displays, incoming, asked);
    let opposite = match come {
        Some((x, y)) => (-x, -y),
        None => (-asked.0, -asked.1),
    };
    // The opposite way, or, where a display lies that way, up or down
    // (as the new one may come from below or above for the same reason).
    // Never the way the new one comes from: the two would cross. Before,
    // an old workspace whose way out was taken by a display was hidden at
    // once: beside another display, a change one way moved and a change
    // the other way did not, and a change to a workspace with no window
    // on it showed nothing at all.
    let height = outgoing.first().map_or(0, |rect| {
        let middle = (rect.0 + rect.2 / 2, rect.1 + rect.3 / 2);
        displays
            .iter()
            .find(|display| {
                middle.0 >= display.0
                    && middle.0 < display.0 + display.2
                    && middle.1 >= display.1
                    && middle.1 < display.1 + display.3
            })
            .map_or(rect.3, |display| display.3)
    });
    let go = [opposite, (0, height), (0, -height)]
        .into_iter()
        .filter(|way| *way != (0, 0) && *way != (-opposite.0, -opposite.1))
        .find(|way| slide_clear_of_displays(displays, outgoing, *way) == Some(*way));
    WorkspaceWays {
        come,
        go,
        fade: false,
    }
}

/// How long the windows of a workspace that gives way stay shown where
/// nothing moves (`begin_workspace_transition`): until those of the one
/// that comes are drawn.
const HELD_FOR_THE_NEXT: Duration = Duration::from_millis(120);

/// The windows of a workspace that gives way to another, between being
/// asked to hide and being hidden.
#[derive(Clone, Default)]
struct GoingOut {
    /// The windows asked to hide next are kept shown until
    /// `begin_workspace_transition` (`workspace_gives_way`).
    expected: bool,
    /// Asked to hide and not hidden yet: the window and where it belongs.
    waiting: Vec<(WindowHandle, crate::dwm::container::Rect)>,
    /// On their way out: where each belongs, how solid it was (where it
    /// fades), and when it is hidden.
    leaving: HashMap<WindowHandle, (crate::dwm::container::Rect, Option<u8>, Instant)>,
}

/// When the desktop was last given the keyboard on purpose (a workspace
/// with no window is shown).
static DESKTOP_KEPT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Whether the desktop was given the keyboard on purpose a moment ago.
/// Windows, when the window with the keyboard is hidden, gives the keyboard
/// to a window of its own choosing (on another display, as likely as not):
/// for this moment such a window is not let have it.
pub fn desktop_kept_just_now() -> bool {
    DESKTOP_KEPT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some_and(|when| when.elapsed() < std::time::Duration::from_millis(700))
}

/// A window is given the keyboard on purpose: the desktop no longer is to
/// keep it.
pub fn desktop_no_longer_kept() {
    *DESKTOP_KEPT
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
}

/// Whether a top-level window is where the desktop begins, going down
/// the order the windows lie in: Explorer's desktop, or a surface of the
/// shell kept below the windows (the one the zones are drawn on). No
/// program's window belongs below it.
fn is_desktop_band(hwnd: HWND) -> bool {
    // SAFETY: plain queries of a window; the buffer outlives the call.
    unsafe {
        let mut class = [0u16; 256];
        let length = GetClassNameW(hwnd, &mut class);
        if length <= 0 {
            return false;
        }
        let class = String::from_utf16_lossy(&class[..length as usize]);
        desktop_band_class(&class)
            && GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as i32 == 0
            && IsWindowVisible(hwnd).as_bool()
    }
}

/// The classes of the windows the desktop is made of: Explorer's, and the
/// shell's surfaces (`Gevrek_Shell_<name>`, `gevrek_shellkit::place`; of
/// those, only the ones kept below the windows are not topmost). Not the
/// shell's menus, hints or backdrops, whose classes begin otherwise.
fn desktop_band_class(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW") || class.starts_with("Gevrek_Shell_")
}

pub trait HwndExt {
    fn to_hwnd(self) -> HWND;
    fn from_hwnd(hwnd: HWND) -> Self;
}

impl HwndExt for WindowHandle {
    fn to_hwnd(self) -> HWND {
        HWND(super::hwnd_map::hwnd_of(self.0) as *mut core::ffi::c_void)
    }

    /// The id of the window that has this HWND now. A reused HWND value is
    /// a different id than the destroyed window had (see `hwnd_map`).
    fn from_hwnd(hwnd: HWND) -> Self {
        WindowHandle(super::hwnd_map::id_for(hwnd.0 as isize))
    }
}

/// Visible frame rect `(x, y, width, height)` of a window, derived from its
/// `GetWindowRect` bounds `(left, top, right, bottom)` and the invisible DWM
/// frame insets `(left, top, right, bottom)`.
///
/// This is the single authoritative conversion from a raw window rect to the
/// visible area a border must hug; every border placement path uses it.
fn visible_frame_rect(
    window_rect: (i32, i32, i32, i32),
    frame_insets: (i32, i32, i32, i32),
) -> (i32, i32, i32, i32) {
    let (left, top, right, bottom) = window_rect;
    let (inset_left, inset_top, inset_right, inset_bottom) = frame_insets;
    (
        left + inset_left,
        top + inset_top,
        (right - left) - inset_left - inset_right,
        (bottom - top) - inset_top - inset_bottom,
    )
}

/// Border window rect `(x, y, width, height)` surrounding a visible frame
/// rect `(x, y, width, height)`. A positive `border_offset` pushes the border
/// outward, a negative one pulls it over the window edge.
fn border_rect_around_frame(
    frame: (i32, i32, i32, i32),
    border_width: i32,
    border_offset: i32,
) -> (i32, i32, i32, i32) {
    let (x, y, width, height) = frame;
    let grow = border_width + border_offset;
    (x - grow, y - grow, width + grow * 2, height + grow * 2)
}

/// A border's top and height when it goes around a strip of tabs above
/// its window too (`strip` pixels high; none: as they are).
fn border_rect_with_strip(top: i32, height: i32, strip: i32) -> (i32, i32) {
    (top - strip.max(0), height + strip.max(0))
}

/// How far a strip of tabs reaches down behind its window, so that the
/// window's round upper corners show the strip and not what lies behind:
/// the radius of a window's corners, at the display's scale.
fn stackbar_overlap(dpi: u32) -> i32 {
    (8 * dpi.max(96) as i32 + 48) / 96
}

/// Border window rect for a raw `GetWindowRect` rect plus frame insets.
/// Combines [`visible_frame_rect`] and [`border_rect_around_frame`].
fn border_rect_for_window(
    window_rect: (i32, i32, i32, i32),
    frame_insets: (i32, i32, i32, i32),
    border_width: i32,
    border_offset: i32,
) -> (i32, i32, i32, i32) {
    border_rect_around_frame(
        visible_frame_rect(window_rect, frame_insets),
        border_width,
        border_offset,
    )
}

pub use gevrek_engine::decoration::{invisible_border_content_rect, stackbar_content_rect};

/// Tab `(x_offset, width)` pairs dividing a stackbar of `bar_width` pixels
/// into `tab_count` equal tabs; the first `bar_width % tab_count` tabs absorb
/// the remainder pixels so the tabs exactly cover the bar.
fn stackbar_tab_rects(bar_width: i32, tab_count: usize) -> Vec<(i32, i32)> {
    if tab_count == 0 || bar_width <= 0 {
        return Vec::new();
    }
    let count = tab_count as i32;
    let base = bar_width / count;
    let remainder = bar_width % count;
    let mut tabs = Vec::with_capacity(tab_count);
    let mut offset = 0;
    for index in 0..count {
        let width = base + i32::from(index < remainder);
        tabs.push((offset, width));
        offset += width;
    }
    tabs
}

/// Hit-test a client x-coordinate against the tabs produced by
/// [`stackbar_tab_rects`]; returns the tab index, or `None` when the point is
/// outside the tab strip.
fn stackbar_tab_index_at(bar_width: i32, tab_count: usize, x: i32) -> Option<usize> {
    stackbar_tab_rects(bar_width, tab_count)
        .iter()
        .position(|(offset, width)| x >= *offset && x < offset + width)
}

/// A square ring region `thickness` pixels wide inside the given rect.
unsafe fn rect_ring_region(x: i32, y: i32, width: i32, height: i32, thickness: i32) -> HRGN {
    let outer = CreateRectRgn(x, y, x + width, y + height);
    if thickness * 2 >= width || thickness * 2 >= height {
        return outer; // Ring fills the whole rect; nothing to cut out.
    }
    let inner = CreateRectRgn(
        x + thickness,
        y + thickness,
        x + width - thickness,
        y + height - thickness,
    );
    let _ = CombineRgn(outer, outer, inner, RGN_DIFF);
    let _ = DeleteObject(inner);
    outer
}

unsafe fn apply_border_window_region(
    border_hwnd: HWND,
    width: i32,
    height: i32,
    border_width: i32,
    border_offset: i32,
    style: BorderStyle,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    let inset = (border_width + border_offset).max(1);
    if width <= inset * 2 || height <= inset * 2 {
        return;
    }

    match style {
        BorderStyle::Rounded => {
            let radius = 16;
            let outer = CreateRoundRectRgn(0, 0, width, height, radius, radius);
            let inner_radius = (radius - inset).max(0);
            let inner = CreateRoundRectRgn(
                inset,
                inset,
                width - inset,
                height - inset,
                inner_radius,
                inner_radius,
            );
            let _ = CombineRgn(outer, outer, inner, RGN_DIFF);
            let _ = DeleteObject(inner);
            let _ = SetWindowRgn(border_hwnd, outer, true);
        }
        BorderStyle::Double => {
            // Two concentric square rings separated by a transparent gap.
            let ring_thickness = (inset / 3).max(1);
            let gap = (inset - 2 * ring_thickness).max(1);
            let inner_start = ring_thickness + gap;
            let region = rect_ring_region(0, 0, width, height, ring_thickness);
            if width > inner_start * 2 && height > inner_start * 2 {
                let inner_ring = rect_ring_region(
                    inner_start,
                    inner_start,
                    width - inner_start * 2,
                    height - inner_start * 2,
                    ring_thickness,
                );
                let _ = CombineRgn(region, region, inner_ring, RGN_OR);
                let _ = DeleteObject(inner_ring);
            }
            let _ = SetWindowRgn(border_hwnd, region, true);
        }
        BorderStyle::Dashed => {
            // Dash segments along the four edges of the border band.
            let dash_len = (inset * 4).max(8);
            let gap_len = (inset * 2).max(4);
            let region = CreateRectRgn(0, 0, 0, 0);
            // Top and bottom edges.
            let mut x = 0;
            while x < width {
                let end = (x + dash_len).min(width);
                for band_y in [0, height - inset] {
                    let segment = CreateRectRgn(x, band_y, end, band_y + inset);
                    let _ = CombineRgn(region, region, segment, RGN_OR);
                    let _ = DeleteObject(segment);
                }
                x += dash_len + gap_len;
            }
            // Left and right edges.
            let mut y = 0;
            while y < height {
                let end = (y + dash_len).min(height);
                for band_x in [0, width - inset] {
                    let segment = CreateRectRgn(band_x, y, band_x + inset, end);
                    let _ = CombineRgn(region, region, segment, RGN_OR);
                    let _ = DeleteObject(segment);
                }
                y += dash_len + gap_len;
            }
            let _ = SetWindowRgn(border_hwnd, region, true);
        }
        _ => {
            let ring = rect_ring_region(0, 0, width, height, inset);
            let _ = SetWindowRgn(border_hwnd, ring, true);
        }
    }
}

pub use gevrek_model::window::WindowInfo;

/// The overview of a workspace's windows, while it is shown: the window
/// behind the pictures, and the pictures (`dwm::overview`).
struct Overview {
    backdrop: super::overview::Backdrop,
    /// The windows that are shown, each with where its picture is.
    items: Vec<(WindowHandle, crate::dwm::container::Rect)>,
    /// The one that is pointed at.
    marked: Option<usize>,
    /// The window that had the keyboard before.
    before: Option<WindowHandle>,
    opened: Instant,
}

/// How an overview ends.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OverviewEnd {
    /// With this window chosen: it gets the keyboard.
    Chosen(WindowHandle),
    /// With none chosen: the window that had the keyboard has it again.
    Back,
    /// The keyboard went elsewhere meanwhile: it stays there.
    Away,
}

/// (A copy of the window manager's state does not own the overview's
/// window, as with `aura::Auras`: it has none.)
#[derive(Default)]
struct OverviewShown(Option<Overview>);

impl Clone for OverviewShown {
    fn clone(&self) -> Self {
        Self(None)
    }
}

#[derive(Clone)]
pub struct WindowManager {
    windows: HashMap<WindowHandle, WindowInfo>,
    /// Metadata captured the first time a window is observed. Unlike live
    /// titles, these values do not change during the window's lifetime.
    initial_window_metadata: HashMap<WindowHandle, (String, String)>,
    /// First observed client size, used by Dwindle pseudotiling.
    preferred_window_sizes: HashMap<WindowHandle, (i32, i32)>,
    /// The smallest size each window's program lets it have, as it said
    /// when it was first asked (asking is a message to the program, so
    /// it is asked once): `None`, it named none or did not answer.
    smallest_window_sizes: RefCell<HashMap<WindowHandle, Option<(i32, i32)>>>,
    /// The boxes (to minimise, to maximise) gevrek took from a window's
    /// title bar, by window: exactly those are given back.
    caption_buttons_removed: RefCell<HashMap<WindowHandle, i32>>,
    /// Whether the windows of programs that run as an administrator have
    /// a border of their own colours (`marks`).
    mark_elevated: bool,
    /// The windows of a workspace that is giving way to another.
    going_out: GoingOut,
    /// The windows of such a workspace that go out as pictures of
    /// themselves (`workspace_slides_as_pictures`): their picture stays until
    /// they are hidden, or until this moment at the latest.
    pictures_going: HashMap<WindowHandle, Instant>,
    /// The windows whose shadow and glow are left out while they slide
    /// with their workspace (`workspace_slides_without_shadows`).
    effects_resting: HashSet<WindowHandle>,
    /// The overview of a workspace's windows, while it is shown.
    overview: OverviewShown,
    /// The windows of an overview that is over, whose pictures are on
    /// their way back: the borders are back when they have arrived.
    overview_leaving: HashSet<WindowHandle>,
    /// What was hidden along with the window it belongs to (the dialogs a
    /// window has open, when the window is put away with its workspace):
    /// shown again when that window is.
    hidden_with_owner: RefCell<HashMap<WindowHandle, Vec<WindowHandle>>>,
    /// Where each window that gevrek has hidden belongs: where it was when
    /// it was hidden, or where gevrek has put it since. A hidden window is
    /// moved by messages its program answers when it gets to them, and
    /// what Windows says of where it is can be a place it only passed
    /// through (`hidden_place`).
    hidden_places: RefCell<HashMap<WindowHandle, crate::dwm::container::Rect>>,
    /// The windows that are cut at their display's edge (a column of the
    /// scrolling layout that reaches past it), each with the window's
    /// place and the display it was last cut for.
    cut: RefCell<HashMap<WindowHandle, ((i32, i32, i32, i32), (i32, i32, i32, i32))>>,
    /// Those of them whose title bar the system draws: it is told not to
    /// while they are cut (the bar it draws takes no shape), and told to
    /// again when they are whole.
    plain_frame: RefCell<HashSet<WindowHandle>>,
    border_windows: HashMap<WindowHandle, BorderInfo>, // Maps target window -> border info
    border_hwnd_set: HashSet<WindowHandle>,            // O(1) lookup for "is this a border window?"
    /// Shadow, glow, gradient border and tint of the bordered windows.
    auras: RefCell<super::aura::Auras>,
    /// Stackbar windows keyed by the container's focused window handle.
    stackbar_windows: HashMap<WindowHandle, StackbarInfo>,
    /// O(1) lookup for "is this a stackbar window?" Kept separate from
    /// `border_hwnd_set` because that set is rebuilt from `border_windows`.
    stackbar_hwnd_set: HashSet<WindowHandle>,
    /// Tab clicks queued by the stackbar window procedure; the runtime loop
    /// drains them and dispatches the group-focus path.
    stackbar_click_queue: Arc<Mutex<Vec<(WindowHandle, usize)>>>,
    rule_engine: Option<Arc<Mutex<RuleEngine>>>, // Optional RuleEngine for filtering
    fullscreen_windows: HashMap<WindowHandle, FullscreenRestore>,
    /// Windows whose title bar (`WS_CAPTION`) Gevrek removed, keyed to the
    /// original style so it can be restored exactly.
    title_bar_hidden: HashMap<WindowHandle, i32>,
    window_alphas: HashMap<WindowHandle, u8>,
    /// Windows for which Gevrek added `WS_EX_LAYERED`.
    ///
    /// Applying that style to an otherwise opaque DirectComposition window can
    /// turn its client area black. Track ownership so alpha 255 restores native
    /// composition and never removes a style that belonged to the application.
    gevrek_layered_windows: HashSet<WindowHandle>,
    window_border_colors: HashMap<WindowHandle, RuleColor>,
    window_caption_colors: HashMap<WindowHandle, RuleColor>,
    window_backdrops: HashMap<WindowHandle, SystemBackdrop>,
    window_corners: HashMap<WindowHandle, WindowCorner>,
    window_shadows: HashMap<WindowHandle, ShadowPolicy>,
    animation_scheduler: RuntimeAnimationScheduler,
    /// Per-window identity (pid, class, executable path), resolved once.
    /// Rule matching used to open the owning process for every check, i.e.
    /// for every title change of every window.
    identities: RefCell<HashMap<WindowHandle, WindowIdentity>>,
    /// Appearance before an exit animation, kept until the close is verified.
    exit_restore: HashMap<WindowHandle, ExitRestore>,
    /// The windows whose way out ends in being minimised, not closed.
    minimise_after_exit: std::collections::HashSet<WindowHandle>,
    /// Windows sent `WM_CLOSE` after an exit animation, with the time after
    /// which a window that is still alive (it cancelled the close, e.g. with
    /// a "save changes?" prompt) gets its appearance back.
    close_checks: HashMap<WindowHandle, (Instant, ExitRestore)>,
    /// The windows that are coming in, each seen as a picture of itself
    /// while it is unseen in its place (`stand_in`).
    stand_ins: super::stand_in::StandIns,
}

/// The border of a window of a program that runs as an administrator:
/// crimson, and a lighter red while it has the keyboard.
const ELEVATED: (u8, u8, u8) = (220, 20, 60);
const ELEVATED_FOCUSED: (u8, u8, u8) = (255, 99, 112);
/// No glow at all (around such a window).
const NO_GLOW: gevrek_engine::contract::GlowStrength = gevrek_engine::contract::GlowStrength {
    radius: 0,
    opacity: 0,
    inactive_opacity: 0,
};

/// What never changes for the lifetime of a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowIdentity {
    pub pid: u32,
    pub class: String,
    pub path: String,
}

/// How long a window may take to close after `WM_CLOSE` before an exit
/// animation is undone.
const CLOSE_CHECK_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
struct ExitRestore {
    rect: crate::dwm::container::Rect,
    alpha: u8,
}

pub use gevrek_engine::rules::RuleResolutionScope;

#[derive(Debug, Clone, Copy)]
struct FullscreenRestore {
    rect: RECT,
    style: i32,
    ex_style: i32,
    was_maximized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowAlphaAction {
    RestoreNative,
    ApplyLayered,
    SkipApplicationOwnedLayered,
}

fn window_alpha_action(alpha: u8, layered: bool, gevrek_owned: bool) -> WindowAlphaAction {
    if alpha == 255 {
        WindowAlphaAction::RestoreNative
    } else if layered && !gevrek_owned {
        WindowAlphaAction::SkipApplicationOwnedLayered
    } else {
        WindowAlphaAction::ApplyLayered
    }
}

pub use gevrek_engine::contract::StackbarContainerState;

#[derive(Clone)]
struct StackbarInfo {
    stackbar_hwnd: WindowHandle,
    /// How high the strip is above its window (what the window's frame
    /// reaches over).
    height: i32,
}

/// Paint/click state owned by a stackbar window procedure. Heap-allocated per
/// stackbar window; the pointer lives in `GWLP_USERDATA` and is reclaimed on
/// `WM_DESTROY`.
struct StackbarWndData {
    titles: Vec<String>,
    focused_tab: usize,
    background: COLORREF,
    active_text: COLORREF,
    inactive_text: COLORREF,
    /// What marks the tab of the window that is shown: that window's
    /// border colour (the words' colour where it has no border).
    mark: COLORREF,
    /// How high the tabs are; the window is higher by what lies behind
    /// the program's window.
    bar_height: i32,
    font: HFONT,
    font_height: i32,
    /// The family `font` was made of (empty: Segoe UI).
    font_family: String,
    key: WindowHandle,
    click_queue: Arc<Mutex<Vec<(WindowHandle, usize)>>>,
}

fn stackbar_color((r, g, b): (u8, u8, u8)) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | (r as u32))
}

/// The titles' font: of `family`, or Segoe UI where none is named. (A
/// family Windows does not have is given another by Windows itself.)
unsafe fn stackbar_create_font(bar_height: i32, family: &str) -> HFONT {
    let font_height = (bar_height - 16).clamp(10, 20);
    let family = if family.trim().is_empty() {
        "Segoe UI"
    } else {
        family.trim()
    };
    // (LOGFONT's face holds 31 letters and the end.)
    let face: Vec<u16> = family
        .encode_utf16()
        .take(31)
        .chain(std::iter::once(0))
        .collect();
    CreateFontW(
        -font_height,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
        windows::core::PCWSTR(face.as_ptr()),
    )
}

struct BorderInfo {
    border_hwnd: WindowHandle,
    width: i32,
    offset: i32,
    style: BorderStyle,
    active_color: (u8, u8, u8),
    inactive_color: (u8, u8, u8),
    active_alpha: u8,
    inactive_alpha: u8,
    is_focused: bool,
    current_color: (u8, u8, u8),
    current_alpha: u8,
    last_update: Instant, // Track when border was last updated for throttling
    /// Last known good invisible frame insets (left, top, right, bottom).
    /// Refreshed from DWM at border position time (see
    /// `query_frame_insets`); only a fallback for transient DWM failures
    /// during live resize, never a creation-time constant.
    frame_offsets: (i32, i32, i32, i32),
}

impl Clone for BorderInfo {
    fn clone(&self) -> Self {
        Self {
            border_hwnd: self.border_hwnd,
            width: self.width,
            offset: self.offset,
            style: self.style,
            active_color: self.active_color,
            inactive_color: self.inactive_color,
            active_alpha: self.active_alpha,
            inactive_alpha: self.inactive_alpha,
            is_focused: self.is_focused,
            current_color: self.current_color,
            current_alpha: self.current_alpha,
            last_update: Instant::now(), // Reset timestamp on clone
            frame_offsets: self.frame_offsets,
        }
    }
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            initial_window_metadata: HashMap::new(),
            preferred_window_sizes: HashMap::new(),
            smallest_window_sizes: RefCell::new(HashMap::new()),
            caption_buttons_removed: RefCell::new(HashMap::new()),
            mark_elevated: true,
            going_out: GoingOut::default(),
            pictures_going: HashMap::new(),
            effects_resting: HashSet::new(),
            overview: OverviewShown::default(),
            overview_leaving: HashSet::new(),
            hidden_with_owner: RefCell::new(HashMap::new()),
            hidden_places: RefCell::new(HashMap::new()),
            cut: RefCell::new(HashMap::new()),
            plain_frame: RefCell::new(HashSet::new()),
            border_windows: HashMap::new(),
            border_hwnd_set: HashSet::new(),
            auras: RefCell::new(Default::default()),
            stackbar_windows: HashMap::new(),
            stackbar_hwnd_set: HashSet::new(),
            stackbar_click_queue: Arc::new(Mutex::new(Vec::new())),
            rule_engine: None,
            fullscreen_windows: HashMap::new(),
            title_bar_hidden: HashMap::new(),
            window_alphas: HashMap::new(),
            gevrek_layered_windows: HashSet::new(),
            window_border_colors: HashMap::new(),
            window_caption_colors: HashMap::new(),
            window_backdrops: HashMap::new(),
            window_corners: HashMap::new(),
            window_shadows: HashMap::new(),
            animation_scheduler: RuntimeAnimationScheduler::default(),
            exit_restore: HashMap::new(),
            minimise_after_exit: std::collections::HashSet::new(),
            close_checks: HashMap::new(),
            identities: RefCell::new(HashMap::new()),
            stand_ins: Default::default(),
        }
    }

    pub fn new_with_rules(rule_engine: Arc<Mutex<RuleEngine>>) -> Self {
        Self {
            windows: HashMap::new(),
            initial_window_metadata: HashMap::new(),
            preferred_window_sizes: HashMap::new(),
            smallest_window_sizes: RefCell::new(HashMap::new()),
            caption_buttons_removed: RefCell::new(HashMap::new()),
            mark_elevated: true,
            going_out: GoingOut::default(),
            pictures_going: HashMap::new(),
            effects_resting: HashSet::new(),
            overview: OverviewShown::default(),
            overview_leaving: HashSet::new(),
            hidden_with_owner: RefCell::new(HashMap::new()),
            hidden_places: RefCell::new(HashMap::new()),
            cut: RefCell::new(HashMap::new()),
            plain_frame: RefCell::new(HashSet::new()),
            border_hwnd_set: HashSet::new(),
            auras: RefCell::new(Default::default()),
            stackbar_windows: HashMap::new(),
            stackbar_hwnd_set: HashSet::new(),
            stackbar_click_queue: Arc::new(Mutex::new(Vec::new())),
            border_windows: HashMap::new(),
            rule_engine: Some(rule_engine),
            fullscreen_windows: HashMap::new(),
            title_bar_hidden: HashMap::new(),
            window_alphas: HashMap::new(),
            gevrek_layered_windows: HashSet::new(),
            window_border_colors: HashMap::new(),
            window_caption_colors: HashMap::new(),
            window_backdrops: HashMap::new(),
            window_corners: HashMap::new(),
            window_shadows: HashMap::new(),
            animation_scheduler: RuntimeAnimationScheduler::default(),
            exit_restore: HashMap::new(),
            minimise_after_exit: std::collections::HashSet::new(),
            close_checks: HashMap::new(),
            identities: RefCell::new(HashMap::new()),
            stand_ins: Default::default(),
        }
    }

    pub fn set_rule_engine(&mut self, rule_engine: Arc<Mutex<RuleEngine>>) {
        self.rule_engine = Some(rule_engine);
    }

    pub fn set_animation_profile(&mut self, profile: AnimationProfile) {
        self.animation_scheduler.set_profile(profile);
    }

    pub fn animation_profile(&self) -> &AnimationProfile {
        self.animation_scheduler.profile()
    }

    pub fn active_animation_count(&self) -> usize {
        // (A workspace's windows that are still to be hidden count: the
        // loop comes round at the pace of a frame until they are.)
        self.animation_scheduler.active_count()
            + self.stand_ins.count()
            + self.going_out.waiting.len()
            + self.going_out.leaving.len()
            + self.effects_resting.len()
    }

    pub fn animation_summary(&self) -> String {
        let profile = self.animation_profile();
        if !profile.enabled {
            return "disabled".to_string();
        }
        format!(
            "layout={}ms/{:?}, workspace={}ms/{:?}, border={}ms/{:?}, window={}ms/{:?} enter={:?} exit={:?}",
            profile.layout.duration_ms,
            profile.layout.curve,
            profile.workspace.duration_ms,
            profile.workspace.curve,
            profile.border.duration_ms,
            profile.border.curve,
            profile.window.duration_ms,
            profile.window.curve,
            profile.window_enter_style,
            profile.window_exit_style,
        )
    }

    pub fn is_animation_event_suppressed(&self, handle: WindowHandle) -> bool {
        self.animation_scheduler
            .is_position_event_suppressed(handle, Instant::now())
    }

    /// Full refresh of window list - clears and re-enumerates all windows
    /// Use sparingly as this is expensive (calls EnumWindows)
    pub fn refresh_window_list(&mut self) -> Result<()> {
        self.windows.clear();

        unsafe {
            let mut context = EnumContext {
                windows: &mut self.windows,
            };
            EnumWindows(
                Some(enum_windows_proc),
                LPARAM(&mut context as *mut _ as isize),
            )?;
        }
        //commented out to reduce log spam
        //log::debug!("Window enumeration complete: {} windows found", self.windows.len());
        for (handle, info) in &self.windows {
            self.preferred_window_sizes
                .entry(*handle)
                .or_insert((info.width.max(1), info.height.max(1)));
            log::trace!(
                "  Window {}: '{}' at ({}, {}) {}x{}",
                handle.0,
                info.title,
                info.x,
                info.y,
                info.width,
                info.height
            );
        }
        let newly_observed = self
            .windows
            .iter()
            .filter(|(handle, _)| !self.initial_window_metadata.contains_key(handle))
            .map(|(handle, info)| (*handle, info.title.clone()))
            .collect::<Vec<_>>();
        for (handle, title) in newly_observed {
            let class = self.get_window_class(handle);
            self.initial_window_metadata
                .entry(handle)
                .or_insert((title, class));
        }
        Ok(())
    }

    /// Incrementally add a single window to tracking (more efficient than full
    /// refresh). Applies the same admission filter as the startup enumeration.
    pub fn track_window(&mut self, handle: WindowHandle) -> Result<bool> {
        if self.windows.contains_key(&handle) {
            return Ok(false);
        }
        let Some(info) = (unsafe { eligible_window_info(handle.to_hwnd()) }) else {
            return Ok(false);
        };
        self.preferred_window_sizes
            .entry(handle)
            .or_insert((info.width.max(1), info.height.max(1)));
        let initial_title = info.title.clone();
        let initial_class = self.get_window_class(handle);
        self.windows.insert(handle, info);
        self.initial_window_metadata
            .entry(handle)
            .or_insert((initial_title, initial_class));
        log::debug!("Tracked new window: {:?}", handle);
        Ok(true)
    }

    /// Keep the registry current for the one window an event is about.
    /// This replaces the full `EnumWindows` refresh that used to run for
    /// every create/destroy/minimize/restore/title event.
    pub fn apply_window_event(&mut self, event: &crate::dwm::WindowEvent) {
        use crate::dwm::WindowEvent;
        match *event {
            WindowEvent::Created(handle) | WindowEvent::Restored(handle) => {
                if self.windows.contains_key(&handle) {
                    let _ = self.update_window_info(handle);
                } else {
                    let _ = self.track_window(handle);
                }
            }
            WindowEvent::Destroyed(handle) => self.forget_window(handle),
            WindowEvent::Moved(handle, _, _)
            | WindowEvent::Resized(handle, _, _)
            | WindowEvent::Minimized(handle)
            | WindowEvent::DragEnded(handle) => {
                let _ = self.update_window_info(handle);
            }
            WindowEvent::MetadataChanged(handle) => {
                if let Some(info) = self.windows.get_mut(&handle) {
                    // Title only; a window's identity never changes.
                    info.title = read_window_title(handle.to_hwnd());
                } else {
                    // Many apps show a window first and title it later; the
                    // title change is when it becomes eligible.
                    let _ = self.track_window(handle);
                }
            }
            WindowEvent::Unmapped(handle) => {
                if self.is_app_unmap(handle) {
                    // Hidden by its app: no longer a candidate for management.
                    // Identity stays cached; a later show re-admits it.
                    self.windows.remove(&handle);
                }
            }
            WindowEvent::Focused(_) | WindowEvent::DragStarted(_) => {}
        }
    }

    /// True when a hide event means the app hid its own window, as opposed
    /// to a hide the WM made (workspace switch, scratchpad, cloak) or a stale
    /// event for a window that is visible again.
    pub fn is_app_unmap(&self, handle: WindowHandle) -> bool {
        !super::hidden_journal::visibility_changed_by_wm(handle)
            && self.window_exists(handle)
            && !unsafe { IsWindowVisible(handle.to_hwnd()) }.as_bool()
    }

    /// Register a window as if it had passed the admission filter. Test
    /// windows are never shown, so they would otherwise never be registered.
    #[cfg(any(test, feature = "test-support"))]
    pub fn register_for_tests(&mut self, handle: WindowHandle, title: &str) {
        self.windows.insert(
            handle,
            WindowInfo {
                handle,
                title: title.to_string(),
                x: 0,
                y: 0,
                width: 10,
                height: 10,
                is_visible: true,
                is_minimized: false,
                is_maximized: false,
            },
        );
    }

    /// Drop registry state for a window that is gone.
    fn forget_window(&mut self, handle: WindowHandle) {
        self.windows.remove(&handle);
        self.preferred_window_sizes.remove(&handle);
        self.smallest_window_sizes.borrow_mut().remove(&handle);
        self.initial_window_metadata.remove(&handle);
        self.identities.borrow_mut().remove(&handle);
        self.hidden_places.borrow_mut().remove(&handle);
    }

    /// Remove registry entries whose window no longer exists (a destroy
    /// event can be missed). Liveness is `IsWindow`; hidden windows stay.
    pub fn prune_dead_windows(&mut self) -> usize {
        let dead: Vec<WindowHandle> = self
            .windows
            .keys()
            .filter(|handle| !self.window_exists(**handle))
            .copied()
            .collect();
        for handle in &dead {
            self.forget_window(*handle);
            // (Its end was never told: the next window with this handle's
            // value is another window, as after an end that was.)
            if super::hwnd_map::id_for(super::hwnd_map::hwnd_of(handle.0)) == handle.0 {
                super::hwnd_map::retire(super::hwnd_map::hwnd_of(handle.0));
            }
        }
        self.identities
            .borrow_mut()
            .retain(|handle, _| unsafe { IsWindow(handle.to_hwnd()) }.as_bool());
        dead.len()
    }

    /// Update info for a single tracked window (efficient position/size update)
    pub fn update_window_info(&mut self, handle: WindowHandle) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();

            if !IsWindow(hwnd).as_bool() {
                // Window no longer exists, remove it
                self.windows.remove(&handle);
                self.preferred_window_sizes.remove(&handle);
                self.smallest_window_sizes.borrow_mut().remove(&handle);
                self.initial_window_metadata.remove(&handle);
                return Ok(false);
            }

            if let Some(info) = self.windows.get_mut(&handle) {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_ok() {
                    info.x = rect.left;
                    info.y = rect.top;
                    info.width = rect.right - rect.left;
                    info.height = rect.bottom - rect.top;
                    info.is_minimized = IsIconic(hwnd).as_bool();
                    info.is_maximized = IsZoomed(hwnd).as_bool();
                    info.is_visible = IsWindowVisible(hwnd).as_bool();
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    pub fn get_window_info(&self, handle: WindowHandle) -> Option<&WindowInfo> {
        self.windows.get(&handle)
    }

    pub fn get_all_windows(&self) -> Vec<WindowInfo> {
        self.windows.values().cloned().collect()
    }

    pub fn initial_window_title(&self, handle: WindowHandle) -> String {
        self.initial_window_metadata
            .get(&handle)
            .map(|(title, _)| title.clone())
            .unwrap_or_else(|| self.get_window_title_live(handle))
    }

    pub fn initial_window_class(&self, handle: WindowHandle) -> String {
        self.initial_window_metadata
            .get(&handle)
            .map(|(_, class)| class.clone())
            .unwrap_or_else(|| self.get_window_class(handle))
    }

    pub fn find_windows_by_initial_match(
        &self,
        pattern: &str,
        class: bool,
        strategy: &crate::dwm::MatchStrategy,
        case_sensitive: bool,
    ) -> Vec<WindowHandle> {
        self.windows
            .keys()
            .filter(|&&handle| {
                let value = if class {
                    self.initial_window_class(handle)
                } else {
                    self.initial_window_title(handle)
                };
                if matches!(strategy, crate::dwm::MatchStrategy::Regex) {
                    return regex::RegexBuilder::new(pattern)
                        .case_insensitive(!case_sensitive)
                        .build()
                        .is_ok_and(|regex| regex.is_match(&value));
                }
                let (value, pattern) = if case_sensitive {
                    (value, pattern.to_string())
                } else {
                    (value.to_lowercase(), pattern.to_lowercase())
                };
                match strategy {
                    crate::dwm::MatchStrategy::Equals => value == pattern,
                    crate::dwm::MatchStrategy::Contains => value.contains(&pattern),
                    crate::dwm::MatchStrategy::StartsWith => value.starts_with(&pattern),
                    crate::dwm::MatchStrategy::EndsWith => value.ends_with(&pattern),
                    crate::dwm::MatchStrategy::Regex => unreachable!(),
                }
            })
            .copied()
            .collect()
    }

    pub fn preferred_window_size(&self, handle: WindowHandle) -> Option<(i32, i32)> {
        self.preferred_window_sizes.get(&handle).copied()
    }

    /// Check if a window is minimized (iconified)
    pub fn is_window_minimized(&self, handle: WindowHandle) -> bool {
        unsafe { IsIconic(handle.to_hwnd()).as_bool() }
    }

    /// Check live Windows visibility rather than the periodically refreshed
    /// snapshot. Workspace switching and scratchpads can change this between
    /// metadata refreshes.
    pub fn is_window_visible(&self, handle: WindowHandle) -> bool {
        unsafe {
            IsWindow(handle.to_hwnd()).as_bool()
                && IsWindowVisible(handle.to_hwnd()).as_bool()
                && !IsIconic(handle.to_hwnd()).as_bool()
        }
    }

    /// Check if a window should be excluded from window management
    /// Returns true if the window should NOT be managed (should be ignored)
    pub fn should_ignore_window(&self, handle: WindowHandle) -> bool {
        self.ignore_reason(handle).is_some()
    }

    /// Why a window is not managed, or `None` where it is. Asked again and
    /// again (for each event of a window): nothing is said here; the
    /// runtime says it once, when the window opens.
    pub fn ignore_reason(&self, handle: WindowHandle) -> Option<IgnoreReason> {
        use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

        // CRITICAL: Always check for Gevrek internal windows FIRST, before any other logic
        // This ensures our own windows are NEVER managed, even if rules haven't been set up yet
        unsafe {
            let hwnd = handle.to_hwnd();
            let mut class_name: [u16; 256] = [0; 256];
            let class_len = GetClassNameW(hwnd, &mut class_name);
            if class_len > 0 {
                let class_str = String::from_utf16_lossy(&class_name[0..class_len as usize]);
                // Check ALL Gevrek internal window classes (fast path before rule engine)
                if class_str.starts_with("Gevrek_Border")
                    || class_str.starts_with("Gevrek_Stackbar")
                    || class_str.starts_with("Gevrek_Window")
                    || class_str.starts_with("Gevrek_Overlay")
                    || class_str.starts_with("Gevrek_Shell")
                    || class_str == "GevrekHotkeyWindow"
                {
                    return Some(IgnoreReason::Own(class_str));
                }
            }
        }

        // A window on a display the configuration switches off is not ours.
        if crate::dwm::monitor::is_on_unmanaged_display(handle.to_hwnd()) {
            return Some(IgnoreReason::UnmanagedDisplay);
        }

        // (The window as Windows has it: gevrek's own number for a window
        // is another once a handle's value has been given out again.)
        let native = handle.to_hwnd().0 as isize;

        // Nor is what only passes by (a notification a program draws
        // itself, a display of the volume), nor what belongs to another
        // window (a dialog, a menu): `window_kind.rs`.
        let kind = crate::window_kind::of_window(native);
        if !kind.is_managed() {
            return Some(IgnoreReason::Kind(kind));
        }

        // Nor is one of a program with an administrator's rights, where
        // gevrek has none: Windows lets it be neither moved nor resized,
        // and a tile kept for it would stay empty (`elevated.rs`).
        // (With the helper that runs so too it is a window as any other:
        // `elevated_helper`.)
        if crate::elevated::window_is_beyond_reach(native)
            && !crate::elevated_helper::client::connected()
        {
            return Some(IgnoreReason::Elevated);
        }

        // If we have a RuleEngine, use it for filtering
        if let Some(rule_engine_arc) = &self.rule_engine {
            if let Ok(engine) = rule_engine_arc.lock() {
                // Build WindowInfo for rule checking
                // (A window that is gone meanwhile is nobody's to manage.)
                return match self.rule_window_info(handle) {
                    Some(rule_window_info) => {
                        let cloaked = rule_window_info.is_cloaked;
                        engine.ignored_by(&rule_window_info).map(|rule| {
                            if cloaked {
                                IgnoreReason::Cloaked(rule)
                            } else {
                                IgnoreReason::Rule(rule)
                            }
                        })
                    }
                    None => Some(IgnoreReason::Gone),
                };
            }
        }

        // Fallback: Basic hardcoded checks if no RuleEngine
        if let Some(window_info) = self.windows.get(&handle) {
            let title = window_info.title.to_lowercase();

            // Always ignore Gevrek's own windows (config editor, etc.)
            if title.contains("gevrek") {
                return Some(IgnoreReason::Fallback("its title says it is gevrek's"));
            }

            // Ignore common dialog/popup windows
            if title.contains("dialog") || title.contains("popup") {
                return Some(IgnoreReason::Fallback("its title says it is a dialog"));
            }
        }

        None
    }

    /// Resolve the rules for a live window. The window's tags are engine
    /// state and are passed in.
    pub fn resolve_window_rules_with_context(
        &self,
        handle: WindowHandle,
        workspace: Option<String>,
        workspace_index: Option<usize>,
        scope: RuleResolutionScope,
        tags: &gevrek_engine::rule_state::RuleMetadataMap,
    ) -> crate::dwm::RuleResolution {
        let Some(rule_engine) = &self.rule_engine else {
            return crate::dwm::RuleResolution::default();
        };
        let Ok(engine) = rule_engine.lock() else {
            return crate::dwm::RuleResolution::default();
        };
        {
            self.rule_window_info(handle)
                .map(|mut window| {
                    window.tags = match scope {
                        RuleResolutionScope::All => tags.tags(handle),
                        RuleResolutionScope::Static | RuleResolutionScope::Dynamic => {
                            tags.base_tags(handle)
                        }
                    };
                    window.workspace = workspace;
                    window.workspace_index = workspace_index;
                    match scope {
                        RuleResolutionScope::All => engine.resolve(&window),
                        RuleResolutionScope::Static => engine.resolve_static(&window),
                        RuleResolutionScope::Dynamic => engine.resolve_dynamic(&window),
                    }
                })
                .unwrap_or_default()
        }
    }

    /// Whether a window floats: as a rule of the person's says; where none
    /// speaks of it, as the window itself is made (a dialog, a window of
    /// one size, one kept above the others: `window_kind::floats`); and
    /// else as the rules gevrek comes with say.
    pub fn should_float_window(&self, handle: WindowHandle) -> bool {
        let facts = self.rule_window_info(handle);
        let engine = self
            .rule_engine
            .as_ref()
            .and_then(|rule_engine| rule_engine.lock().ok());
        if let (Some(engine), Some(facts)) = (engine.as_ref(), facts.as_ref()) {
            if let Some(floating) = engine.resolve_static(facts).floating {
                return floating;
            }
        }
        if crate::window_kind::window_floats(handle.to_hwnd().0 as isize) {
            return true;
        }
        match (engine, facts) {
            (Some(engine), Some(facts)) => engine.should_float(&facts),
            _ => false,
        }
    }

    /// The window's identity, resolved once per window. The cached entry is
    /// revalidated against the owning pid, so a reused HWND is resolved again.
    pub fn window_identity(&self, handle: WindowHandle) -> WindowIdentity {
        let hwnd = handle.to_hwnd();
        let pid = unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            pid
        };
        if let Some(identity) = self.identities.borrow().get(&handle) {
            if pid != 0 && identity.pid == pid {
                return identity.clone();
            }
        }
        let class = read_window_class(hwnd);
        // A UWP frame belongs to ApplicationFrameHost.exe; the app itself is
        // the process of its CoreWindow child. Rules match the app. Until the
        // child exists (app still starting) the identity is not cached, so it
        // is resolved again on the next lookup.
        let (app_pid, cacheable) = if class == UWP_FRAME_CLASS {
            match uwp_app_pid(hwnd) {
                Some(app_pid) => (app_pid, true),
                None => (pid, false),
            }
        } else {
            (pid, true)
        };
        let identity = WindowIdentity {
            pid,
            class,
            path: if app_pid == 0 {
                String::new()
            } else {
                process_image_path(app_pid)
            },
        };
        if pid != 0 && cacheable {
            self.identities
                .borrow_mut()
                .insert(handle, identity.clone());
        }
        identity
    }

    /// Query the window class name for a given handle.
    pub fn get_window_class(&self, handle: WindowHandle) -> String {
        self.window_identity(handle).class
    }

    /// Query the executable name (e.g. "notepad.exe") for a given window handle.
    pub fn get_window_exe(&self, handle: WindowHandle) -> String {
        self.get_window_path(handle)
            .rsplit('\\')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    /// The smallest size a window's program lets it have (what it
    /// answers to `WM_GETMINMAXINFO`, as when its edge is dragged), in
    /// pixels; `None` where it names none or does not answer in time.
    /// Asked once for each window and kept.
    pub fn smallest_window_size(&self, handle: WindowHandle) -> Option<(i32, i32)> {
        if let Some(known) = self.smallest_window_sizes.borrow().get(&handle) {
            return *known;
        }
        let mut limits = MINMAXINFO::default();
        // SAFETY: the message carries a pointer to `limits`, which lives
        // until the call returns; the system carries the structure to
        // the window's program and back. A program that hangs is not
        // waited for.
        let answered = unsafe {
            SendMessageTimeoutW(
                handle.to_hwnd(),
                WM_GETMINMAXINFO,
                WPARAM(0),
                LPARAM(&mut limits as *mut MINMAXINFO as isize),
                SMTO_ABORTIFHUNG,
                40,
                None,
            )
        };
        let least = (limits.ptMinTrackSize.x, limits.ptMinTrackSize.y);
        let named = (answered.0 != 0 && (least.0 > 0 || least.1 > 0))
            .then_some((least.0.max(0), least.1.max(0)));
        self.smallest_window_sizes
            .borrow_mut()
            .insert(handle, named);
        named
    }

    /// Query the process ID that owns a window.
    pub fn get_window_pid(&self, handle: WindowHandle) -> Option<u32> {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(handle.to_hwnd(), Some(&mut pid));
            (pid != 0).then_some(pid)
        }
    }

    /// Query the full executable path for a given window handle.
    pub fn get_window_path(&self, handle: WindowHandle) -> String {
        self.window_identity(handle).path
    }

    /// Get the current title of a window (live query, not cached).
    pub fn get_window_title_live(&self, handle: WindowHandle) -> String {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        unsafe {
            let hwnd = handle.to_hwnd();
            let mut buf: [u16; 256] = [0; 256];
            let len = GetWindowTextW(hwnd, &mut buf);
            if len > 0 {
                String::from_utf16_lossy(&buf[0..len as usize])
            } else {
                String::new()
            }
        }
    }

    /// Find all windows matching a pattern (by title, class, or exe).
    /// Returns all matching WindowHandles from tracked windows.
    pub fn find_windows_by_pattern(
        &self,
        pattern: &str,
        match_kind: &crate::node_editor::RuleMatchKind,
        match_strategy: &crate::node_editor::RuleMatchStrategy,
    ) -> Vec<WindowHandle> {
        use crate::node_editor::{RuleMatchKind, RuleMatchStrategy};
        let kind = match match_kind {
            RuleMatchKind::Title => crate::dwm::MatchKind::Title,
            RuleMatchKind::Class => crate::dwm::MatchKind::Class,
            RuleMatchKind::Exe => crate::dwm::MatchKind::Exe,
            RuleMatchKind::Path => crate::dwm::MatchKind::Path,
            RuleMatchKind::AppId => crate::dwm::MatchKind::AppId,
        };
        let strategy = match match_strategy {
            RuleMatchStrategy::Equals => crate::dwm::MatchStrategy::Equals,
            RuleMatchStrategy::Contains => crate::dwm::MatchStrategy::Contains,
            RuleMatchStrategy::StartsWith => crate::dwm::MatchStrategy::StartsWith,
            RuleMatchStrategy::EndsWith => crate::dwm::MatchStrategy::EndsWith,
            RuleMatchStrategy::Regex => crate::dwm::MatchStrategy::Regex,
        };
        self.find_windows_by_match(pattern, &kind, &strategy, false)
    }

    pub fn find_windows_by_match(
        &self,
        pattern: &str,
        match_kind: &crate::dwm::MatchKind,
        match_strategy: &crate::dwm::MatchStrategy,
        case_sensitive: bool,
    ) -> Vec<WindowHandle> {
        self.windows
            .keys()
            .filter(|&&handle| {
                self.window_matches(handle, pattern, match_kind, match_strategy, case_sensitive)
            })
            .copied()
            .collect()
    }

    pub fn window_matches(
        &self,
        handle: WindowHandle,
        pattern: &str,
        match_kind: &crate::dwm::MatchKind,
        match_strategy: &crate::dwm::MatchStrategy,
        case_sensitive: bool,
    ) -> bool {
        let value = match match_kind {
            crate::dwm::MatchKind::Title => self.get_window_title_live(handle),
            crate::dwm::MatchKind::Class => self.get_window_class(handle),
            crate::dwm::MatchKind::Exe => self.get_window_exe(handle),
            crate::dwm::MatchKind::Path => self.get_window_path(handle),
            crate::dwm::MatchKind::AppId => {
                gevrek_engine::contract::app_id_of_exe(&self.get_window_exe(handle))
            }
            _ => return false,
        };
        if matches!(match_strategy, crate::dwm::MatchStrategy::Regex) {
            return regex::RegexBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .build()
                .map(|regex| regex.is_match(&value))
                .unwrap_or(false);
        }
        let (value, pattern) = if case_sensitive {
            (value, pattern.to_string())
        } else {
            (value.to_lowercase(), pattern.to_lowercase())
        };
        match match_strategy {
            crate::dwm::MatchStrategy::Equals => value == pattern,
            crate::dwm::MatchStrategy::Contains => value.contains(&pattern),
            crate::dwm::MatchStrategy::StartsWith => value.starts_with(&pattern),
            crate::dwm::MatchStrategy::EndsWith => value.ends_with(&pattern),
            crate::dwm::MatchStrategy::Regex => unreachable!(),
        }
    }

    pub fn get_active_window(&self) -> Option<WindowHandle> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if !hwnd.is_invalid() {
                Some(WindowHandle::from_hwnd(hwnd))
            } else {
                None
            }
        }
    }

    pub fn move_window(&self, handle: WindowHandle, x: i32, y: i32) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let mut rect = RECT::default();
                if let Err(e) = GetWindowRect(hwnd, &mut rect) {
                    log::warn!("Failed to get window rect for move: {}", e);
                    return Ok(()); // Skip this window
                }
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;

                // Use ASYNCWINDOWPOS and NOACTIVATE to prevent visible shuffling
                if let Err(e) = super::reach::set_window_pos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                ) {
                    log::warn!(
                        "Failed to move window (might be protected/system window): {}",
                        e
                    );
                    return Ok(()); // Continue without crashing
                }
                note_posted_move(hwnd, (x, y, width, height));
            }
        }
        Ok(())
    }

    pub fn resize_window(&self, handle: WindowHandle, width: i32, height: i32) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let mut rect = RECT::default();
                if let Err(e) = GetWindowRect(hwnd, &mut rect) {
                    log::warn!("Failed to get window rect for resize: {}", e);
                    return Ok(()); // Skip this window
                }

                // Use ASYNCWINDOWPOS and NOACTIVATE to prevent visible shuffling
                if let Err(e) = super::reach::set_window_pos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                ) {
                    log::warn!(
                        "Failed to resize window (might be protected/system window): {}",
                        e
                    );
                    return Ok(()); // Continue without crashing
                }
                note_posted_move(hwnd, (rect.left, rect.top, width, height));
            }
        }
        Ok(())
    }

    /// Query the current invisible frame insets (left, top, right, bottom),
    /// clamped to a plausible range.
    ///
    /// Returns `None` when DWM cannot report the extended frame bounds or
    /// reports transient garbage (e.g. while a live resize is in flight), so
    /// callers can keep their last known good value instead of caching it.
    /// Frame insets go stale after DPI, style, or theme changes, so border
    /// positioning calls this instead of trusting a creation-time cache.
    fn query_frame_insets(&self, hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let mut window_rect = RECT::default();
            GetWindowRect(hwnd, &mut window_rect).ok()?;

            let mut frame_rect = RECT::default();
            DwmGetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(9), // DWMWA_EXTENDED_FRAME_BOUNDS
                &mut frame_rect as *mut _ as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
            .ok()?;

            // The visible frame is always inside the window rect, and the
            // invisible DWM margins are a few pixels per side. Anything
            // outside 0..=64 is a transient artifact; reject the sample.
            let plausible = |value: i32| (0..=64).contains(&value).then_some(value);
            Some((
                plausible(frame_rect.left - window_rect.left)?,
                plausible(frame_rect.top - window_rect.top)?,
                plausible(window_rect.right - frame_rect.right)?,
                plausible(window_rect.bottom - frame_rect.bottom)?,
            ))
        }
    }

    /// Get the window frame offsets (invisible borders)
    /// Returns (left, top, right, bottom) offsets between window rect and visible rect.
    /// Uses the same plausibility clamp as `query_frame_insets`, so a transient
    /// or DPI-mixed DWM sample never shifts a tile by a huge bogus inset.
    fn get_window_frame_offsets(&self, hwnd: HWND) -> (i32, i32, i32, i32) {
        self.query_frame_insets(hwnd).unwrap_or((0, 0, 0, 0))
    }

    /// Move and resize window in one operation (more efficient)
    /// Accounts for Windows' invisible window borders to ensure pixel-perfect positioning
    pub fn move_and_resize_window(
        &self,
        handle: WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // Get window frame offsets to account for invisible borders
                let (left_offset, top_offset, right_offset, bottom_offset) =
                    self.get_window_frame_offsets(hwnd);

                // Adjust position and size to account for invisible borders
                // We want the VISIBLE window to be at (x, y) with size (width, height)
                // So we need to offset the window position by the invisible border size
                let adjusted_x = x - left_offset;
                let adjusted_y = y - top_offset;
                let adjusted_width = width + left_offset + right_offset;
                let adjusted_height = height + top_offset + bottom_offset;

                log::trace!("🪟 SetWindowPos: hwnd={:?}, requested=({},{} {}x{}), offsets=({},{},{},{}), adjusted=({},{} {}x{})",
                    hwnd, x, y, width, height,
                    left_offset, top_offset, right_offset, bottom_offset,
                    adjusted_x, adjusted_y, adjusted_width, adjusted_height);

                // Use NOACTIVATE to prevent visible shuffling
                // Removed SWP_ASYNCWINDOWPOS to ensure synchronous positioning
                let flags = nonblocking_swp(hwnd, SWP_NOZORDER | SWP_NOACTIVATE);
                if let Err(e) = super::reach::set_window_pos(
                    hwnd,
                    None,
                    adjusted_x,
                    adjusted_y,
                    adjusted_width,
                    adjusted_height,
                    flags,
                ) {
                    log::warn!(
                        "Failed to move/resize window (might be protected/system window): {}",
                        e
                    );
                    return Ok(()); // Continue without crashing
                }
                if flags.contains(SWP_ASYNCWINDOWPOS) {
                    note_posted_move(
                        hwnd,
                        (adjusted_x, adjusted_y, adjusted_width, adjusted_height),
                    );
                }
                self.note_placed(
                    handle,
                    crate::dwm::container::Rect::new(x, y, width, height),
                );
                log::trace!("✓ SetWindowPos succeeded for hwnd={:?}", hwnd);
            } else {
                log::warn!("⚠️ IsWindow returned false for handle {:?}", handle);
            }
        }
        Ok(())
    }

    /// Remove a window from tracking (without closing it)
    pub fn untrack_window(&mut self, handle: WindowHandle) {
        self.end_stand_in(handle);
        let _ = self.restore_native_window_alpha(handle);
        self.caption_buttons_removed.borrow_mut().remove(&handle);
        self.show_owned_windows(handle);
        self.hidden_places.borrow_mut().remove(&handle);
        self.windows.remove(&handle);
        self.preferred_window_sizes.remove(&handle);
        self.smallest_window_sizes.borrow_mut().remove(&handle);
        self.initial_window_metadata.remove(&handle);
        self.fullscreen_windows.remove(&handle);
        self.window_alphas.remove(&handle);
        self.window_border_colors.remove(&handle);
        self.window_caption_colors.remove(&handle);
        self.window_backdrops.remove(&handle);
        self.window_corners.remove(&handle);
        self.window_shadows.remove(&handle);
    }

    pub fn close_window(&mut self, handle: WindowHandle) -> Result<()> {
        if self.animation_scheduler.is_close_pending(handle) {
            return Ok(());
        }
        // (A window still coming in goes from where it is to be seen.)
        self.end_stand_in(handle);
        let profile = self.animation_scheduler.profile();
        let style = profile.window_exit_style;
        if self
            .animation_scheduler
            .scope_enabled(TransitionScope::Explicit)
            && style != WindowTransitionStyle::None
        {
            let start = self.get_window_rect(handle)?;
            let now = Instant::now();
            let alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
            let scheduled = match style {
                WindowTransitionStyle::None => false,
                WindowTransitionStyle::Fade => self
                    .animation_scheduler
                    .schedule_exit_opacity(handle, alpha, now),
                WindowTransitionStyle::Slide => {
                    let target = animation_styles::offset_rect(start, start.width / 2, 0);
                    self.animation_scheduler
                        .schedule_exit_rect(handle, start, target, now)
                }
                WindowTransitionStyle::Scale => {
                    let target = animation_styles::centered_scaled_rect(start, 20);
                    self.animation_scheduler
                        .schedule_exit_rect(handle, start, target, now)
                }
            };
            if scheduled {
                self.exit_restore
                    .insert(handle, ExitRestore { rect: start, alpha });
                return Ok(());
            }
        }
        self.close_window_immediate(handle)
    }

    fn close_window_immediate(&mut self, handle: WindowHandle) -> Result<()> {
        self.end_stand_in(handle);
        self.animation_scheduler.cancel_window(handle);
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                super::reach::post_close(hwnd)?;
                self.windows.remove(&handle);
                self.preferred_window_sizes.remove(&handle);
                self.smallest_window_sizes.borrow_mut().remove(&handle);
                self.initial_window_metadata.remove(&handle);
                self.window_alphas.remove(&handle);
                self.window_border_colors.remove(&handle);
                self.window_caption_colors.remove(&handle);
                self.window_backdrops.remove(&handle);
                self.window_corners.remove(&handle);
                self.window_shadows.remove(&handle);
            }
        }
        Ok(())
    }

    /// Cut these windows at their displays' edges (each with its
    /// display's rectangle: left, top, right, bottom), and let every
    /// other window that was cut be whole again. A window's shape is in
    /// its own coordinates, so it is made again when the window has
    /// moved; where nothing is another, nothing is asked of the system.
    /// The border around a window is cut with it.
    pub fn cut_at_displays(&self, windows: &[(WindowHandle, (i32, i32, i32, i32))]) {
        let mut cut = self.cut.borrow_mut();
        let mut plain_frame = self.plain_frame.borrow_mut();
        // Whether the system draws the window's frame (its title bar):
        // as the window's style says, or not at all. The bar the system
        // draws is not cut by a window's shape (seen on Windows 11: it
        // stays on the display beside); the plain one a window gets
        // without it is.
        let system_frame = |hwnd: HWND, drawn: bool| {
            let policy = if drawn {
                DWMNCRP_USEWINDOWSTYLE
            } else {
                DWMNCRP_DISABLED
            };
            // SAFETY: a value of the kind the attribute takes, and its size.
            unsafe {
                let _ = super::reach::dwm_set_window_attribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    &policy as *const _ as *const core::ffi::c_void,
                    std::mem::size_of_val(&policy) as u32,
                );
            }
        };
        // Whether a window has a title bar the system draws: what it
        // draws in itself begins well below its top.
        let has_title_bar = |hwnd: HWND| -> bool {
            let (mut outer, mut inner) = (RECT::default(), POINT::default());
            // SAFETY: plain queries.
            unsafe {
                GetWindowRect(hwnd, &mut outer).is_ok()
                    && ClientToScreen(hwnd, &mut inner).as_bool()
                    && inner.y - outer.top >= 16
            }
        };
        let shape = |hwnd: HWND, part: Option<(i32, i32, i32, i32)>| {
            // SAFETY: the shape is the window's once it is given to it;
            // none makes the window whole. A window that is gone, or
            // takes no shape, is left as it is.
            unsafe {
                if !IsWindow(hwnd).as_bool() {
                    return;
                }
                match part {
                    Some((left, top, right, bottom)) => {
                        let region = CreateRectRgn(left, top, right, bottom);
                        if !region.is_invalid() && SetWindowRgn(hwnd, region, true) == 0 {
                            let _ = DeleteObject(region);
                        }
                    }
                    None => {
                        let _ = SetWindowRgn(hwnd, HRGN::default(), true);
                    }
                }
            }
        };
        let rect_of = |hwnd: HWND| -> Option<(i32, i32, i32, i32)> {
            let mut rect = RECT::default();
            // SAFETY: a plain query.
            unsafe { GetWindowRect(hwnd, &mut rect).ok()? };
            Some((rect.left, rect.top, rect.right, rect.bottom))
        };
        // Those that are no longer to be cut are whole again.
        let gone: Vec<WindowHandle> = cut
            .keys()
            .filter(|known| !windows.iter().any(|(window, _)| window == *known))
            .copied()
            .collect();
        for window in gone {
            cut.remove(&window);
            shape(window.to_hwnd(), None);
            if plain_frame.remove(&window) {
                system_frame(window.to_hwnd(), true);
            }
            if let Some(border) = self.border_windows.get(&window) {
                shape(border.border_hwnd.to_hwnd(), None);
            }
        }
        for (window, display) in windows {
            let hwnd = window.to_hwnd();
            let Some(place) = rect_of(hwnd) else {
                continue;
            };
            if cut.get(window) == Some(&(place, *display)) {
                continue;
            }
            let part = part_on_display(place, *display);
            // (One that lies wholly on its display, and was never cut,
            // is not touched at all.)
            if part.is_none() && !cut.contains_key(window) {
                continue;
            }
            // (Before the shape: the frame's change takes a shape away.)
            if part.is_some() && !plain_frame.contains(window) && has_title_bar(hwnd) {
                plain_frame.insert(*window);
                system_frame(hwnd, false);
            }
            shape(hwnd, part);
            if part.is_none() && plain_frame.remove(window) {
                system_frame(hwnd, true);
            }
            if let Some(border) = self.border_windows.get(window) {
                let border = border.border_hwnd.to_hwnd();
                if let Some(around) = rect_of(border) {
                    // (A border has a shape of its own, a ring: cut, it
                    // is the part of its rectangle on the display, which
                    // lies behind the window where the ring's hole was.)
                    if let Some(part) = part_on_display(around, *display) {
                        shape(border, Some(part));
                    }
                }
            }
            match part {
                Some(_) => {
                    cut.insert(*window, (place, *display));
                }
                None => {
                    cut.remove(window);
                }
            }
        }
    }

    /// The windows hidden next are a workspace's that gives way to
    /// another: they stay shown until `begin_workspace_transition` says
    /// how they go (or, if that never comes, until the next pass).
    pub fn workspace_gives_way(&mut self) {
        self.close_window_overview(OverviewEnd::Away);
        self.going_out.expected = true;
    }

    /// A window that was kept shown to go out is as any other again: one
    /// that waits is forgotten, one on its way is stopped and put where
    /// it belongs, as solid as it was. `hide`: and hidden.
    fn stop_going_out(&mut self, handle: WindowHandle, hide: bool) {
        let waited = self
            .going_out
            .waiting
            .iter()
            .position(|(waiting, _)| *waiting == handle)
            .map(|at| self.going_out.waiting.remove(at));
        let left = self.going_out.leaving.remove(&handle);
        if waited.is_none() && left.is_none() {
            return;
        }
        // (One that went out as a picture and stays in sight after all is
        // seen itself again. One that is hidden keeps its picture until
        // it is: `settle_stand_ins`.)
        if !hide {
            self.end_stand_in(handle);
        } else {
            // (Its border, which went out with it, goes with it: left
            // shown, it stayed around nothing on the workspace that came.)
            let _ = self.hide_border(handle);
        }
        // SAFETY: plain calls on a window that may be gone meanwhile; both
        // are posted to the window's own thread, in this order, so that a
        // window is not seen where it belongs before it is hidden.
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return;
            }
            if hide {
                super::reach::show_window_async(hwnd, SW_HIDE);
            }
            if let Some((home, alpha, _)) = left {
                self.animation_scheduler.cancel_window(handle);
                if hide {
                    // (Posted after the hiding, so that it is not seen
                    // where it belongs first. Where it belongs is where
                    // what is seen of it is: the window itself is larger
                    // by its frame that is not seen.)
                    let (left, top, right, bottom) = self.get_window_frame_offsets(hwnd);
                    let outer = (
                        home.x - left,
                        home.y - top,
                        home.width + left + right,
                        home.height + top + bottom,
                    );
                    if super::reach::set_window_pos(
                        hwnd,
                        None,
                        outer.0,
                        outer.1,
                        outer.2,
                        outer.3,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    )
                    .is_ok()
                    {
                        note_posted_move(hwnd, outer);
                    }
                    self.note_placed(handle, home);
                } else {
                    // (It stays in sight: put there now, as every window is.)
                    let _ = self.batch_move_resize(&[(handle, home)]);
                }
                if let Some(alpha) = alpha {
                    let _ = self.set_window_alpha(handle, alpha);
                }
            }
        }
    }

    /// The windows that were kept shown and are not to go out after all,
    /// and those whose way out is over, are hidden.
    fn finish_going_out(&mut self, now: Instant) {
        self.going_out.expected = false;
        let waiting: Vec<WindowHandle> = self
            .going_out
            .waiting
            .iter()
            .map(|(handle, _)| *handle)
            .collect();
        let over: Vec<WindowHandle> = self
            .going_out
            .leaving
            .iter()
            .filter(|(_, (_, _, until))| *until <= now)
            .map(|(handle, _)| *handle)
            .collect();
        for handle in waiting.into_iter().chain(over) {
            self.stop_going_out(handle, true);
        }
    }

    /// Where a window is, or is on its way to: where it belongs while it
    /// goes out with its workspace, where a movement of it ends, else
    /// where it is now.
    ///
    /// What Windows says of a window that is being moved is where it is
    /// in that moment. Taken as the window's place, that was a place
    /// beside the display: a window hidden a second time on its way out
    /// was put back, once hidden, where its way out ended.
    fn settled_rect(&self, handle: WindowHandle) -> Option<crate::dwm::container::Rect> {
        if let Some((home, _, _)) = self.going_out.leaving.get(&handle) {
            return Some(*home);
        }
        if let Some((_, home)) = self
            .going_out
            .waiting
            .iter()
            .find(|(waiting, _)| *waiting == handle)
        {
            return Some(*home);
        }
        self.animation_scheduler
            .rect_target(handle)
            .or_else(|| self.get_window_rect(handle).ok())
    }

    /// Where a window that gevrek has hidden belongs, and is put when it
    /// is shown again (`hidden_places`). `None` for a window that is not
    /// hidden by gevrek, and for one hidden while it was minimised or
    /// maximised (Windows keeps the place of such a window itself).
    ///
    /// Only while it is out of sight, or on its way out: a window that its
    /// program has shown again by itself is where Windows says it is.
    pub fn hidden_place(&self, handle: WindowHandle) -> Option<crate::dwm::container::Rect> {
        let place = self.hidden_places.borrow().get(&handle).copied()?;
        let going = self.going_out.leaving.contains_key(&handle)
            || self
                .going_out
                .waiting
                .iter()
                .any(|(waiting, _)| *waiting == handle);
        // SAFETY: a plain question about a window that may be gone.
        (going || !unsafe { IsWindowVisible(handle.to_hwnd()) }.as_bool()).then_some(place)
    }

    /// gevrek puts a window somewhere: if it is hidden, that is where it
    /// belongs from now on.
    fn note_placed(&self, handle: WindowHandle, place: crate::dwm::container::Rect) {
        if let Some(kept) = self.hidden_places.borrow_mut().get_mut(&handle) {
            *kept = place;
        }
    }

    /// A window that is shown again is where it belongs. While it was
    /// hidden it was moved by messages its program answered when it got to
    /// them, in an order that is not always the order they were sent in:
    /// a window of a program that was busy for a moment (Unreal Editor is,
    /// often) was left where its workspace's way in began, beside the
    /// display, and was shown there: on a workspace whose windows float,
    /// where no layout puts it right, it was gone.
    fn put_back_where_it_belongs(&mut self, handle: WindowHandle) {
        let Some(place) = self.hidden_places.borrow_mut().remove(&handle) else {
            return;
        };
        let hwnd = handle.to_hwnd();
        // SAFETY: plain questions about a window that may be gone.
        let placed = unsafe {
            IsWindow(hwnd).as_bool() && !IsIconic(hwnd).as_bool() && !IsZoomed(hwnd).as_bool()
        };
        // (One that is on its way somewhere arrives there by itself, and
        // one that comes in as a picture is in its place already.)
        if !placed
            || self.animation_scheduler.rect_target(handle).is_some()
            || self.stand_ins.has(handle)
            || self.is_window_fullscreen(handle)
        {
            return;
        }
        if self.get_window_rect(handle).is_ok_and(|now| now != place) {
            log::debug!("Window {} is shown again where it belongs: {place:?}", handle.0);
            if let Err(error) = self.batch_move_resize(&[(handle, place)]) {
                log::debug!("window {} was not put back where it belongs: {error:#}", handle.0);
            }
            let _ = self.update_border_position_explicit(
                handle,
                place.x,
                place.y,
                place.width,
                place.height,
            );
        }
    }

    /// Whether gevrek has hidden this window, or has asked for it to be
    /// and Windows has not done so yet. A window is hidden by a message it
    /// answers when it gets to it, and until then Windows still calls it
    /// visible. A border that was placed in that moment was shown again,
    /// and stayed around nothing once the window was gone: nothing hid it
    /// a second time. So a window that is to be out of sight has no
    /// border, whatever Windows says of it meanwhile. One that goes out
    /// with its workspace keeps its border on the way; it is hidden with
    /// the window (`stop_going_out`).
    fn kept_out_of_sight(&self, handle: WindowHandle) -> bool {
        super::hidden_journal::is_hidden(handle)
            && !self.going_out.leaving.contains_key(&handle)
            && !self
                .going_out
                .waiting
                .iter()
                .any(|(waiting, _)| *waiting == handle)
    }

    /// Hide a window (for workspace switching)
    pub fn hide_window(&mut self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // A workspace's window that may go out first stays shown
                // for now (`workspace_gives_way`); all else about hiding
                // it is done at once.
                // (Not one that is minimized: where it "is" then is
                // where its mark is kept, far off every display and as
                // small as a title. Put back there when it had gone out,
                // that became where the window is when it is brought
                // back: very small, and out of sight.)
                let placed = !IsZoomed(hwnd).as_bool() && !IsIconic(hwnd).as_bool();
                let kept = self.going_out.expected && self.is_window_visible(handle) && placed;
                // (Where it belongs, asked before anything about its way
                // out is undone: `settled_rect`.)
                let settled = placed.then(|| self.settled_rect(handle)).flatten();
                // Where it is to be when it is shown again. One that is
                // hidden already keeps the place it has: what Windows says
                // of a hidden window's place is not to be relied on.
                if let Some(place) = settled {
                    if self.hidden_place(handle).is_none() {
                        self.hidden_places.borrow_mut().insert(handle, place);
                    }
                }
                let home = kept.then_some(settled).flatten();
                match home {
                    Some(home) => {
                        self.stop_going_out(handle, false);
                        self.going_out.waiting.push((handle, home));
                    }
                    None => {
                        self.stop_going_out(handle, false);
                        // Async: a busy app must not stall a workspace
                        // switch. Posted show/hide requests are applied in
                        // order.
                        super::reach::show_window_async(hwnd, SW_HIDE);
                    }
                }
                super::hidden_journal::record_hidden(handle, false);
                self.hide_owned_windows(handle);

                // Also hide border to prevent ghost windows
                self.hide_border(handle)?;
                // A stackbar keyed by this window (its container's focused
                // tab) must disappear with it, e.g. on a workspace switch.
                self.hide_stackbar(handle);

                log::trace!("Hid window: {:?}", handle);
            }
        }
        Ok(())
    }

    /// Show a window (for workspace switching)
    pub fn show_window(&mut self, handle: WindowHandle) -> Result<()> {
        // (One that was on its way out is where it belongs again.)
        self.stop_going_out(handle, false);
        self.put_back_where_it_belongs(handle);
        // Forget the window even when it is gone, so the journal never keeps
        // dead handles. A window hidden by a cloak path (e.g. the graph's
        // workspace nodes) needs an uncloak; SW_SHOW alone leaves it invisible.
        let journaled = super::hidden_journal::record_shown(handle);
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                if journaled.is_some_and(|entry| entry.cloaked) {
                    let uncloak: i32 = 0;
                    let _ = super::reach::dwm_set_window_attribute(
                        hwnd,
                        DWMWINDOWATTRIBUTE(13), // DWMWA_CLOAK
                        &uncloak as *const _ as *const _,
                        std::mem::size_of::<i32>() as u32,
                    );
                }
                // SW_SHOWNA shows the window in its current state;
                // SW_SHOWNOACTIVATE would restore a maximized window.
                super::reach::show_window_async(hwnd, SW_SHOWNA);

                // Also show border if it exists
                self.show_border(handle)?;
                self.show_stackbar(handle);

                log::trace!("Showed window: {:?}", handle);
            }
        }
        // (After the window itself: what belongs to it lies above it.)
        self.show_owned_windows(handle);
        Ok(())
    }

    /// Hide what belongs to a window that gevrek hides: the dialogs it has
    /// open, a palette of it. Windows keeps such a window above its owner
    /// and minimises it with it, and leaves it on the screen when the
    /// owner is hidden: a "Save as" would stay behind on a workspace its
    /// program is not on. They are no windows of gevrek's (`window_kind`),
    /// so they are remembered here, by the window they went with.
    ///
    /// And what a program shows beside its windows that belongs to none
    /// of them: Unreal Editor's notifications ("Compiling shaders", up for
    /// minutes), its menus and the small window it keeps from its start
    /// belong to no window, and stayed on the screen over the workspace
    /// that came. They go with the last of the program's windows that is
    /// in sight (`companions_on_screen`).
    fn hide_owned_windows(&self, owner: WindowHandle) {
        let mut owned = owned_windows_on_screen(owner.to_hwnd());
        owned.extend(companions_on_screen(owner.to_hwnd(), |hwnd| {
            let handle = WindowHandle::from_hwnd(hwnd);
            self.windows.contains_key(&handle) && !super::hidden_journal::is_hidden(handle)
        }));
        if owned.is_empty() {
            return;
        }
        let mut hidden = self.hidden_with_owner.borrow_mut();
        let list = hidden.entry(owner).or_default();
        for hwnd in owned {
            let handle = WindowHandle::from_hwnd(hwnd);
            // (Posted to the window's own thread; a window that is gone
            // meanwhile takes no harm.)
            super::reach::show_window_async(hwnd, SW_HIDE);
            // (In the journal as every window gevrek hides: a gevrek that
            // ends, in whatever way, shows it again.)
            super::hidden_journal::record_hidden(handle, false);
            if !list.contains(&handle) {
                list.push(handle);
            }
        }
        log::trace!("Hid {} window(s) that belong to {:?}", list.len(), owner);
    }

    /// A window that is no window of gevrek's was shown just now: where
    /// it belongs to a window gevrek has hidden with its workspace, or is
    /// something a program shows beside its windows while all of them are
    /// hidden, it is hidden too, and shown again with that window
    /// (`show_owned_windows`). `hide_owned_windows` sees to what is there
    /// at the moment a window is hidden; this is for what comes after: a
    /// dialog a program opens while the person works on another workspace
    /// (it came up there, with the keyboard), a notification it puts up
    /// meanwhile. Returns the window it was hidden with.
    ///
    /// In the journal as every window gevrek hides, so that a gevrek that
    /// ends, in whatever way, shows it again.
    pub fn hide_late_arrival(&self, handle: WindowHandle) -> Option<WindowHandle> {
        let hwnd = handle.to_hwnd();
        // SAFETY: plain questions about a window that may be gone.
        let in_sight = unsafe { IsWindow(hwnd).as_bool() && IsWindowVisible(hwnd).as_bool() };
        if !in_sight {
            return None;
        }
        let class = read_window_class(hwnd);
        let owners: Vec<(WindowHandle, bool)> = owners_of(hwnd)
            .into_iter()
            .map(|owner| {
                let owner = WindowHandle::from_hwnd(owner);
                (owner, super::hidden_journal::is_hidden(owner))
            })
            .collect();
        let kind = crate::window_kind::of_window(hwnd.0 as isize);
        // (Asked only for what could go with something: most windows that
        // are shown belong to no window, and are windows of gevrek's.)
        let arrival = LateArrival {
            already_hidden: super::hidden_journal::is_hidden(handle),
            passes_in_a_moment: class.starts_with("Gevrek_")
                || crate::window_kind::is_a_popups_class(&class),
            gevreks_own: owners.iter().any(|(_, hidden)| *hidden)
                && !self.should_ignore_window(handle),
            kind,
            sized: {
                let mut rect = RECT::default();
                // SAFETY: a plain question with a valid out pointer.
                unsafe {
                    GetWindowRect(hwnd, &mut rect).is_ok()
                        && rect.right > rect.left
                        && rect.bottom > rect.top
                }
            },
        };
        let with = arrival.goes_with(&owners, || {
            let mut program = Program::default();
            for other in thread_windows(hwnd) {
                if other == hwnd {
                    continue;
                }
                let other = WindowHandle::from_hwnd(other);
                if !self.windows.contains_key(&other) {
                    continue;
                }
                if super::hidden_journal::is_hidden(other) {
                    program.hidden.get_or_insert(other);
                } else {
                    program.in_sight = true;
                }
            }
            program
        })?;
        let mut hidden = self.hidden_with_owner.borrow_mut();
        // What it goes with may itself have been hidden with a window (a
        // dialog's own dialog): it comes back with that one.
        let key = hidden
            .iter()
            .find(|(_, list)| list.contains(&with))
            .map_or(with, |(key, _)| *key);
        super::reach::show_window_async(hwnd, SW_HIDE);
        super::hidden_journal::record_hidden(handle, false);
        let list = hidden.entry(key).or_default();
        if !list.contains(&handle) {
            list.push(handle);
        }
        log::debug!("{handle:?} was shown while {key:?} is hidden: it is hidden with it");
        Some(key)
    }

    /// Show again what was hidden with a window (`hide_owned_windows`).
    /// Also when that window is gone: Windows ends a window's dialogs with
    /// it, and one that is still there is a window of its own again.
    fn show_owned_windows(&self, owner: WindowHandle) {
        let Some(owned) = self.hidden_with_owner.borrow_mut().remove(&owner) else {
            return;
        };
        for handle in owned {
            super::hidden_journal::record_shown(handle);
            let hwnd = handle.to_hwnd();
            // SAFETY: plain calls; a window that closed meanwhile is skipped.
            unsafe {
                if IsWindow(hwnd).as_bool() {
                    super::reach::show_window_async(hwnd, SW_SHOWNA);
                }
            }
        }
    }

    /// Bring a minimized window back, without giving it the keyboard:
    /// whoever asks for it gives the keyboard to the window that is to
    /// have it.
    ///
    /// `SW_RESTORE` gives the keyboard to the window it brings back. A
    /// workspace whose windows had minimized themselves while it was away
    /// (Windows Terminal's do, when they are hidden) had them brought
    /// back one after the other as it was shown: each took the keyboard
    /// in its turn, and the border of the focused window went from one to
    /// the next before it came to the window that was to have it.
    pub fn restore_window(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() && IsIconic(hwnd).as_bool() {
                let mut placement = WINDOWPLACEMENT {
                    length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                    ..Default::default()
                };
                let to_maximized = GetWindowPlacement(hwnd, &mut placement).is_ok()
                    && placement.flags.contains(WPF_RESTORETOMAXIMIZED);
                show_window_nonblocking(hwnd, restore_command(to_maximized));
                if !to_maximized {
                    // (Minimized, it went below every window, and brought
                    // back without the keyboard it stays there: it is
                    // put above, as it was when it took the keyboard.)
                    let _ = super::reach::set_window_pos(
                        hwnd,
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE),
                    );
                }
                log::debug!("Restored minimized window: {:?}", handle);
            }
        }
        Ok(())
    }

    /// Check if a window is maximized
    pub fn is_window_maximized(&self, handle: WindowHandle) -> bool {
        unsafe { IsZoomed(handle.to_hwnd()).as_bool() }
    }

    /// Restore a maximized window to its normal (tiled) state
    /// Uses SW_RESTORE which properly restores from maximized state
    pub fn unmaximize_window(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() && IsZoomed(hwnd).as_bool() {
                show_window_nonblocking(hwnd, SW_RESTORE);
                log::debug!("Unmaximized window: {:?}", handle);
            }
        }
        Ok(())
    }

    /// Remove minimize and maximize buttons from a window's title bar
    /// This is used by tiling WMs to prevent users from minimizing/maximizing
    pub fn remove_min_max_buttons(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            // A style change sends the window a message and waits for it.
            if !IsWindow(hwnd).as_bool() || is_hung(hwnd) {
                return Ok(());
            }

            // Get current window style
            let style = GetWindowLongW(hwnd, GWL_STYLE);

            // Remove minimize and maximize box styles
            const WS_MINIMIZEBOX: i32 = 0x00020000;
            const WS_MAXIMIZEBOX: i32 = 0x00010000;
            const WS_THICKFRAME: i32 = 0x00040000;
            // (Not from a window without a frame to resize it by, nor
            // from one without a title bar: what kind of window it is is
            // read from these very boxes there (`window_kind`), and it
            // would be another kind without them. A window without a
            // title bar shows no boxes to take away in any case.)
            const WS_CAPTION: i32 = 0x00C0_0000;
            if style & WS_THICKFRAME == 0 || style & WS_CAPTION != WS_CAPTION {
                return Ok(());
            }
            let new_style = style & !(WS_MINIMIZEBOX | WS_MAXIMIZEBOX);

            if new_style != style {
                *self
                    .caption_buttons_removed
                    .borrow_mut()
                    .entry(handle)
                    .or_insert(0) |= style & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
                super::reach::set_window_long(hwnd, GWL_STYLE, new_style);
                // Force window to redraw its non-client area (title bar)
                super::reach::set_window_pos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(
                        hwnd,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                    ),
                )?;
                log::trace!("Removed min/max buttons from window: {:?}", handle);
            }
        }
        Ok(())
    }

    /// Restore minimize and maximize buttons to a window's title bar
    /// Used when a window is unmanaged (e.g., becomes floating)
    pub fn restore_min_max_buttons(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() || is_hung(hwnd) {
                return Ok(());
            }

            // Get current window style
            let style = GetWindowLongW(hwnd, GWL_STYLE);

            // The boxes gevrek took away, and no other: a window that
            // never had a box to maximise it by does not get one.
            let Some(removed) = self.caption_buttons_removed.borrow_mut().remove(&handle) else {
                return Ok(());
            };
            let new_style = style | removed;

            if new_style != style {
                super::reach::set_window_long(hwnd, GWL_STYLE, new_style);
                // Force window to redraw its non-client area (title bar)
                super::reach::set_window_pos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(
                        hwnd,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                    ),
                )?;
                log::trace!("Restored min/max buttons to window: {:?}", handle);
            }
        }
        Ok(())
    }

    /// Restore all minimized windows - useful for tiling WM startup
    /// This works across ALL monitors (EnumWindows is system-wide)
    pub fn restore_all_minimized(&self) -> Result<usize> {
        let mut restored_count = 0;
        let total_windows = self.windows.len();

        log::info!(
            "Checking {} tracked windows for minimized state...",
            total_windows
        );

        for window in self.windows.values() {
            if self.is_window_minimized(window.handle) {
                log::info!(
                    "  Restoring minimized window: '{}' ({:?})",
                    window.title,
                    window.handle
                );
                self.restore_window(window.handle)?;
                restored_count += 1;
            }
        }

        if restored_count > 0 {
            log::info!(
                "✅ Restored {} minimized windows (out of {} total)",
                restored_count,
                total_windows
            );
        } else {
            log::info!("No minimized windows found to restore");
        }
        Ok(restored_count)
    }

    pub fn set_window_title(&self, handle: WindowHandle, title: &str) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
                SetWindowTextW(hwnd, windows::core::PCWSTR(title_wide.as_ptr()))?;
            }
        }
        Ok(())
    }

    /// Restore native composition when Gevrek no longer needs per-window alpha.
    ///
    /// `WS_EX_LAYERED` is intentionally removed only when Gevrek added it.
    /// Application-owned layered windows retain their original rendering path.
    fn restore_native_window_alpha(&mut self, handle: WindowHandle) -> Result<bool> {
        if is_hung(handle.to_hwnd()) {
            return Ok(false);
        }
        if !self.gevrek_layered_windows.remove(&handle) {
            return Ok(false);
        }

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                GetWindowLongW, GWL_EXSTYLE, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_LAYERED,
            };

            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                if (ex_style & WS_EX_LAYERED.0 as i32) != 0 {
                    super::reach::set_window_long(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED.0 as i32));
                    super::reach::set_window_pos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        nonblocking_swp(
                            hwnd,
                            SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOZORDER
                                | SWP_NOACTIVATE
                                | SWP_FRAMECHANGED,
                        ),
                    )?;
                }
            }
        }
        Ok(true)
    }

    /// Remove every layered style owned by Gevrek.
    ///
    /// This is part of graceful shutdown so applications return to their native
    /// DirectComposition path even if the WM exits during a fade.
    /// Give every window back what gevrek took from its frame: the boxes
    /// of its title bar, and the title bar itself. (When gevrek ends:
    /// they would else stay away until the window's program is started
    /// again.)
    pub fn restore_all_window_frames(&mut self) {
        let without_buttons: Vec<WindowHandle> =
            self.caption_buttons_removed.borrow().keys().copied().collect();
        for handle in without_buttons {
            let _ = self.restore_min_max_buttons(handle);
        }
        let without_title: Vec<WindowHandle> = self.title_bar_hidden.keys().copied().collect();
        for handle in without_title {
            let _ = self.set_title_bar_visible(handle, true);
        }
    }

    pub fn restore_all_window_alpha_styles(&mut self) -> usize {
        for handle in self.stand_ins.handles() {
            self.end_stand_in(handle);
        }
        self.stand_ins.sweep();
        let handles = self
            .gevrek_layered_windows
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let mut restored = 0;
        for handle in handles {
            match self.restore_native_window_alpha(handle) {
                Ok(true) => restored += 1,
                Ok(false) => {}
                Err(error) => {
                    log::warn!(
                        "Failed to restore native composition for {:?}: {}",
                        handle,
                        error
                    );
                }
            }
        }
        restored
    }

    pub fn set_window_alpha(&mut self, handle: WindowHandle, alpha: u8) -> Result<()> {
        // A window that is coming in stays unseen until it is there: the
        // strength asked for is its picture's meanwhile, and its own after.
        if self.stand_ins.has(handle) {
            let part = self.seen_part(handle.to_hwnd()).unwrap_or_default();
            self.stand_ins.set_alpha(handle, alpha, part);
            self.window_alphas.insert(handle, alpha);
            return Ok(());
        }
        self.apply_window_alpha(handle, alpha)
    }

    /// `set_window_alpha` without regard to a stand-in.
    fn apply_window_alpha(&mut self, handle: WindowHandle, alpha: u8) -> Result<()> {
        // Changing the layered style sends the window a message and waits.
        // Not recorded, so the next focus-effects pass retries it.
        if is_hung(handle.to_hwnd()) {
            return Ok(());
        }
        if window_alpha_action(
            alpha,
            self.gevrek_layered_windows.contains(&handle),
            self.gevrek_layered_windows.contains(&handle),
        ) == WindowAlphaAction::RestoreNative
        {
            let _ = self.restore_native_window_alpha(handle)?;
            self.window_alphas.insert(handle, alpha);
            return Ok(());
        }

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                GetWindowLongW, GWL_EXSTYLE,
                WS_EX_LAYERED,
            };

            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

                let layered = (ex_style & WS_EX_LAYERED.0 as i32) != 0;
                let gevrek_owned = self.gevrek_layered_windows.contains(&handle);
                if window_alpha_action(alpha, layered, gevrek_owned)
                    == WindowAlphaAction::SkipApplicationOwnedLayered
                {
                    // `UpdateLayeredWindow` and `SetLayeredWindowAttributes` are
                    // mutually disruptive for application-owned layered
                    // windows. Keep the application's renderer intact.
                    log::warn!(
                        "Skipping opacity {} for application-owned layered window {:?}",
                        alpha,
                        handle
                    );
                    self.window_alphas.insert(handle, 255);
                    return Ok(());
                }

                if (ex_style & WS_EX_LAYERED.0 as i32) == 0 {
                    super::reach::set_window_long(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED.0 as i32);
                    self.gevrek_layered_windows.insert(handle);
                }

                if let Err(error) = super::reach::set_layered_alpha(hwnd, alpha) {
                    let _ = self.restore_native_window_alpha(handle);
                    return Err(error.into());
                }
            }
        }
        self.window_alphas.insert(handle, alpha);
        Ok(())
    }

    pub fn window_alpha(&self, handle: WindowHandle) -> u8 {
        self.window_alphas.get(&handle).copied().unwrap_or(255)
    }

    /// Configure the Windows compositor's native non-client rendering policy.
    ///
    /// DWM owns the actual shadow shape and blur on Windows. Gevrek can
    /// request native rendering, disable it, or return control to the window's
    /// own style without pretending to provide compositor-only controls.
    pub fn set_window_shadow_policy(
        &mut self,
        handle: WindowHandle,
        policy: ShadowPolicy,
    ) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                self.window_shadows.remove(&handle);
                return Ok(false);
            }

            // DWMNCRENDERINGPOLICY: use-window-style=0, disabled=1, enabled=2.
            // DWMWA_NCRENDERING_POLICY is attribute 2 on every supported DWM.
            let value: i32 = match policy {
                ShadowPolicy::Default => 0,
                ShadowPolicy::Disabled => 1,
                ShadowPolicy::Enabled => 2,
            };
            let applied = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(2),
                &value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            )
            .is_ok();
            if applied {
                self.window_shadows.insert(handle, policy);
            }
            Ok(applied)
        }
    }

    pub fn window_shadow_policy(&self, handle: WindowHandle) -> ShadowPolicy {
        self.window_shadows
            .get(&handle)
            .copied()
            .unwrap_or_default()
    }

    /// Put a window below all of `above` in the order the windows lie in,
    /// leaving the keyboard where it is. (For the window that has the
    /// keyboard: Windows lets no other program put a window above that
    /// one, but it lets that one be put below another.) Its border goes
    /// with it. Nothing happens when none of `above` is on the screen.
    ///
    /// Never below the desktop: a minimized window is still "visible" to
    /// Windows and lies at the very bottom, under the surface the zones
    /// are drawn on and under Explorer's own desktop. A window put below
    /// that one would be covered by the desktop. So minimized windows are
    /// not counted, and the search ends where the desktop begins.
    pub fn put_below(&mut self, handle: WindowHandle, above: &[WindowHandle]) -> Result<()> {
        // SAFETY: plain queries of the windows' order, and one change of it.
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }
            // The lowest of them, going down from the top.
            let mut lowest = None;
            let mut next = GetTopWindow(None).unwrap_or_default();
            let mut looked_at = 0;
            while !next.is_invalid() && looked_at < 4096 {
                if is_desktop_band(next) {
                    break;
                }
                if above.iter().any(|window| window.to_hwnd() == next)
                    && IsWindowVisible(next).as_bool()
                    && !IsIconic(next).as_bool()
                {
                    lowest = Some(next);
                }
                next = GetWindow(next, GW_HWNDNEXT).unwrap_or_default();
                looked_at += 1;
            }
            let Some(lowest) = lowest else {
                return Ok(());
            };
            super::reach::set_window_pos(
                hwnd,
                Some(lowest),
                0,
                0,
                0,
                0,
                nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE),
            )?;
        }
        // (Its border lies behind it, wherever it is.)
        let _ = self.update_border_position(handle);
        Ok(())
    }

    pub fn bring_to_front(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // Use HWND_TOP to bring window to top of Z-order
                super::reach::set_window_pos(
                    hwnd,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE),
                )?;
                log::trace!("Brought window to front");
            }
        }
        Ok(())
    }

    pub fn focus_window(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // A window that waits for a dialog of its own to be answered
                // takes no input (Windows has it disabled): the keyboard
                // goes to the dialog, as it does when the window's button
                // in the taskbar is pressed. Given to the window itself, it
                // would be with a window that hears nothing.
                let hwnd = window_to_activate(hwnd);
                // First bring to front, then give it focus
                super::reach::set_window_pos(
                    hwnd,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE),
                )?;
                // Give the window keyboard focus
                take_foreground(hwnd);
                log::trace!("Focused window");
            }
        }
        Ok(())
    }

    /// Warp the mouse cursor to the center of a window. Used by the
    /// mouse-follows-focus option after dispatch-driven focus changes.
    /// Failures are intentionally silent: warping is a convenience, never
    /// a reason to fail a focus command.
    pub fn warp_cursor_to_window_center(&self, handle: WindowHandle) {
        let Ok(rect) = self.get_window_rect(handle) else {
            return;
        };
        unsafe {
            let _ = SetCursorPos(
                rect.x + rect.width.div_euclid(2),
                rect.y + rect.height.div_euclid(2),
            );
        }
    }

    pub fn send_to_back(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // Use HWND_BOTTOM to send window to bottom of Z-order
                super::reach::set_window_pos(
                    hwnd,
                    Some(HWND_BOTTOM),
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE),
                )?;
                log::trace!("Sent window to back");
            }
        }
        Ok(())
    }

    pub fn set_always_on_top(&self, handle: WindowHandle, enabled: bool) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                // Use HWND_TOPMOST to make window always on top, or HWND_NOTOPMOST to remove it
                let z_order = if enabled {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                };
                super::reach::set_window_pos(
                    hwnd,
                    Some(z_order),
                    0,
                    0,
                    0,
                    0,
                    nonblocking_swp(hwnd, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE),
                )?;
                log::debug!("Set always on top: {} for window", enabled);
            }
        }
        Ok(())
    }

    // ==================== Windows 11 DWM Features ====================

    /// Cloak a window (Windows 11 feature for seamless hiding)
    ///
    /// Unlike ShowWindow(SW_HIDE), cloaking makes the window invisible
    /// without triggering minimize animations or focus changes.
    /// Perfect for workspace switching.
    pub fn cloak_window(&self, handle: WindowHandle, cloak: bool) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // DWMWA_CLOAK = 13
            let cloak_value: i32 = if cloak { 1 } else { 0 };
            let result = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(13), // DWMWA_CLOAK
                &cloak_value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );

            if result.is_ok() {
                log::trace!("Window {} cloaked: {}", handle.0, cloak);
            } else {
                // Fallback to ShowWindow if cloaking not supported
                if cloak {
                    super::reach::show_window_async(hwnd, SW_HIDE);
                } else {
                    super::reach::show_window_async(hwnd, SW_SHOWNA);
                }
            }
            if cloak {
                super::hidden_journal::record_hidden(handle, result.is_ok());
                self.hide_owned_windows(handle);
            } else {
                super::hidden_journal::record_shown(handle);
                self.show_owned_windows(handle);
            }

            // Also handle border visibility
            if cloak {
                self.hide_border(handle)?;
            } else {
                self.show_border(handle)?;
            }
        }
        Ok(())
    }

    /// Set window border color (Windows 11)
    ///
    /// Uses DWMWA_BORDER_COLOR to set the native window border color.
    /// Pass None to reset to default.
    pub fn set_window_border_color(
        &mut self,
        handle: WindowHandle,
        color: Option<(u8, u8, u8)>,
    ) -> Result<()> {
        let color = color
            .map(|(r, g, b)| RuleColor::Rgb { r, g, b })
            .unwrap_or(RuleColor::Default);
        self.set_window_border_rule_color(handle, color)
    }

    pub fn set_window_border_rule_color(
        &mut self,
        handle: WindowHandle,
        color: RuleColor,
    ) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // DWMWA_BORDER_COLOR = 34
            let color_value: u32 = match color {
                RuleColor::Default => 0xFFFFFFFF,
                RuleColor::Hidden => 0xFFFFFFFE,
                RuleColor::Rgb { r, g, b } => (b as u32) << 16 | (g as u32) << 8 | (r as u32),
            };

            let _ = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(34), // DWMWA_BORDER_COLOR
                &color_value as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            log::trace!("Set native border color for window {}", handle.0);
        }
        self.window_border_colors.insert(handle, color);
        Ok(())
    }

    pub fn window_border_color(&self, handle: WindowHandle) -> RuleColor {
        self.window_border_colors
            .get(&handle)
            .copied()
            .unwrap_or(RuleColor::Default)
    }

    /// Set window caption/titlebar color (Windows 11)
    pub fn set_window_caption_color(
        &mut self,
        handle: WindowHandle,
        color: Option<(u8, u8, u8)>,
    ) -> Result<()> {
        let color = color
            .map(|(r, g, b)| RuleColor::Rgb { r, g, b })
            .unwrap_or(RuleColor::Default);
        self.set_window_caption_rule_color(handle, color)
    }

    pub fn set_window_caption_rule_color(
        &mut self,
        handle: WindowHandle,
        color: RuleColor,
    ) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // DWMWA_CAPTION_COLOR = 35
            let color_value: u32 = match color {
                RuleColor::Default => 0xFFFFFFFF,
                RuleColor::Hidden => 0xFFFFFFFE,
                RuleColor::Rgb { r, g, b } => (b as u32) << 16 | (g as u32) << 8 | (r as u32),
            };

            let _ = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(35), // DWMWA_CAPTION_COLOR
                &color_value as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            log::trace!("Set caption color for window {}", handle.0);
        }
        self.window_caption_colors.insert(handle, color);
        Ok(())
    }

    pub fn window_caption_color(&self, handle: WindowHandle) -> RuleColor {
        self.window_caption_colors
            .get(&handle)
            .copied()
            .unwrap_or(RuleColor::Default)
    }

    /// Set system backdrop type (Windows 11 - Mica, Acrylic, etc.)
    ///
    /// Backdrop types:
    /// - 0: Auto (system decides)
    /// - 1: None
    /// - 2: Mica (main backdrop)
    /// - 3: Acrylic (blur)
    /// - 4: Mica Alt (tabbed variant)
    pub fn set_system_backdrop(
        &mut self,
        handle: WindowHandle,
        backdrop_type: SystemBackdrop,
    ) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // DWMWA_SYSTEMBACKDROP_TYPE = 38
            let backdrop_value: i32 = backdrop_type as i32;
            let result = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(38), // DWMWA_SYSTEMBACKDROP_TYPE
                &backdrop_value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );

            if result.is_ok() {
                log::trace!(
                    "Set system backdrop {:?} for window {}",
                    backdrop_type,
                    handle.0
                );
            }
        }
        self.window_backdrops.insert(handle, backdrop_type);
        Ok(())
    }

    pub fn window_backdrop(&self, handle: WindowHandle) -> SystemBackdrop {
        self.window_backdrops
            .get(&handle)
            .copied()
            .unwrap_or(SystemBackdrop::Auto)
    }

    /// Set window corner preference (Windows 11)
    ///
    /// Corner types:
    /// - 0: Default (let system decide)
    /// - 1: Do not round
    /// - 2: Round (default Windows 11 style)
    /// - 3: Round small (tighter radius)
    pub fn set_window_corner_preference(
        &mut self,
        handle: WindowHandle,
        corner: WindowCorner,
    ) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // DWMWA_WINDOW_CORNER_PREFERENCE = 33
            let corner_value: i32 = corner as i32;
            let _ = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(33), // DWMWA_WINDOW_CORNER_PREFERENCE
                &corner_value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );

            log::trace!("Set corner preference {:?} for window {}", corner, handle.0);
        }
        self.window_corners.insert(handle, corner);
        Ok(())
    }

    pub fn window_corner_preference(&self, handle: WindowHandle) -> WindowCorner {
        self.window_corners
            .get(&handle)
            .copied()
            .unwrap_or(WindowCorner::Default)
    }

    // ==================== Stand-ins ====================

    /// What is seen of a window (it without its invisible frame), in the
    /// window's own coordinates: the part of it a picture of it shows.
    fn seen_part(&self, hwnd: HWND) -> Option<RECT> {
        let mut outer = RECT::default();
        // SAFETY: a plain question about a window that may be gone.
        unsafe { GetWindowRect(hwnd, &mut outer) }.ok()?;
        let (left, top, right, bottom) = self.get_window_frame_offsets(hwnd);
        let width = outer.right - outer.left;
        let height = outer.bottom - outer.top;
        (width - left - right > 0 && height - top - bottom > 0).then_some(RECT {
            left,
            top,
            right: width - right,
            bottom: height - bottom,
        })
    }

    /// Slide a window with its workspace as a picture of itself, from
    /// `from` to `to`: the window is unseen meanwhile, at `stays_at`
    /// (where it is, if none is given), and only its picture moves.
    /// `false` where that cannot be done: the caller moves the window
    /// itself, as before.
    fn slide_as_picture(
        &mut self,
        handle: WindowHandle,
        from: crate::dwm::container::Rect,
        to: crate::dwm::container::Rect,
        stays_at: Option<crate::dwm::container::Rect>,
        now: Instant,
    ) -> bool {
        self.end_stand_in(handle);
        let hwnd = handle.to_hwnd();
        // SAFETY: plain questions about a window that may be gone.
        let (exists, ex_style) =
            unsafe { (IsWindow(hwnd).as_bool(), GetWindowLongW(hwnd, GWL_EXSTYLE)) };
        if !exists || is_hung(hwnd) {
            return false;
        }
        // A window its program makes see-through itself cannot be made
        // unseen without spoiling it (`set_window_alpha`).
        let layered = ex_style & WS_EX_LAYERED.0 as i32 != 0;
        if layered && !self.gevrek_layered_windows.contains(&handle) {
            return false;
        }
        let alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
        if alpha == 0 {
            return false;
        }
        if let Some(at) = stays_at {
            if self.batch_move_resize(&[(handle, at)]).is_err() {
                return false;
            }
        }
        let Some(part) = self.seen_part(hwnd) else {
            return false;
        };
        // The picture first, then the window unseen: what is seen never
        // shows nothing where the window was.
        if !self.stand_ins.open(handle, hwnd, alpha, from, part) {
            return false;
        }
        if self.apply_window_alpha(handle, 0).is_err() {
            self.stand_ins.take(handle);
            return false;
        }
        self.window_alphas.insert(handle, alpha);
        let _ = self.update_border_position_explicit(handle, from.x, from.y, from.width, from.height);
        if !self
            .animation_scheduler
            .schedule_rect(handle, from, to, TransitionScope::Workspace, now)
        {
            self.end_stand_in(handle);
            return false;
        }
        true
    }

    /// The shadow and glow of a window are left out while it slides with
    /// its workspace, where the configuration says so.
    fn rest_effects(&mut self, handle: WindowHandle) {
        if !self.animation_scheduler.profile().workspace_slides_without_shadows {
            return;
        }
        self.effects_resting.insert(handle);
        self.auras.borrow().set_visible(handle.to_hwnd(), false);
    }

    /// The windows that have arrived (or are gone) have their shadow and
    /// glow again.
    fn settle_resting_effects(&mut self) {
        if self.effects_resting.is_empty() {
            return;
        }
        let arrived: Vec<WindowHandle> = self
            .effects_resting
            .iter()
            .copied()
            .filter(|handle| {
                self.animation_scheduler.rect_target(*handle).is_none()
                    && !self.going_out.leaving.contains_key(handle)
                    && !self.overview_holds(*handle)
            })
            .collect();
        for handle in arrived {
            self.effects_resting.remove(&handle);
            let _ = self.update_border_position(handle);
        }
    }

    /// Bring a window in from `start` to `target` as a picture of itself
    /// (`stand_in`): the window is put at `target` at once, unseen, and
    /// only the picture moves. `false` where that cannot be done (the
    /// caller moves the window itself, as before).
    fn enter_by_stand_in(
        &mut self,
        handle: WindowHandle,
        start: crate::dwm::container::Rect,
        target: crate::dwm::container::Rect,
        duration_ms: u32,
        now: Instant,
    ) -> bool {
        self.end_stand_in(handle);
        let hwnd = handle.to_hwnd();
        // SAFETY: plain questions about a window that may be gone.
        let (exists, ex_style) =
            unsafe { (IsWindow(hwnd).as_bool(), GetWindowLongW(hwnd, GWL_EXSTYLE)) };
        if !exists || is_hung(hwnd) || !self.animation_scheduler.moves(TransitionScope::Explicit)
        {
            return false;
        }
        // A window its program makes see-through itself cannot be made
        // unseen without spoiling it (`set_window_alpha`).
        let layered = ex_style & WS_EX_LAYERED.0 as i32 != 0;
        if layered && !self.gevrek_layered_windows.contains(&handle) {
            return false;
        }
        let alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
        if alpha == 0 {
            return false;
        }
        let Some(part) = self.seen_part(hwnd) else {
            return false;
        };
        // The picture first, then the window unseen, then the window in
        // its place: what is seen never shows the window jump, nor nothing.
        if !self.stand_ins.open(handle, hwnd, alpha, start, part) {
            return false;
        }
        if self.apply_window_alpha(handle, 0).is_err() {
            self.stand_ins.take(handle);
            return false;
        }
        self.window_alphas.insert(handle, alpha);
        if self.batch_move_resize(&[(handle, target)]).is_err() {
            self.end_stand_in(handle);
            return false;
        }
        if let Some(part) = self.seen_part(hwnd) {
            self.stand_ins.place(handle, start, part);
        }
        let _ = self.update_border_position_explicit(
            handle,
            start.x,
            start.y,
            start.width,
            start.height,
        );
        if !self
            .animation_scheduler
            .schedule_explicit_rect(handle, start, target, duration_ms, now)
        {
            self.end_stand_in(handle);
            return false;
        }
        log::debug!("Window {} comes in as a picture of itself", handle.0);
        true
    }

    /// A window that came in as a picture of itself is seen again, where
    /// it is (where its movement was to end), and its picture goes.
    fn end_stand_in(&mut self, handle: WindowHandle) {
        self.pictures_going.remove(&handle);
        let Some(alpha) = self.stand_ins.take(handle) else {
            return;
        };
        // (Its movement was the picture's: the window is already there.)
        self.animation_scheduler.cancel_rect(handle);
        if let Err(error) = self.apply_window_alpha(handle, alpha) {
            log::debug!("Window {} not seen again after coming in: {error}", handle.0);
        }
        let _ = self.update_border_position(handle);
    }

    /// The pictures of the windows that are coming in: put away, once
    /// their movement is over or their window is gone, hidden or
    /// minimised.
    fn settle_stand_ins(&mut self) {
        self.stand_ins.sweep();
        for handle in self.stand_ins.handles() {
            let hwnd = handle.to_hwnd();
            // SAFETY: plain questions about a window that may be gone.
            let shown = unsafe {
                IsWindow(hwnd).as_bool()
                    && IsWindowVisible(hwnd).as_bool()
                    && !IsIconic(hwnd).as_bool()
            };
            // A window that went out with its workspace as a picture is
            // hidden a moment after its picture has gone aside, and by a
            // message it answers when it gets to it. Seen again before
            // that, it would flash where it was: its picture stays until
            // it is hidden (and no longer than a moment, should it never
            // be).
            if shown
                && self
                    .pictures_going
                    .get(&handle)
                    .is_some_and(|latest| Instant::now() < *latest)
            {
                continue;
            }
            // (The pictures of an overview stay while it is shown.)
            if shown && self.overview_holds(handle) {
                continue;
            }
            if !shown || self.animation_scheduler.rect_target(handle).is_none() {
                self.end_stand_in(handle);
            }
        }
    }

    // ==================== The overview of windows ====================

    fn overview_holds(&self, handle: WindowHandle) -> bool {
        self.overview
            .0
            .as_ref()
            .is_some_and(|overview| overview.items.iter().any(|(held, _)| *held == handle))
    }

    /// Whether the overview of a workspace's windows is shown.
    pub fn window_overview_is_shown(&self) -> bool {
        self.overview.0.is_some()
    }

    /// Show `windows` as live pictures of themselves, small, in `area`,
    /// to choose one of; or put that away where it is shown. `true`: it
    /// is shown now.
    ///
    /// The windows stay where they are and as large as they are, unseen;
    /// only their pictures move, to the places `expose::arrange` gives
    /// them, and back when the overview is over. A window that cannot be
    /// made unseen (one its program makes see-through itself, one that
    /// does not answer) is left out, as it is.
    pub fn toggle_window_overview(
        &mut self,
        windows: &[WindowHandle],
        area: crate::dwm::container::Rect,
    ) -> bool {
        if self.overview.0.is_some() {
            self.close_window_overview(OverviewEnd::Back);
            return false;
        }
        // (One that is still on its way out is out.)
        for handle in std::mem::take(&mut self.overview_leaving) {
            self.end_stand_in(handle);
        }
        let now = Instant::now();
        let there: Vec<(WindowHandle, crate::dwm::container::Rect)> = windows
            .iter()
            .filter_map(|handle| {
                let hwnd = handle.to_hwnd();
                // SAFETY: plain questions about a window that may be gone.
                let shown = unsafe {
                    IsWindow(hwnd).as_bool()
                        && IsWindowVisible(hwnd).as_bool()
                        && !IsIconic(hwnd).as_bool()
                };
                if !shown || is_hung(hwnd) {
                    return None;
                }
                let rect = self.get_window_rect(*handle).ok()?;
                (rect.width > 0 && rect.height > 0).then_some((*handle, rect))
            })
            .collect();
        // SAFETY: a plain question.
        let foreground = unsafe { GetForegroundWindow() };
        // (Not in a test: the keyboard is whoever runs it's, and stays.)
        let before = (!cfg!(test) && !foreground.is_invalid())
            .then(|| WindowHandle::from_hwnd(foreground));
        // The pictures first, each where its window is; then what is
        // behind them, and the pictures over that.
        let mut items = Vec::new();
        for (handle, place) in gevrek_engine::expose::arrange(&there, area) {
            let Some((_, home)) = there.iter().find(|(known, _)| *known == handle) else {
                continue;
            };
            if self.picture_for_overview(handle, *home, place, now) {
                items.push((handle, place));
            }
        }
        if items.is_empty() {
            return false;
        }
        let shown = items
            .iter()
            .map(|(handle, place)| super::overview::Shown {
                rect: *place,
                title: self.get_window_title_live(*handle),
            })
            .collect();
        let accent = self
            .border_windows
            .values()
            .next()
            .map(|border| border.active_color)
            .unwrap_or((90, 150, 255));
        let Some(backdrop) = super::overview::Backdrop::open(area, shown, accent) else {
            for (handle, _) in items {
                self.end_stand_in(handle);
            }
            return false;
        };
        self.hide_all_borders();
        self.hide_all_stackbars();
        for (handle, _) in &items {
            self.stand_ins.raise(*handle);
            self.effects_resting.insert(*handle);
        }
        // The window that had the keyboard is the one pointed at first:
        // Enter goes back to it, the arrows go on from it.
        let marked = before.and_then(|before| items.iter().position(|(handle, _)| *handle == before));
        backdrop.mark(marked);
        self.overview.0 = Some(Overview {
            backdrop,
            items,
            marked,
            before,
            opened: now,
        });
        true
    }

    /// A window of an overview as a picture of itself, from `from` (where
    /// the window is) to `to`: the window stays, unseen. `false` where
    /// that cannot be done.
    fn picture_for_overview(
        &mut self,
        handle: WindowHandle,
        from: crate::dwm::container::Rect,
        to: crate::dwm::container::Rect,
        now: Instant,
    ) -> bool {
        self.end_stand_in(handle);
        let hwnd = handle.to_hwnd();
        // SAFETY: a plain question about a window that may be gone.
        let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
        // A window its program makes see-through itself cannot be made
        // unseen without spoiling it (`set_window_alpha`).
        let layered = ex_style & WS_EX_LAYERED.0 as i32 != 0;
        if layered && !self.gevrek_layered_windows.contains(&handle) {
            return false;
        }
        let alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
        if alpha == 0 {
            return false;
        }
        let Some(part) = self.seen_part(hwnd) else {
            return false;
        };
        // The picture first, then the window unseen: what is seen never
        // shows nothing where the window was.
        if !self.stand_ins.open(handle, hwnd, alpha, from, part) {
            return false;
        }
        if self.apply_window_alpha(handle, 0).is_err() {
            self.stand_ins.take(handle);
            return false;
        }
        self.window_alphas.insert(handle, alpha);
        if !self
            .animation_scheduler
            .schedule_rect(handle, from, to, TransitionScope::Workspace, now)
        {
            // (No movement is asked for: it is there at once.)
            self.stand_ins.place(handle, to, part);
        }
        true
    }

    /// The overview is over: the pictures go back to where their windows
    /// are, and the windows are seen themselves again when they have.
    fn close_window_overview(&mut self, end: OverviewEnd) {
        let Some(overview) = self.overview.0.take() else {
            return;
        };
        let now = Instant::now();
        for (handle, place) in &overview.items {
            if !self.stand_ins.has(*handle) {
                continue;
            }
            let from = self.stand_ins.rect(*handle).unwrap_or(*place);
            let moves = self.get_window_rect(*handle).is_ok_and(|home| {
                self.animation_scheduler.schedule_rect(
                    *handle,
                    from,
                    home,
                    TransitionScope::Workspace,
                    now,
                )
            });
            if moves {
                self.overview_leaving.insert(*handle);
            } else {
                self.end_stand_in(*handle);
            }
        }
        let before = overview.before;
        // (Its window goes: the pictures fly back over the desktop.)
        drop(overview);
        let to = match end {
            OverviewEnd::Chosen(chosen) => Some(chosen),
            OverviewEnd::Back => before,
            OverviewEnd::Away => None,
        };
        if let Some(window) = to.filter(|window| self.window_exists(*window)) {
            // (Over the others on its way back, as it will be there.)
            self.stand_ins.raise(window);
            log::debug!("the overview is over: the keyboard goes to {window:?}");
            if let Err(error) = self.focus_window(window) {
                log::debug!("{window:?} did not take the keyboard after the overview: {error:#}");
            }
        }
        if self.overview_leaving.is_empty() {
            self.window_overview_is_over();
        }
    }

    /// Every picture of an overview has arrived: what was put away for
    /// it is back.
    fn window_overview_is_over(&mut self) {
        self.show_all_borders();
        self.show_all_stackbars();
    }

    /// The overview, each pass: it gets dark, its pictures that are gone
    /// are forgotten, and what the person did at it is done.
    fn tick_window_overview(&mut self) {
        use super::overview::Said;
        if !self.overview_leaving.is_empty() {
            let stand_ins = &self.stand_ins;
            self.overview_leaving.retain(|handle| stand_ins.has(*handle));
            if self.overview_leaving.is_empty() && self.overview.0.is_none() {
                self.window_overview_is_over();
            }
        }
        let Some(overview) = self.overview.0.as_mut() else {
            return;
        };
        // Darker over a moment, not at once.
        let dark = (overview.opened.elapsed().as_millis() as f32 / 160.0).min(1.0);
        overview
            .backdrop
            .darken((f32::from(super::overview::DARK) * dark) as u8);
        // A window of it went (closed, minimised): the overview is over.
        if overview
            .items
            .iter()
            .any(|(handle, _)| !self.stand_ins.has(*handle))
        {
            self.close_window_overview(OverviewEnd::Back);
            return;
        }
        let places: Vec<crate::dwm::container::Rect> =
            overview.items.iter().map(|(_, place)| *place).collect();
        let at = |x: i32, y: i32| {
            places.iter().position(|place| {
                x >= place.x
                    && x < place.x + place.width
                    && y >= place.y
                    && y < place.y + place.height
            })
        };
        let mut end = None;
        for said in overview.backdrop.said() {
            match said {
                Said::Pointed(x, y) => {
                    // (Off the pictures, the one the keys went to stays.)
                    if let Some(pointed) = at(x, y) {
                        overview.marked = Some(pointed);
                    }
                }
                Said::Pressed(x, y) => {
                    end = Some(match at(x, y) {
                        Some(pressed) => OverviewEnd::Chosen(overview.items[pressed].0),
                        None => OverviewEnd::Back,
                    });
                }
                // Escape.
                Said::Key(0x1B) => end = Some(OverviewEnd::Back),
                // Enter and the space bar.
                Said::Key(0x0D | 0x20) => {
                    end = Some(match overview.marked {
                        Some(marked) => OverviewEnd::Chosen(overview.items[marked].0),
                        None => OverviewEnd::Back,
                    });
                }
                // The arrows, and Tab for the next.
                Said::Key(key @ (0x25..=0x28 | 0x09)) => {
                    let count = places.len();
                    overview.marked = Some(match (overview.marked, key) {
                        (None, _) => 0,
                        (Some(from), 0x09) => (from + 1) % count,
                        (Some(from), key) => {
                            let (across, down) = match key {
                                0x25 => (-1, 0),
                                0x26 => (0, -1),
                                0x27 => (1, 0),
                                _ => (0, 1),
                            };
                            gevrek_engine::expose::towards(&places, from, across, down).unwrap_or(from)
                        }
                    });
                }
                Said::Key(_) => {}
                Said::Left => end = Some(OverviewEnd::Away),
            }
            if end.is_some() {
                break;
            }
        }
        overview.backdrop.mark(overview.marked);
        if let Some(end) = end {
            self.close_window_overview(end);
        }
    }

    // ==================== Batch Operations ====================

    /// Schedule a set of layout rectangles through the shared runtime
    /// animation profile. Disabled scopes are applied atomically immediately.
    pub fn apply_animated_rectangles(
        &mut self,
        operations: &[(WindowHandle, crate::dwm::container::Rect)],
        scope: TransitionScope,
    ) -> Result<()> {
        if operations.is_empty() {
            return Ok(());
        }
        // (Windows are laid out anew: an overview of them is over.)
        self.close_window_overview(OverviewEnd::Away);
        // (A window that is hidden belongs where it is laid out.)
        for (handle, target) in operations {
            self.note_placed(*handle, *target);
        }

        let now = Instant::now();
        // A window coming in as a picture of itself: the window goes to
        // its new place at once (it is unseen), and the picture turns
        // there from where it is. A place it is already bound for is no
        // news (a layout pass while it comes in).
        let mut operations = operations.to_vec();
        let pictured: Vec<(WindowHandle, crate::dwm::container::Rect)> = operations
            .iter()
            .filter(|(handle, _)| self.stand_ins.has(*handle))
            .copied()
            .collect();
        operations.retain(|(handle, _)| !self.stand_ins.has(*handle));
        for (handle, target) in pictured {
            if self.animation_scheduler.rect_target(handle) == Some(target) {
                continue;
            }
            let start = self.stand_ins.rect(handle).unwrap_or(target);
            let _ = self.batch_move_resize(&[(handle, target)]);
            if let Some(part) = self.seen_part(handle.to_hwnd()) {
                self.stand_ins.place(handle, start, part);
            }
            if !self
                .animation_scheduler
                .schedule_rect(handle, start, target, scope, now)
            {
                self.end_stand_in(handle);
            }
        }
        let operations = operations.as_slice();
        if operations.is_empty() {
            return Ok(());
        }
        if !self.animation_scheduler.scope_enabled(scope) {
            self.batch_move_resize(operations)?;
            for (handle, rect) in operations {
                let _ = self.update_border_position_explicit(
                    *handle,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                );
            }
            return Ok(());
        }

        let candidates = operations
            .iter()
            .map(|(handle, target)| {
                (
                    *handle,
                    self.get_window_rect(*handle).unwrap_or(*target),
                    *target,
                )
            })
            .collect::<Vec<_>>();
        let mut immediate = Vec::new();
        for (handle, start, target) in candidates {
            if !self
                .animation_scheduler
                .schedule_rect(handle, start, target, scope, now)
            {
                immediate.push((handle, target));
            }
        }

        if !immediate.is_empty() {
            self.batch_move_resize(&immediate)?;
            for (handle, rect) in &immediate {
                let _ = self.update_border_position_explicit(
                    *handle,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                );
            }
        }
        Ok(())
    }

    /// Place an incoming workspace just outside its monitor, then animate it
    /// into the layout rectangles already calculated for that workspace.
    ///
    /// The windows are real windows on Windows, seen wherever they are: a
    /// workspace that came in from the side would be seen crossing the
    /// display that lies on that side. So it comes from a side where no
    /// other display is (`slide_clear_of_displays`): from the side asked
    /// for where that is free, else from the other side, from below or
    /// from above; with a display on every side it does not slide.
    pub fn begin_workspace_transition(
        &mut self,
        operations: &[(WindowHandle, crate::dwm::container::Rect)],
        offset_x: i32,
        offset_y: i32,
    ) -> Result<bool> {
        // The old workspace's windows, kept shown until now
        // (`workspace_gives_way`): those that come back in at once are no
        // longer going.
        self.going_out.expected = false;
        // (A window that is minimized comes nowhere and is moved by
        // nothing: see `hide_window`.)
        let coming: Vec<(WindowHandle, crate::dwm::container::Rect)> = operations
            .iter()
            .copied()
            // SAFETY: a plain question about a window that may be gone.
            .filter(|(handle, _)| !unsafe { IsIconic(handle.to_hwnd()) }.as_bool())
            .collect();
        let operations = coming.as_slice();
        // (A window still coming in is moved as a workspace's window now.)
        let taken_over: Vec<WindowHandle> = operations
            .iter()
            .chain(self.going_out.waiting.iter())
            .map(|(handle, _)| *handle)
            .collect();
        for handle in taken_over {
            self.end_stand_in(handle);
        }
        let waiting: Vec<(WindowHandle, crate::dwm::container::Rect)> =
            std::mem::take(&mut self.going_out.waiting)
                .into_iter()
                .filter(|(handle, _)| !operations.iter().any(|(coming, _)| coming == handle))
                .collect();
        let hide_at_once = |manager: &mut Self, windows: &[(WindowHandle, crate::dwm::container::Rect)]| {
            for (handle, _) in windows {
                // (A plain call on a window that may be gone.)
                super::reach::show_window_async(handle.to_hwnd(), SW_HIDE);
                manager.going_out.leaving.remove(handle);
            }
        };
        // (With motion reduced the windows are put in their places at once:
        // nothing may be moved aside first that is then not brought in.)
        //
        // The old one's windows stay where they are a moment longer, and
        // are hidden when the new one's are there (`finish_going_out`).
        // Hidden at once, they were gone before the others were drawn
        // (every window is hidden and shown by its own program, when it
        // gets to it), and the bare desktop was seen between the two.
        if !self.animation_scheduler.moves(TransitionScope::Workspace) {
            let until = Instant::now() + HELD_FOR_THE_NEXT;
            for (handle, home) in waiting {
                self.going_out.leaving.insert(handle, (home, None, until));
            }
            return Ok(false);
        }
        let displays: Vec<(i32, i32, i32, i32)> = crate::dwm::monitor::all_displays()
            .into_iter()
            .map(|(_, info)| (info.x, info.y, info.width, info.height))
            .collect();
        let targets: Vec<(i32, i32, i32, i32)> = operations
            .iter()
            .map(|(_, rect)| (rect.x, rect.y, rect.width, rect.height))
            .collect();
        let homes: Vec<(i32, i32, i32, i32)> = waiting
            .iter()
            .map(|(_, rect)| (rect.x, rect.y, rect.width, rect.height))
            .collect();
        let ways = workspace_ways(
            self.animation_scheduler.profile().workspace_style,
            &displays,
            &targets,
            &homes,
            (offset_x, offset_y),
        );
        let now = Instant::now();
        let until = now
            + Duration::from_millis(
                u64::from(self.animation_scheduler.profile().workspace.duration_ms.max(16)) + 60,
            );
        let pictures = self.animation_scheduler.profile().workspace_slides_as_pictures;

        // The old one's windows go: aside, or fainter, and are hidden
        // when they have gone (`finish_going_out`).
        let mut went = false;
        for (handle, home) in &waiting {
            let going = if ways.fade {
                let alpha = self.window_alphas.get(handle).copied().unwrap_or(255);
                self.animation_scheduler
                    .schedule_workspace_opacity(*handle, alpha, 0, now)
                    .then_some(Some(alpha))
            } else {
                match ways.go {
                    Some((x, y)) => {
                        let away = crate::dwm::container::Rect::new(
                            home.x + x,
                            home.y + y,
                            home.width,
                            home.height,
                        );
                        // As a picture of itself where that is asked for
                        // and can be had; else the window itself is moved.
                        let pictured = pictures
                            && self.slide_as_picture(*handle, *home, away, None, now);
                        if pictured {
                            self.pictures_going
                                .insert(*handle, until + Duration::from_millis(1500));
                        }
                        let moves = pictured
                            || self.animation_scheduler.schedule_rect(
                                *handle,
                                *home,
                                away,
                                TransitionScope::Workspace,
                                now,
                            );
                        if moves {
                            self.rest_effects(*handle);
                        }
                        moves.then_some(None)
                    }
                    None => None,
                }
            };
            match going {
                Some(alpha) => {
                    self.going_out.leaving.insert(*handle, (*home, alpha, until));
                    went = true;
                }
                None => hide_at_once(self, &[(*handle, *home)]),
            }
        }

        // The new one's windows appear where they belong, from nothing.
        if ways.fade {
            let mut came = false;
            for (handle, _) in operations {
                let alpha = self.window_alphas.get(handle).copied().unwrap_or(255);
                if alpha == 0 {
                    continue;
                }
                self.set_window_alpha(*handle, 0)?;
                came |= self
                    .animation_scheduler
                    .schedule_workspace_opacity(*handle, 0, alpha, now);
            }
            return Ok(came || went);
        }
        let Some((offset_x, offset_y)) = ways.come else {
            return Ok(went);
        };
        let starts = operations
            .iter()
            .map(|(handle, target)| {
                (
                    *handle,
                    crate::dwm::container::Rect::new(
                        target.x + offset_x,
                        target.y + offset_y,
                        target.width,
                        target.height,
                    ),
                )
            })
            .collect::<Vec<_>>();
        // As pictures of themselves where that is asked for and can be
        // had: such a window is in its place from the start, unseen, and
        // its picture comes in. The others are moved themselves.
        let mut moved: Vec<(
            WindowHandle,
            crate::dwm::container::Rect,
            crate::dwm::container::Rect,
        )> = Vec::new();
        for ((handle, start), (_, target)) in starts.iter().zip(operations.iter()) {
            if pictures && self.slide_as_picture(*handle, *start, *target, Some(*target), now) {
                self.rest_effects(*handle);
                continue;
            }
            moved.push((*handle, *start, *target));
        }
        let starts: Vec<(WindowHandle, crate::dwm::container::Rect)> = moved
            .iter()
            .map(|(handle, start, _)| (*handle, *start))
            .collect();
        self.batch_move_resize(&starts)?;
        for (handle, start) in &starts {
            let _ = self.update_border_position_explicit(
                *handle,
                start.x,
                start.y,
                start.width,
                start.height,
            );
        }
        for (handle, start, target) in &moved {
            self.animation_scheduler.schedule_rect(
                *handle,
                *start,
                *target,
                TransitionScope::Workspace,
                now,
            );
            self.rest_effects(*handle);
        }
        Ok(true)
    }

    /// Animate a newly managed window using the configured window channel.
    pub fn start_configured_window_enter(&mut self, handle: WindowHandle) -> Result<bool> {
        if self.animation_scheduler.is_close_pending(handle) {
            return Ok(false);
        }
        if !self
            .animation_scheduler
            .scope_enabled(TransitionScope::Explicit)
        {
            return Ok(false);
        }
        let style = self.animation_scheduler.profile().window_enter_style;
        if style == WindowTransitionStyle::None {
            return Ok(false);
        }
        // Sliding and growing are movement; fading in is not.
        if style != WindowTransitionStyle::Fade
            && !self.animation_scheduler.moves(TransitionScope::Explicit)
        {
            return Ok(false);
        }

        // A new window usually has a layout transition in flight (adoption
        // just tiled it). The entrance replaces that transition, so it must
        // end where the layout puts the window, not where the window is now;
        // otherwise new windows never reach their tile.
        let target = match self.animation_scheduler.rect_target(handle) {
            Some(target) => target,
            None => self.get_window_rect(handle)?,
        };
        let now = Instant::now();
        match style {
            WindowTransitionStyle::None => Ok(false),
            WindowTransitionStyle::Fade => {
                self.end_stand_in(handle);
                let target_alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
                if target_alpha == 0 {
                    return Ok(false);
                }
                self.set_window_alpha(handle, 0)?;
                Ok(self
                    .animation_scheduler
                    .schedule_opacity(handle, 0, target_alpha, 0, now))
            }
            WindowTransitionStyle::Slide => {
                let start = animation_styles::offset_rect(target, -target.width / 3, 0);
                if self.enter_by_stand_in(handle, start, target, 0, now) {
                    return Ok(true);
                }
                self.batch_move_resize(&[(handle, start)])?;
                let _ = self.update_border_position_explicit(
                    handle,
                    start.x,
                    start.y,
                    start.width,
                    start.height,
                );
                Ok(self
                    .animation_scheduler
                    .schedule_explicit_rect(handle, start, target, 0, now))
            }
            WindowTransitionStyle::Scale => {
                let start = animation_styles::centered_scaled_rect(target, 20);
                if self.enter_by_stand_in(handle, start, target, 0, now) {
                    return Ok(true);
                }
                self.batch_move_resize(&[(handle, start)])?;
                let _ = self.update_border_position_explicit(
                    handle,
                    start.x,
                    start.y,
                    start.width,
                    start.height,
                );
                Ok(self
                    .animation_scheduler
                    .schedule_explicit_rect(handle, start, target, 0, now))
            }
        }
    }

    /// Move foreground activation back to the Windows shell when no eligible
    /// managed window exists. This is the final fallback for a no-focus rule.
    pub fn focus_desktop(&self) -> bool {
        *DESKTOP_KEPT
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(std::time::Instant::now());
        self.refocus_desktop()
    }

    /// Give the desktop the keyboard again, without counting it as asked
    /// for anew (`desktop_kept_just_now`).
    pub fn refocus_desktop(&self) -> bool {
        unsafe {
            let shell = GetShellWindow();
            if shell.is_invalid() {
                return false;
            }
            // (As for a window: Windows refuses a plain request from a
            // program that did not get the last input, and a command that
            // came from elsewhere than a key is exactly that.)
            take_foreground(shell);
            GetForegroundWindow() == shell
        }
    }

    /// Run an explicit graph animation through the same non-blocking
    /// scheduler used by layouts, workspaces, and borders.
    pub fn start_window_animation(
        &mut self,
        handle: WindowHandle,
        animation_type: DwmAnimationType,
        duration_ms: u32,
    ) -> Result<bool> {
        if !self
            .animation_scheduler
            .scope_enabled(TransitionScope::Explicit)
        {
            return Ok(false);
        }

        let target = self.get_window_rect(handle)?;
        let now = Instant::now();
        match animation_styles::start_rect(animation_type, target) {
            // Fade tweens opacity instead of geometry.
            None => {
                self.end_stand_in(handle);
                let target_alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
                self.set_window_alpha(handle, 0)?;
                Ok(self.animation_scheduler.schedule_opacity(
                    handle,
                    0,
                    target_alpha,
                    duration_ms,
                    now,
                ))
            }
            Some(_) if !self.animation_scheduler.moves(TransitionScope::Explicit) => Ok(false),
            Some(start) => {
                if self.enter_by_stand_in(handle, start, target, duration_ms, now) {
                    return Ok(true);
                }
                self.batch_move_resize(&[(handle, start)])?;
                let _ = self.update_border_position_explicit(
                    handle,
                    start.x,
                    start.y,
                    start.width,
                    start.height,
                );
                Ok(self.animation_scheduler.schedule_explicit_rect(
                    handle,
                    start,
                    target,
                    duration_ms,
                    now,
                ))
            }
        }
    }

    /// Advance every runtime-owned animation by one event-loop frame.
    pub fn tick_runtime_animations(&mut self) -> Result<()> {
        let frame = self.animation_scheduler.tick(Instant::now());
        let close_windows = frame.close_windows;
        if !frame.rectangles.is_empty() {
            // A window coming in stays where it is: its picture moves.
            let (pictured, moved): (Vec<_>, Vec<_>) = frame
                .rectangles
                .iter()
                .partition(|(handle, _)| self.stand_ins.has(*handle));
            self.batch_move_resize(&moved)?;
            for (handle, rect) in pictured {
                if let Some(part) = self.seen_part(handle.to_hwnd()) {
                    self.stand_ins.place(handle, rect, part);
                }
            }
            for (handle, rect) in &frame.rectangles {
                // (The pictures of an overview have none: the borders are
                // put away for it, and are back when it is over.)
                if self.overview_holds(*handle) || self.overview_leaving.contains(handle) {
                    continue;
                }
                let _ = self.update_border_position_explicit(
                    *handle,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                );
            }
        }
        for border in frame.borders {
            self.apply_border_visual(border.handle, border.color, border.alpha)?;
        }
        for (handle, alpha) in frame.opacities {
            self.set_window_alpha(handle, alpha)?;
        }
        let now = Instant::now();
        for handle in close_windows {
            // A window that went out to be minimised: minimised now, and
            // as solid as it was for when it is brought back.
            if self.minimise_after_exit.remove(&handle) {
                self.animation_scheduler.cancel_window(handle);
                self.minimize_window_at_once(handle);
                if let Some(restore) = self.exit_restore.remove(&handle) {
                    let _ = self.set_window_alpha(handle, restore.alpha);
                }
                continue;
            }
            if let Some(restore) = self.exit_restore.remove(&handle) {
                self.close_checks
                    .insert(handle, (now + CLOSE_CHECK_DELAY, restore));
            }
            self.close_window_immediate(handle)?;
        }
        self.undo_cancelled_exits(now);
        self.finish_going_out(now);
        self.settle_stand_ins();
        self.settle_resting_effects();
        self.tick_window_overview();
        Ok(())
    }

    /// A window that is still alive a while after `WM_CLOSE` refused to
    /// close. Undo its exit animation so it is not left invisible or shrunk.
    fn undo_cancelled_exits(&mut self, now: Instant) {
        if self.close_checks.is_empty() {
            return;
        }
        let due: Vec<(WindowHandle, ExitRestore)> = self
            .close_checks
            .iter()
            .filter(|(_, (deadline, _))| *deadline <= now)
            .map(|(handle, (_, restore))| (*handle, *restore))
            .collect();
        for (handle, restore) in due {
            self.close_checks.remove(&handle);
            if !self.window_exists(handle) {
                continue;
            }
            log::debug!(
                "Window {} is still open after its exit animation; restoring it",
                handle.0
            );
            let _ = self.batch_move_resize(&[(handle, restore.rect)]);
            let _ = self.set_window_alpha(handle, restore.alpha);
            let _ = self.update_border_position(handle);
        }
    }

    /// Batch move and resize multiple windows in a single operation
    ///
    /// Uses BeginDeferWindowPos/DeferWindowPos/EndDeferWindowPos for
    /// flicker-free, atomic window positioning. If any deferred call fails,
    /// the system has already freed the HDWP, so continuing with it would
    /// make `EndDeferWindowPos` fail and move nothing at all. In that case
    /// every window is positioned on its own with an async `SetWindowPos`,
    /// skipping only the window that failed.
    pub fn batch_move_resize(
        &self,
        operations: &[(WindowHandle, crate::dwm::container::Rect)],
    ) -> Result<()> {
        if operations.is_empty() {
            return Ok(());
        }

        // Target outer rectangles (visible rect plus the invisible frame).
        let targets: Vec<(WindowHandle, HWND, i32, i32, i32, i32)> = operations
            .iter()
            .filter_map(|(handle, rect)| {
                let hwnd = handle.to_hwnd();
                if !unsafe { IsWindow(hwnd) }.as_bool() {
                    return None;
                }
                let (left, top, right, bottom) = self.get_window_frame_offsets(hwnd);
                Some((
                    *handle,
                    hwnd,
                    rect.x - left,
                    rect.y - top,
                    rect.width + left + right,
                    rect.height + top + bottom,
                ))
            })
            .collect();
        if targets.is_empty() {
            return Ok(());
        }

        // EndDeferWindowPos waits for every window in the batch; one hung app
        // would freeze the whole layout. Hung windows get a posted, async
        // move instead and land in place once they recover.
        // (And a window that was sent a move by post before, which it may
        // not have carried out yet, is sent this one by post as well: see
        // `move_must_be_posted`.)
        let (hung, targets): (Vec<_>, Vec<_>) = targets.into_iter().partition(|target| {
            move_must_be_posted(target.1) || super::reach::by_helper(target.1)
        });
        for (handle, hwnd, x, y, width, height) in &hung {
            log::trace!(
                "Window {} is not responding; positioning it asynchronously",
                handle.0
            );
            let posted = unsafe {
                super::reach::set_window_pos(
                    *hwnd,
                    None,
                    *x,
                    *y,
                    *width,
                    *height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                )
            };
            if posted.is_ok() && !super::reach::by_helper(*hwnd) {
                note_posted_move(*hwnd, (*x, *y, *width, *height));
            }
        }
        if targets.is_empty() {
            return Ok(());
        }

        let skip = match Self::defer_window_positions(&targets) {
            Ok(()) => {
                log::trace!("Batch positioned {} windows", targets.len());
                return Ok(());
            }
            Err(failed) => failed,
        };

        let mut positioned = 0;
        for (index, (handle, hwnd, x, y, width, height)) in targets.iter().enumerate() {
            if Some(index) == skip {
                continue;
            }
            // Async, so one hung window cannot block the rest of the layout.
            match unsafe {
                super::reach::set_window_pos(
                    *hwnd,
                    None,
                    *x,
                    *y,
                    *width,
                    *height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                )
            } {
                Ok(()) => {
                    positioned += 1;
                    note_posted_move(*hwnd, (*x, *y, *width, *height));
                }
                Err(error) => log::warn!("SetWindowPos failed for window {}: {}", handle.0, error),
            }
        }
        log::debug!(
            "Positioned {} of {} windows individually after a deferred batch failed",
            positioned,
            targets.len()
        );
        Ok(())
    }

    /// Applies `targets` as one deferred batch. On failure returns the index
    /// of the window whose `DeferWindowPos` failed (`None` if the batch
    /// itself could not be started or committed).
    fn defer_window_positions(
        targets: &[(WindowHandle, HWND, i32, i32, i32, i32)],
    ) -> std::result::Result<(), Option<usize>> {
        use windows::Win32::UI::WindowsAndMessaging::{
            BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos,
        };

        unsafe {
            let mut hdwp = BeginDeferWindowPos(targets.len() as i32).map_err(|error| {
                log::warn!("BeginDeferWindowPos failed: {}", error);
                None
            })?;
            for (index, (handle, hwnd, x, y, width, height)) in targets.iter().enumerate() {
                hdwp = DeferWindowPos(
                    hdwp,
                    *hwnd,
                    None, // No z-order change
                    *x,
                    *y,
                    *width,
                    *height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .map_err(|error| {
                    log::warn!(
                        "DeferWindowPos failed for window {}: {}; falling back to per-window positioning",
                        handle.0,
                        error
                    );
                    Some(index)
                })?;
            }
            EndDeferWindowPos(hdwp).map_err(|error| {
                log::warn!("EndDeferWindowPos failed: {}", error);
                None
            })
        }
    }

    /// Show every window still in the hidden-window journal, whichever path
    /// hid it (dispatch, graph cloak nodes, scratchpads). Returns how many
    /// live windows were shown.
    pub fn restore_journaled_windows(&mut self) -> usize {
        let mut restored = 0;
        for handle in super::hidden_journal::hidden_handles() {
            let alive = self.window_exists(handle);
            if self.show_window(handle).is_ok() && alive {
                restored += 1;
            }
        }
        restored
    }

    /// Hide multiple windows using cloaking (for workspace switch)
    pub fn batch_cloak(&self, handles: &[WindowHandle], cloak: bool) -> Result<()> {
        for handle in handles {
            // cloak_window already handles hide_border/show_border internally
            self.cloak_window(*handle, cloak)?;
        }
        Ok(())
    }

    pub fn get_window_position(&self, handle: WindowHandle) -> Result<(i32, i32)> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect)?;
                let (left_offset, top_offset, _, _) = self.get_window_frame_offsets(hwnd);
                Ok((rect.left + left_offset, rect.top + top_offset))
            } else {
                Err(anyhow::anyhow!("Invalid window handle"))
            }
        }
    }

    pub fn get_window_size(&self, handle: WindowHandle) -> Result<(i32, i32)> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect)?;
                let (left_offset, top_offset, right_offset, bottom_offset) =
                    self.get_window_frame_offsets(hwnd);
                Ok((
                    (rect.right - rect.left - left_offset - right_offset).max(0),
                    (rect.bottom - rect.top - top_offset - bottom_offset).max(0),
                ))
            } else {
                Err(anyhow::anyhow!("Invalid window handle"))
            }
        }
    }

    pub fn get_window_rect(&self, handle: WindowHandle) -> Result<crate::dwm::container::Rect> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect)?;
                let (left_offset, top_offset, right_offset, bottom_offset) =
                    self.get_window_frame_offsets(hwnd);
                Ok(crate::dwm::container::Rect::new(
                    rect.left + left_offset,
                    rect.top + top_offset,
                    (rect.right - rect.left - left_offset - right_offset).max(0),
                    (rect.bottom - rect.top - top_offset - bottom_offset).max(0),
                ))
            } else {
                Err(anyhow::anyhow!("Invalid window handle"))
            }
        }
    }

    /// A maximised window is given `room` (what is seen of it; the edges
    /// Windows hides of a maximised window stay hidden beyond it), and
    /// stays maximised. For a window that Windows maximised over the room
    /// the window manager keeps for a panel: Windows knows nothing of that
    /// room. `true` when the window was moved.
    pub fn keep_maximised_within(
        &self,
        handle: WindowHandle,
        room: crate::dwm::container::Rect,
    ) -> bool {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() || !IsZoomed(hwnd).as_bool() {
                return false;
            }
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return false;
            }
            let (left, top, right, bottom) = self.get_window_frame_offsets(hwnd);
            let seen = (
                rect.left + left,
                rect.top + top,
                rect.right - right,
                rect.bottom - bottom,
            );
            let wanted = (room.x, room.y, room.x + room.width, room.y + room.height);
            if seen == wanted || room.width <= 0 || room.height <= 0 {
                return false;
            }
            super::reach::set_window_pos(
                hwnd,
                None,
                wanted.0 - left,
                wanted.1 - top,
                room.width + left + right,
                room.height + top + bottom,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .is_ok()
        }
    }

    pub fn window_exists(&self, handle: WindowHandle) -> bool {
        unsafe { IsWindow(handle.to_hwnd()).as_bool() }
    }

    pub fn window_has_title(&self, handle: WindowHandle, title: &str) -> Result<bool> {
        if let Some(info) = self.windows.get(&handle) {
            Ok(info.title.contains(title))
        } else {
            Ok(false)
        }
    }

    /// The alpha of the classic border window: nothing while the border is
    /// drawn together with the effects.
    fn classic_border_alpha(&self, alpha: u8) -> u8 {
        if self.auras.borrow().draws_border() {
            0
        } else {
            alpha
        }
    }

    /// Take the effects settings (shadow, glow, gradient border, tint) and
    /// bring every bordered window in line with them.
    /// A glow of its own reach and strength around one window (a floating
    /// one's), or the configuration's again; what is drawn follows at once.
    /// Whether the windows of programs that run as an administrator are
    /// told from the others by their border
    /// (`desktop.platform.windows.mark_elevated_windows`). Returns whether
    /// it is another than before (the borders are then to be made anew).
    pub fn set_mark_elevated(&mut self, mark: bool) -> bool {
        if std::mem::replace(&mut self.mark_elevated, mark) == mark {
            return false;
        }
        if !mark {
            // (Their glow is the configuration's again.)
            let mut auras = self.auras.borrow_mut();
            for target in self.border_windows.keys() {
                if crate::elevated::window_is_elevated(target.to_hwnd().0 as isize) {
                    auras.set_glow(target.to_hwnd(), None);
                }
            }
        }
        true
    }

    /// Whether a window's border is the one of an elevated program's
    /// window: crimson, a lighter red while it has the keyboard, no glow.
    fn marks(&self, handle: WindowHandle) -> bool {
        self.mark_elevated && crate::elevated::window_is_elevated(handle.to_hwnd().0 as isize)
    }

    pub fn set_window_glow(
        &mut self,
        handle: WindowHandle,
        glow: Option<gevrek_engine::contract::GlowStrength>,
    ) {
        // (An elevated program's window has none, whatever is asked.)
        let glow = if self.marks(handle) {
            Some(NO_GLOW)
        } else {
            glow
        };
        if !self.auras.borrow_mut().set_glow(handle.to_hwnd(), glow) {
            return;
        }
        if let Some((color, alpha)) = self
            .border_windows
            .get(&handle)
            .map(|info| (info.current_color, info.current_alpha))
        {
            let _ = self.apply_border_visual(handle, color, alpha);
        }
    }

    pub fn set_window_effects(&mut self, effects: gevrek_model::settings::WindowEffectsProfile) {
        if !self.auras.borrow_mut().set_effects(effects) {
            return;
        }
        let targets: Vec<(WindowHandle, (u8, u8, u8), u8)> = self
            .border_windows
            .iter()
            .map(|(target, info)| (*target, info.current_color, info.current_alpha))
            .collect();
        for (target, color, alpha) in targets {
            // Sets the classic border's alpha for the new settings, and syncs.
            let _ = self.apply_border_visual(target, color, alpha);
        }
    }

    /// Draw what surrounds `target` for its current place, colour and focus.
    fn sync_effects(&self, target: WindowHandle) {
        let mut auras = self.auras.borrow_mut();
        if !auras.effects().any() && auras.count() == (0, 0) {
            return;
        }
        let target_hwnd = target.to_hwnd();
        // (Left out while the window slides with its workspace.)
        if self.effects_resting.contains(&target) {
            return;
        }
        let Some(info) = self.border_windows.get(&target) else {
            auras.remove(target_hwnd);
            return;
        };
        unsafe {
            let border_hwnd = info.border_hwnd.to_hwnd();
            let mut rect = RECT::default();
            if GetWindowRect(border_hwnd, &mut rect).is_err() {
                return;
            }
            let grow = info.width + info.offset;
            let visible = IsWindowVisible(border_hwnd).as_bool()
                && IsWindowVisible(target_hwnd).as_bool()
                && !IsIconic(target_hwnd).as_bool();
            auras.sync(super::aura::Target {
                hwnd: target_hwnd,
                behind: border_hwnd,
                frame: (
                    rect.left + grow,
                    rect.top + grow,
                    rect.right - rect.left - grow * 2,
                    rect.bottom - rect.top - grow * 2,
                ),
                border_width: info.width,
                border_offset: info.offset,
                color: info.current_color,
                alpha: info.current_alpha,
                focused: info.is_focused,
                visible,
            });
        }
    }

    /// Whether `tick_effects` has something to advance every frame.
    pub fn effects_animate(&self) -> bool {
        self.auras.borrow().spins() && self.border_windows.values().any(|info| info.is_focused)
    }

    /// Called from the runtime loop: turns the focused window's gradient
    /// border one step further, if it is set to turn.
    pub fn tick_effects(&self) {
        if !self.auras.borrow().spins() {
            return;
        }
        let focused: Vec<WindowHandle> = self
            .border_windows
            .iter()
            .filter(|(_, info)| info.is_focused)
            .map(|(target, _)| *target)
            .collect();
        for target in focused {
            self.sync_effects(target);
        }
    }

    /// Decorate a window with a colored border
    /// Creates an overlay window behind the target window to simulate a border
    pub fn decorate_window(
        &mut self,
        handle: WindowHandle,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> Result<()> {
        let gevrek_engine::workspace::BorderLook {
            width,
            offset,
            style,
            active_color,
            inactive_color,
            active_alpha,
            inactive_alpha,
            ..
        } = *look;
        // A window of a program that runs as an administrator has a
        // border of its own colours, and no glow (`marks`).
        let (active_color, inactive_color, active_alpha, inactive_alpha) = if self.marks(handle) {
            self.auras
                .borrow_mut()
                .set_glow(handle.to_hwnd(), Some(NO_GLOW));
            (ELEVATED_FOCUSED, ELEVATED, 255, 255)
        } else {
            (active_color, inactive_color, active_alpha, inactive_alpha)
        };
        // What goes with a window (a dialog) has no border: the border is
        // for a program's own window (`window_kind.rs`).
        if crate::window_kind::of_window(handle.to_hwnd().0 as isize)
            != crate::window_kind::Kind::Ordinary
        {
            return Ok(());
        }
        // CRITICAL: Never create borders for our own border windows!
        // This prevents infinite recursion of borders-for-borders
        if self.is_border_window(handle) {
            log::warn!("⚠️  Attempted to create border for a border window! Rejected.");
            return Ok(());
        }

        // Also check by class name in case border_hwnd_set isn't populated yet
        unsafe {
            let hwnd = handle.to_hwnd();
            let mut class_name: [u16; 256] = [0; 256];
            let class_len = GetClassNameW(hwnd, &mut class_name);
            if class_len > 0 {
                let class_str = String::from_utf16_lossy(&class_name[0..class_len as usize]);
                if class_str.starts_with("Gevrek_") {
                    log::warn!("⚠️  Attempted to create border for Gevrek internal window (class={})! Rejected.", class_str);
                    return Ok(());
                }
            }
        }

        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // Remove existing border if any (must remove from HashMap to prevent double borders)
            if let Some(old_border_info) = self.border_windows.remove(&handle) {
                // CRITICAL: Remove from border_hwnd_set BEFORE destroying.
                // Without this, stale entries accumulate. Since Windows recycles HWNDs,
                // a new app window could get a recycled HWND still in the set,
                // causing is_border_window() to incorrectly return true.
                self.border_hwnd_set.remove(&old_border_info.border_hwnd);
                let old_border_hwnd = old_border_info.border_hwnd.to_hwnd();
                if IsWindow(old_border_hwnd).as_bool() {
                    let _ = DestroyWindow(old_border_hwnd);
                    log::trace!("Removed old border before creating new one");
                }
            }

            // Compute invisible frame insets and the border rect around the
            // visible window area. The insets are stored as the initial
            // last-known-good value and refreshed at position time.
            let mut window_rect = RECT::default();
            GetWindowRect(hwnd, &mut window_rect)?;
            let cached_frame_offsets = self.query_frame_insets(hwnd).unwrap_or((0, 0, 0, 0));
            let (border_x, border_y, border_width_total, border_height_total) =
                border_rect_for_window(
                    (
                        window_rect.left,
                        window_rect.top,
                        window_rect.right,
                        window_rect.bottom,
                    ),
                    cached_frame_offsets,
                    width,
                    offset,
                );
            // (Around the strip of tabs too, where the window has one.)
            let (border_y, border_height_total) =
                border_rect_with_strip(border_y, border_height_total, self.strip_above(handle));

            // Create color value for the border based on focus state (RGB to COLORREF)
            let (r, g, b) = if is_focused {
                active_color
            } else {
                inactive_color
            };
            let color = COLORREF((b as u32) << 16 | (g as u32) << 8 | (r as u32));
            let alpha = if is_focused {
                active_alpha
            } else {
                inactive_alpha
            };

            // Create unique class name
            let class_name_str = "Gevrek_Border\0".to_string();
            let class_name_wide: Vec<u16> = class_name_str.encode_utf16().collect();

            // Register a window class for borders (once)
            let wc = WNDCLASSW {
                lpfnWndProc: Some(border_window_proc),
                hInstance: HINSTANCE(std::ptr::null_mut()),
                lpszClassName: windows::core::PCWSTR(class_name_wide.as_ptr()),
                hbrBackground: HBRUSH(std::ptr::null_mut()), // No background - we paint manually
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC, // Own DC for better performance
                ..Default::default()
            };
            let _ = RegisterClassW(&wc); // Ignore if already registered

            // Create the border overlay window with layered style for better performance
            let border_hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, // Layered for performance, transparent to mouse, not in taskbar
                windows::core::PCWSTR(class_name_wide.as_ptr()),
                windows::core::w!(""),
                WS_POPUP | WS_VISIBLE, // Popup window, always visible
                border_x,
                border_y,
                border_width_total,
                border_height_total,
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )?;

            // Store the border color in window user data for WM_PAINT
            SetWindowLongW(border_hwnd, GWLP_USERDATA, color.0 as i32);

            // Set layered window attributes: color key (magenta = transparent interior) + alpha
            let _ = SetLayeredWindowAttributes(
                border_hwnd,
                COLORREF(0),
                self.classic_border_alpha(alpha),
                LWA_ALPHA,
            );

            // Position border behind the target window
            // NOTE: No SWP_ASYNCWINDOWPOS here - border must be positioned synchronously
            // on creation to prevent flicker at wrong position
            SetWindowPos(
                border_hwnd,
                hwnd, // Place behind the target window
                border_x,
                border_y,
                border_width_total,
                border_height_total,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
            )?;

            apply_border_window_region(
                border_hwnd,
                border_width_total,
                border_height_total,
                width,
                offset,
                style,
            );

            // Force window to show and redraw
            let _ = ShowWindow(border_hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(border_hwnd);

            // Verify window was created
            if IsWindow(border_hwnd).as_bool() {
                log::trace!(
                    "Border created for window: {}x{} with {}px border",
                    border_width_total - (width * 2) - (offset * 2),
                    border_height_total - (width * 2) - (offset * 2),
                    width
                );
            } else {
                log::error!("Border window creation failed!");
                return Ok(());
            }

            // Store the border window info
            let border_handle = WindowHandle::from_hwnd(border_hwnd);
            let border_info = BorderInfo {
                border_hwnd: border_handle,
                width,
                offset,
                style,
                active_color,
                inactive_color,
                active_alpha,
                inactive_alpha,
                is_focused,
                current_color: if is_focused {
                    active_color
                } else {
                    inactive_color
                },
                current_alpha: if is_focused {
                    active_alpha
                } else {
                    inactive_alpha
                },
                last_update: Instant::now(),
                frame_offsets: cached_frame_offsets,
            };
            self.border_windows.insert(handle, border_info);
            // Also add to O(1) lookup set
            self.border_hwnd_set.insert(border_handle);
        }
        self.sync_effects(handle);
        Ok(())
    }

    /// Decorate a window with a colored border at an EXPLICIT position
    /// This is preferred during tiling operations to avoid timing issues with GetWindowRect
    pub fn decorate_window_at(
        &mut self,
        handle: WindowHandle,
        at: gevrek_engine::container::Rect,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> Result<()> {
        let (x, y, target_width, target_height) = (at.x, at.y, at.width, at.height);
        let gevrek_engine::workspace::BorderLook {
            width,
            offset,
            style,
            active_color,
            inactive_color,
            active_alpha,
            inactive_alpha,
            ..
        } = *look;
        // A window of a program that runs as an administrator has a
        // border of its own colours, and no glow (`marks`).
        let (active_color, inactive_color, active_alpha, inactive_alpha) = if self.marks(handle) {
            self.auras
                .borrow_mut()
                .set_glow(handle.to_hwnd(), Some(NO_GLOW));
            (ELEVATED_FOCUSED, ELEVATED, 255, 255)
        } else {
            (active_color, inactive_color, active_alpha, inactive_alpha)
        };
        // What goes with a window (a dialog) has no border: the border is
        // for a program's own window (`window_kind.rs`).
        if crate::window_kind::of_window(handle.to_hwnd().0 as isize)
            != crate::window_kind::Kind::Ordinary
        {
            return Ok(());
        }
        // CRITICAL: Never create borders for our own border windows!
        // This prevents infinite recursion of borders-for-borders
        if self.is_border_window(handle) {
            log::warn!("⚠️  Attempted to create border for a border window! Rejected.");
            return Ok(());
        }

        // Also check by class name in case border_hwnd_set isn't populated yet
        unsafe {
            let hwnd = handle.to_hwnd();
            let mut class_name: [u16; 256] = [0; 256];
            let class_len = GetClassNameW(hwnd, &mut class_name);
            if class_len > 0 {
                let class_str = String::from_utf16_lossy(&class_name[0..class_len as usize]);
                if class_str.starts_with("Gevrek_") {
                    log::warn!("⚠️  Attempted to create border for Gevrek internal window (class={})! Rejected.", class_str);
                    return Ok(());
                }
            }
        }

        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(());
            }

            // Remove existing border if any
            if let Some(old_border_info) = self.border_windows.remove(&handle) {
                // CRITICAL: Clean up border_hwnd_set to prevent stale HWND entries
                self.border_hwnd_set.remove(&old_border_info.border_hwnd);
                let old_border_hwnd = old_border_info.border_hwnd.to_hwnd();
                if IsWindow(old_border_hwnd).as_bool() {
                    let _ = DestroyWindow(old_border_hwnd);
                }
            }

            // Initial last-known-good frame insets; refreshed at position time.
            let cached_frame_offsets = self.query_frame_insets(hwnd).unwrap_or((0, 0, 0, 0));

            // Create color value for the border based on focus state (RGB to COLORREF)
            let (r, g, b) = if is_focused {
                active_color
            } else {
                inactive_color
            };
            let color = COLORREF((b as u32) << 16 | (g as u32) << 8 | (r as u32));
            let alpha = if is_focused {
                active_alpha
            } else {
                inactive_alpha
            };

            // Create unique class name
            let class_name_str = "Gevrek_Border\0".to_string();
            let class_name_wide: Vec<u16> = class_name_str.encode_utf16().collect();

            // Register a window class for borders (once)
            let wc = WNDCLASSW {
                lpfnWndProc: Some(border_window_proc),
                hInstance: HINSTANCE(std::ptr::null_mut()),
                lpszClassName: windows::core::PCWSTR(class_name_wide.as_ptr()),
                hbrBackground: HBRUSH(std::ptr::null_mut()),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            // Calculate border dimensions with offset using EXPLICIT position.
            // The explicit rect is the window's visible frame (the layout
            // engine compensates for invisible frame insets when placing
            // windows), so no frame inset conversion is needed here.
            let (border_x, border_y, border_width_total, border_height_total) =
                border_rect_around_frame((x, y, target_width, target_height), width, offset);
            // (Around the strip of tabs too, where the window has one.)
            let (border_y, border_height_total) =
                border_rect_with_strip(border_y, border_height_total, self.strip_above(handle));

            log::trace!("🎨 Creating border at EXPLICIT position: window ({},{}) {}x{}, border ({},{}) {}x{}",
                x, y, target_width, target_height,
                border_x, border_y, border_width_total, border_height_total);

            // Create the border overlay window
            let border_hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                windows::core::PCWSTR(class_name_wide.as_ptr()),
                windows::core::w!(""),
                WS_POPUP | WS_VISIBLE,
                border_x,
                border_y,
                border_width_total,
                border_height_total,
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )?;

            // Store the border color in window user data for WM_PAINT
            SetWindowLongW(border_hwnd, GWLP_USERDATA, color.0 as i32);

            // Set layered window attributes: color key (magenta = transparent interior) + alpha
            let _ = SetLayeredWindowAttributes(
                border_hwnd,
                COLORREF(0),
                self.classic_border_alpha(alpha),
                LWA_ALPHA,
            );

            // Position border behind the target window
            // NOTE: No SWP_ASYNCWINDOWPOS - synchronous on creation to prevent flicker
            SetWindowPos(
                border_hwnd,
                hwnd,
                border_x,
                border_y,
                border_width_total,
                border_height_total,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
            )?;

            apply_border_window_region(
                border_hwnd,
                border_width_total,
                border_height_total,
                width,
                offset,
                style,
            );

            let _ = ShowWindow(border_hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(border_hwnd);

            // Store the border window info
            let border_handle = WindowHandle::from_hwnd(border_hwnd);
            let border_info = BorderInfo {
                border_hwnd: border_handle,
                width,
                offset,
                style,
                active_color,
                inactive_color,
                active_alpha,
                inactive_alpha,
                is_focused,
                current_color: if is_focused {
                    active_color
                } else {
                    inactive_color
                },
                current_alpha: if is_focused {
                    active_alpha
                } else {
                    inactive_alpha
                },
                last_update: Instant::now(),
                frame_offsets: cached_frame_offsets,
            };
            self.border_windows.insert(handle, border_info);
            self.border_hwnd_set.insert(border_handle);
        }
        self.sync_effects(handle);
        Ok(())
    }

    /// Update border position when target window moves
    /// Throttled to max 144 FPS (7ms minimum between updates) for smooth rendering
    pub fn update_border_position(&mut self, target_handle: WindowHandle) -> Result<()> {
        const MIN_UPDATE_INTERVAL: Duration = Duration::from_millis(7); // ~144 FPS for smooth desktop-like rendering

        // Copy what we need, then drop the map borrow so fresh frame insets
        // can be queried through &self below.
        let Some(border_info) = self.border_windows.get(&target_handle) else {
            return Ok(());
        };
        let border_hwnd_handle = border_info.border_hwnd;
        let border_width = border_info.width;
        let border_offset = border_info.offset;
        let border_style = border_info.style;
        let cached_insets = border_info.frame_offsets;

        // Throttle updates to prevent event storm
        let now = Instant::now();
        if now.duration_since(border_info.last_update) < MIN_UPDATE_INTERVAL {
            return Ok(()); // Skip this update, too soon
        }

        unsafe {
            let target_hwnd = target_handle.to_hwnd();
            let border_hwnd = border_hwnd_handle.to_hwnd();

            if !IsWindow(target_hwnd).as_bool() || !IsWindow(border_hwnd).as_bool() {
                return Ok(());
            }

            if IsIconic(target_hwnd).as_bool()
                || !IsWindowVisible(target_hwnd).as_bool()
                || self.kept_out_of_sight(target_handle)
            {
                let _ = ShowWindow(border_hwnd, SW_HIDE);
                self.auras.borrow().set_visible(target_hwnd, false);
                if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
                    border_info.last_update = now;
                }
                return Ok(());
            }

            // Refresh the invisible frame insets at position time: they go
            // stale after DPI, style, or theme changes. Fall back to the last
            // known good value when DWM reports transient garbage during a
            // live resize.
            let frame_insets = match self.query_frame_insets(target_hwnd) {
                Some(fresh) => {
                    if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
                        border_info.frame_offsets = fresh;
                    }
                    fresh
                }
                None => cached_insets,
            };

            // Get real-time window rect and compute the border rect around the
            // VISIBLE window area (single authoritative computation).
            let mut rect = RECT::default();
            GetWindowRect(target_hwnd, &mut rect)?;
            let (border_x, border_y, border_width_total, border_height_total) =
                border_rect_for_window(
                    (rect.left, rect.top, rect.right, rect.bottom),
                    frame_insets,
                    border_width,
                    border_offset,
                );
            // (Around the strip of tabs too, where the window has one.)
            let (border_y, border_height_total) =
                border_rect_with_strip(border_y, border_height_total, self.strip_above(target_handle));

            // Move and resize border window, keeping it behind its target window.
            // No SWP_NOCOPYBITS: let Windows copy old content and only invalidate new areas.
            // This is faster and avoids full-window black flash during resize.
            SetWindowPos(
                border_hwnd,
                target_hwnd, // Place immediately behind the target window in z-order
                border_x,
                border_y,
                border_width_total,
                border_height_total,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;

            apply_border_window_region(
                border_hwnd,
                border_width_total,
                border_height_total,
                border_width,
                border_offset,
                border_style,
            );

            // Invalidate + force synchronous repaint so new area is filled
            // before DWM composites the next frame (prevents black flash).
            let _ = InvalidateRect(border_hwnd, None, true); // true = trigger WM_ERASEBKGND
            let _ = UpdateWindow(border_hwnd); // Synchronous: sends WM_PAINT now
        }

        self.sync_effects(target_handle);

        // Update timestamp after successful move
        if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
            border_info.last_update = now;
        }
        Ok(())
    }

    /// Update border position to an explicit rect (used when tiling to avoid timing issues with GetWindowRect)
    /// This is preferred over update_border_position when we know the intended position
    pub fn update_border_position_explicit(
        &mut self,
        target_handle: WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        // Get border info data we need before the mutable borrow
        let border_data = self.border_windows.get(&target_handle).map(|border_info| {
            (
                border_info.border_hwnd,
                border_info.width,
                border_info.offset,
                border_info.style,
            )
        });

        if let Some((border_hwnd_handle, border_width, border_offset, border_style)) = border_data {
            unsafe {
                let border_hwnd = border_hwnd_handle.to_hwnd();
                let target_hwnd = target_handle.to_hwnd();

                if !IsWindow(border_hwnd).as_bool() {
                    return Ok(());
                }

                if IsIconic(target_hwnd).as_bool()
                || !IsWindowVisible(target_hwnd).as_bool()
                || self.kept_out_of_sight(target_handle)
            {
                    let _ = ShowWindow(border_hwnd, SW_HIDE);
                    self.auras.borrow().set_visible(target_hwnd, false);
                    return Ok(());
                }

                // The explicit rect IS the visible window area (the layout
                // engine compensates for invisible frame insets when placing
                // windows); compute the border rect around it with the same
                // authoritative math as the live path.
                let (border_x, border_y, border_width_total, border_height_total) =
                    border_rect_around_frame((x, y, width, height), border_width, border_offset);
                // (Around the strip of tabs too, where the window has one.)
                let (border_y, border_height_total) =
                    border_rect_with_strip(border_y, border_height_total, self.strip_above(target_handle));

                log::trace!(
                    "🎨 Border position explicit: window at ({},{}) {}x{}, border at ({},{}) {}x{}",
                    x,
                    y,
                    width,
                    height,
                    border_x,
                    border_y,
                    border_width_total,
                    border_height_total
                );

                // A border that only moves (its window slides: a change of
                // workspace, a window coming in) keeps its shape and its
                // picture. Cut and painted again on every frame, each
                // border of both workspaces was drawn anew and handed to
                // the compositor sixty times a second for nothing.
                let mut was = RECT::default();
                let same_size = IsWindowVisible(border_hwnd).as_bool()
                    && GetWindowRect(border_hwnd, &mut was).is_ok()
                    && was.right - was.left == border_width_total
                    && was.bottom - was.top == border_height_total;

                // Move and resize border window
                SetWindowPos(
                    border_hwnd,
                    target_hwnd, // Keep border behind target
                    border_x,
                    border_y,
                    border_width_total,
                    border_height_total,
                    if same_size {
                        SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOSIZE
                    } else {
                        SWP_NOACTIVATE | SWP_SHOWWINDOW
                    },
                )?;

                if !same_size {
                    apply_border_window_region(
                        border_hwnd,
                        border_width_total,
                        border_height_total,
                        border_width,
                        border_offset,
                        border_style,
                    );

                    // Invalidate border window to trigger repaint
                    let _ = InvalidateRect(border_hwnd, None, false);
                }
            }

            self.sync_effects(target_handle);

            // Update timestamp
            if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
                border_info.last_update = Instant::now();
            }
        }
        Ok(())
    }

    /// Check if a window handle IS a border window (not whether it HAS a border)
    /// This checks if the handle is one of our border windows
    /// O(1) lookup using HashSet instead of O(n) iteration
    pub fn is_border_window(&self, handle: WindowHandle) -> bool {
        // Stackbar windows are Gevrek-owned decoration windows too: every
        // caller that skips borders (management, focus-follows-mouse, state
        // save, appearance application) must skip them as well.
        self.border_hwnd_set.contains(&handle)
            || self.stackbar_hwnd_set.contains(&handle)
            || self.auras.borrow().owns(handle.to_hwnd())
    }

    /// Whether the handle belongs to a stackbar tab strip window.
    pub fn is_stackbar_window(&self, handle: WindowHandle) -> bool {
        self.stackbar_hwnd_set.contains(&handle)
    }

    /// Check if a window has a border
    pub fn has_border(&self, handle: WindowHandle) -> bool {
        self.border_windows.contains_key(&handle)
    }

    /// Set border focus state and update color/alpha accordingly
    pub fn set_border_focus(
        &mut self,
        target_handle: WindowHandle,
        is_focused: bool,
    ) -> Result<()> {
        let Some((current_color, current_alpha, target_color, target_alpha)) =
            self.border_windows.get(&target_handle).map(|border_info| {
                (
                    border_info.current_color,
                    border_info.current_alpha,
                    if is_focused {
                        border_info.active_color
                    } else {
                        border_info.inactive_color
                    },
                    if is_focused {
                        border_info.active_alpha
                    } else {
                        border_info.inactive_alpha
                    },
                )
            })
        else {
            return Ok(());
        };

        if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
            border_info.is_focused = is_focused;
        }
        if self.animation_scheduler.schedule_border(
            target_handle,
            current_color,
            target_color,
            current_alpha,
            target_alpha,
            Instant::now(),
        ) {
            return Ok(());
        }
        self.apply_border_visual(target_handle, target_color, target_alpha)
    }

    fn apply_border_visual(
        &mut self,
        target_handle: WindowHandle,
        color_rgb: (u8, u8, u8),
        alpha: u8,
    ) -> Result<()> {
        // Copy what we need, then drop the map borrow so fresh frame insets
        // can be queried through &self below.
        let Some(border_info) = self.border_windows.get(&target_handle) else {
            return Ok(());
        };
        let border_hwnd_handle = border_info.border_hwnd;
        let border_width = border_info.width;
        let border_offset = border_info.offset;
        let border_style = border_info.style;
        let cached_insets = border_info.frame_offsets;

        if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
            border_info.current_color = color_rgb;
            border_info.current_alpha = alpha;
        }

        unsafe {
            let border_hwnd = border_hwnd_handle.to_hwnd();
            if !IsWindow(border_hwnd).as_bool() {
                return Ok(());
            }

            let (r, g, b) = color_rgb;

            // Update layered window attributes: color key + alpha
            let _ = SetLayeredWindowAttributes(
                border_hwnd,
                COLORREF(0),
                self.classic_border_alpha(alpha),
                LWA_ALPHA,
            );

            // Update border color in window user data
            let color = COLORREF((b as u32) << 16 | (g as u32) << 8 | (r as u32));
            SetWindowLongW(border_hwnd, GWLP_USERDATA, color.0 as i32);

            // Recompute border position from TARGET window (not from border's current rect).
            // Using the border's own GetWindowRect and re-applying the same coords causes
            // Windows to optimize away the SetWindowPos (no geometry change detected),
            // so DWM never recomposites the layered window surface with the new color.
            let target_hwnd = target_handle.to_hwnd();
            if IsWindow(target_hwnd).as_bool() {
                // Refresh frame insets (see update_border_position); fall back
                // to the last known good value on transient DWM failure.
                let frame_insets = match self.query_frame_insets(target_hwnd) {
                    Some(fresh) => {
                        if let Some(border_info) = self.border_windows.get_mut(&target_handle) {
                            border_info.frame_offsets = fresh;
                        }
                        fresh
                    }
                    None => cached_insets,
                };

                let mut rect = RECT::default();
                let _ = GetWindowRect(target_hwnd, &mut rect);
                let (border_x, border_y, border_w, border_h) = border_rect_for_window(
                    (rect.left, rect.top, rect.right, rect.bottom),
                    frame_insets,
                    border_width,
                    border_offset,
                );
                // (Around the strip of tabs too, where the window has one.)
                let (border_y, border_h) =
                    border_rect_with_strip(border_y, border_h, self.strip_above(target_handle));

                let _ = SetWindowPos(
                    border_hwnd,
                    target_hwnd,
                    border_x,
                    border_y,
                    border_w,
                    border_h,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
                );

                apply_border_window_region(
                    border_hwnd,
                    border_w,
                    border_h,
                    border_width,
                    border_offset,
                    border_style,
                );
            }

            // Force synchronous full repaint with new color
            let _ = RedrawWindow(
                border_hwnd,
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );

            // Re-apply layered attributes AFTER painting to force DWM to
            // re-read the window surface (DWM caches layered window surfaces
            // and only re-reads them when it detects a change).
            let _ = SetLayeredWindowAttributes(
                border_hwnd,
                COLORREF(0),
                self.classic_border_alpha(alpha),
                LWA_ALPHA,
            );

            log::trace!(
                "Updated animated border visual for window {} to rgb({},{},{}) alpha {}",
                target_handle.0,
                r,
                g,
                b,
                alpha,
            );
        }
        self.sync_effects(target_handle);
        Ok(())
    }

    /// Hide border window (when target window is hidden)
    pub fn hide_border(&self, target_handle: WindowHandle) -> Result<()> {
        unsafe {
            if let Some(border_info) = self.border_windows.get(&target_handle) {
                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    let _ = ShowWindow(border_hwnd, SW_HIDE);
                    log::trace!("Hid border for window");
                }
            }
        }
        self.auras
            .borrow()
            .set_visible(target_handle.to_hwnd(), false);
        Ok(())
    }

    /// Show border window (when target window is shown)
    pub fn show_border(&self, target_handle: WindowHandle) -> Result<()> {
        unsafe {
            if let Some(border_info) = self.border_windows.get(&target_handle) {
                let target_hwnd = target_handle.to_hwnd();
                if !IsWindow(target_hwnd).as_bool()
                    || IsIconic(target_hwnd).as_bool()
                    || !IsWindowVisible(target_hwnd).as_bool()
                    || self.kept_out_of_sight(target_handle)
                {
                    return Ok(());
                }

                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    // Refresh frame insets (see update_border_position); fall
                    // back to the last known good value on transient failure.
                    let frame_insets = self
                        .query_frame_insets(target_hwnd)
                        .unwrap_or(border_info.frame_offsets);

                    let mut rect = RECT::default();
                    GetWindowRect(target_hwnd, &mut rect)?;
                    let (border_x, border_y, border_w, border_h) = border_rect_for_window(
                        (rect.left, rect.top, rect.right, rect.bottom),
                        frame_insets,
                        border_info.width,
                        border_info.offset,
                    );
                    // (Around the strip of tabs too, where the window has one.)
                    let (border_y, border_h) =
                        border_rect_with_strip(border_y, border_h, self.strip_above(target_handle));

                    SetWindowPos(
                        border_hwnd,
                        target_hwnd,
                        border_x,
                        border_y,
                        border_w,
                        border_h,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )?;
                    apply_border_window_region(
                        border_hwnd,
                        border_w,
                        border_h,
                        border_info.width,
                        border_info.offset,
                        border_info.style,
                    );
                    let _ = ShowWindow(border_hwnd, SW_SHOWNOACTIVATE);
                    let _ = RedrawWindow(
                        border_hwnd,
                        None,
                        None,
                        RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
                    );
                    log::trace!("Showed border for window");
                }
            }
        }
        self.sync_effects(target_handle);
        Ok(())
    }

    /// Hide all border windows (used during expose mode)
    pub fn hide_all_borders(&self) {
        unsafe {
            for border_info in self.border_windows.values() {
                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    let _ = ShowWindow(border_hwnd, SW_HIDE);
                }
            }
        }
        self.auras.borrow().set_all_visible(false);
    }

    /// Show all border windows (used when exiting expose mode)
    pub fn show_all_borders(&self) {
        unsafe {
            for (target_handle, border_info) in &self.border_windows {
                let target_hwnd = target_handle.to_hwnd();
                if !IsWindow(target_hwnd).as_bool()
                    || IsIconic(target_hwnd).as_bool()
                    || !IsWindowVisible(target_hwnd).as_bool()
                    || self.kept_out_of_sight(*target_handle)
                {
                    continue;
                }

                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    let _ = ShowWindow(border_hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
        let targets: Vec<WindowHandle> = self.border_windows.keys().copied().collect();
        for target in targets {
            self.sync_effects(target);
        }
    }

    /// Remove border when target window is destroyed
    pub fn remove_border(&mut self, target_handle: WindowHandle) -> Result<()> {
        self.auras.borrow_mut().remove(target_handle.to_hwnd());
        unsafe {
            if let Some(border_info) = self.border_windows.remove(&target_handle) {
                // Also remove from O(1) lookup set
                self.border_hwnd_set.remove(&border_info.border_hwnd);

                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    DestroyWindow(border_hwnd)?;
                    log::debug!("Removed border for destroyed window");
                }
            }
        }
        Ok(())
    }

    pub fn maximize_window(&self, handle: WindowHandle) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                anyhow::bail!("Window {:?} no longer exists", handle);
            }
            show_window_nonblocking(hwnd, SW_MAXIMIZE);
        }
        Ok(())
    }

    pub fn toggle_maximized(&self, handle: WindowHandle) -> Result<bool> {
        if self.is_window_maximized(handle) {
            self.unmaximize_window(handle)?;
            Ok(false)
        } else {
            self.maximize_window(handle)?;
            Ok(true)
        }
    }

    /// Minimise a window as gevrek does it: it fades where it stands (as
    /// a window that is closed does, where windows have a way out), and is
    /// minimised when it is gone. Windows' own way, the window flying to
    /// its button on the taskbar, is for a desktop with that taskbar
    /// (`set_system_transitions`).
    pub fn minimize_window(&mut self, handle: WindowHandle) -> Result<()> {
        if !self.window_exists(handle) {
            anyhow::bail!("Window {:?} no longer exists", handle);
        }
        if self.animation_scheduler.is_close_pending(handle) {
            return Ok(());
        }
        self.end_stand_in(handle);
        let fades = self
            .animation_scheduler
            .scope_enabled(TransitionScope::Explicit)
            && self.animation_scheduler.profile().window_exit_style != WindowTransitionStyle::None;
        if fades {
            let alpha = self.window_alphas.get(&handle).copied().unwrap_or(255);
            if self
                .animation_scheduler
                .schedule_exit_opacity(handle, alpha, Instant::now())
            {
                if let Ok(rect) = self.get_window_rect(handle) {
                    self.exit_restore
                        .insert(handle, ExitRestore { rect, alpha });
                }
                self.minimise_after_exit.insert(handle);
                return Ok(());
            }
        }
        self.minimize_window_at_once(handle);
        Ok(())
    }

    fn minimize_window_at_once(&self, handle: WindowHandle) {
        unsafe {
            let hwnd = handle.to_hwnd();
            if IsWindow(hwnd).as_bool() {
                show_window_nonblocking(hwnd, SW_MINIMIZE);
            }
        }
    }

    /// Windows' own movement of a window (flying to the taskbar when it is
    /// minimised and back, growing when it is maximised) switched off for
    /// it, or on again. Off where the taskbar is put away: a window then
    /// flew to a taskbar nobody sees.
    pub fn set_system_transitions(&self, handle: WindowHandle, off: bool) {
        // (The attribute's number: DWMWA_TRANSITIONS_FORCEDISABLED.)
        let value: i32 = i32::from(off);
        // SAFETY: a plain attribute of a window; one that is gone has none.
        unsafe {
            let _ = super::reach::dwm_set_window_attribute(
                handle.to_hwnd(),
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(3),
                (&value as *const i32).cast(),
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    /// The same for every window gevrek knows.
    pub fn set_system_transitions_for_all(&self, off: bool) {
        for handle in self.windows.keys() {
            self.set_system_transitions(*handle, off);
        }
    }

    pub fn is_window_pinned(&self, handle: WindowHandle) -> bool {
        unsafe {
            let ex_style = GetWindowLongW(handle.to_hwnd(), GWL_EXSTYLE);
            (ex_style & WS_EX_TOPMOST.0 as i32) != 0
        }
    }

    pub fn toggle_window_pinned(&self, handle: WindowHandle) -> Result<bool> {
        let pinned = !self.is_window_pinned(handle);
        self.set_always_on_top(handle, pinned)?;
        Ok(pinned)
    }

    pub fn is_window_fullscreen(&self, handle: WindowHandle) -> bool {
        self.fullscreen_windows.contains_key(&handle)
    }

    /// Toggle a borderless, monitor-sized fullscreen state while preserving the
    /// window's previous style, bounds, and maximized state.
    pub fn toggle_fullscreen(&mut self, handle: WindowHandle) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                self.fullscreen_windows.remove(&handle);
                anyhow::bail!("Window {:?} no longer exists", handle);
            }

            if let Some(restore) = self.fullscreen_windows.remove(&handle) {
                super::reach::set_window_long(hwnd, GWL_STYLE, restore.style);
                super::reach::set_window_long(hwnd, GWL_EXSTYLE, restore.ex_style);
                super::reach::set_window_pos(
                    hwnd,
                    None,
                    restore.rect.left,
                    restore.rect.top,
                    restore.rect.right - restore.rect.left,
                    restore.rect.bottom - restore.rect.top,
                    nonblocking_swp(hwnd, SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED),
                )?;
                if restore.was_maximized {
                    show_window_nonblocking(hwnd, SW_MAXIMIZE);
                }
                self.show_border(handle)?;
                return Ok(false);
            }

            let was_maximized = IsZoomed(hwnd).as_bool();
            if was_maximized {
                show_window_nonblocking(hwnd, SW_RESTORE);
            }

            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect)?;
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                anyhow::bail!("Failed to query monitor for window {:?}", handle);
            }

            self.fullscreen_windows.insert(
                handle,
                FullscreenRestore {
                    rect,
                    style,
                    ex_style,
                    was_maximized,
                },
            );

            let borderless = style
                & !(WS_CAPTION.0 as i32
                    | WS_THICKFRAME.0 as i32
                    | WS_MINIMIZEBOX.0 as i32
                    | WS_MAXIMIZEBOX.0 as i32);
            super::reach::set_window_long(hwnd, GWL_STYLE, borderless);
            let bounds = monitor_info.rcMonitor;
            super::reach::set_window_pos(
                hwnd,
                Some(HWND_TOP),
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                nonblocking_swp(hwnd, SWP_NOOWNERZORDER | SWP_FRAMECHANGED),
            )?;
            self.hide_border(handle)?;
            Ok(true)
        }
    }

    /// Whether Gevrek removed this window's title bar.
    pub fn is_title_bar_hidden(&self, handle: WindowHandle) -> bool {
        self.title_bar_hidden.contains_key(&handle)
    }

    /// Show or hide the window title bar (`WS_CAPTION`) while preserving the
    /// original style so it can be restored exactly. This generalizes the
    /// style-stripping mechanism used by the borderless fullscreen path.
    pub fn set_title_bar_visible(&mut self, handle: WindowHandle, visible: bool) -> Result<()> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                self.title_bar_hidden.remove(&handle);
                anyhow::bail!("Window {:?} no longer exists", handle);
            }

            if visible {
                let Some(style) = self.title_bar_hidden.remove(&handle) else {
                    return Ok(());
                };
                super::reach::set_window_long(hwnd, GWL_STYLE, style);
            } else {
                if self.title_bar_hidden.contains_key(&handle) {
                    return Ok(());
                }
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                self.title_bar_hidden.insert(handle, style);
                super::reach::set_window_long(hwnd, GWL_STYLE, style & !(WS_CAPTION.0 as i32));
            }
            super::reach::set_window_pos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                nonblocking_swp(
                    hwnd,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                ),
            )?;
        }
        Ok(())
    }

    /// Toggle the window title bar, returning `true` when it is now visible.
    pub fn toggle_title_bar(&mut self, handle: WindowHandle) -> Result<bool> {
        let hidden = self.is_title_bar_hidden(handle);
        self.set_title_bar_visible(handle, hidden)?;
        Ok(hidden)
    }

    /// Remove border overlays whose target or border HWND no longer exists.
    pub fn cleanup_stale_borders(&mut self) -> Result<usize> {
        let stale_targets = self
            .border_windows
            .iter()
            .filter_map(|(target, border_info)| unsafe {
                let target_valid = IsWindow(target.to_hwnd()).as_bool();
                let border_valid = IsWindow(border_info.border_hwnd.to_hwnd()).as_bool();
                // (And a border around what is no program's own window:
                // one that became a menu's or a palette's after it was
                // given its border, or was given one before it was known
                // for what it is.)
                let bordered = crate::window_kind::of_window(target.to_hwnd().0 as isize)
                    == crate::window_kind::Kind::Ordinary;
                (!target_valid || !border_valid || !bordered).then_some(*target)
            })
            .collect::<Vec<_>>();

        let mut removed = 0;
        unsafe {
            for target in stale_targets {
                self.auras.borrow_mut().remove(target.to_hwnd());
                if let Some(border_info) = self.border_windows.remove(&target) {
                    self.border_hwnd_set.remove(&border_info.border_hwnd);
                    let border_hwnd = border_info.border_hwnd.to_hwnd();
                    if IsWindow(border_hwnd).as_bool() {
                        DestroyWindow(border_hwnd)?;
                    }
                    removed += 1;
                }
            }
        }

        self.border_hwnd_set = self
            .border_windows
            .values()
            .map(|info| info.border_hwnd)
            .collect();
        // What was drawn around a window that has no border any more (a
        // border that could not be made again, for one) goes too.
        let bordered: HashSet<isize> = self
            .border_windows
            .keys()
            .map(|target| target.to_hwnd().0 as isize)
            .collect();
        self.auras
            .borrow_mut()
            .retain(|key| bordered.contains(&key));

        Ok(removed)
    }

    /// Destroy every border overlay owned by the WM.
    ///
    /// Used during shutdown and hard cleanup so border HWNDs cannot outlive the
    /// runtime if a target window was hidden, minimized, or destroyed mid-cycle.
    pub fn destroy_all_borders(&mut self) -> Result<usize> {
        self.auras.borrow_mut().clear();
        let borders = self.border_windows.drain().collect::<Vec<_>>();
        let mut destroyed = 0;

        unsafe {
            for (_, border_info) in borders {
                self.border_hwnd_set.remove(&border_info.border_hwnd);
                let border_hwnd = border_info.border_hwnd.to_hwnd();
                if IsWindow(border_hwnd).as_bool() {
                    DestroyWindow(border_hwnd)?;
                    destroyed += 1;
                }
            }
        }

        Ok(destroyed)
    }

    // ===== Stackbar (tab strip for grouped containers) =====

    /// Create, update, reposition, or remove the stackbar of every grouped
    /// container so the live windows exactly match `desired`. Bars whose key
    /// is absent from `desired` (group dissolved, focus moved to another tab,
    /// stackbar disabled) are destroyed. A bar is shown only while its key
    /// window is visible.
    pub fn sync_stackbars(
        &mut self,
        desired: &[StackbarContainerState],
        config: &StackbarProfile,
    ) -> Result<()> {
        let desired_keys: HashSet<WindowHandle> = if config.enabled {
            desired.iter().map(|state| state.key).collect()
        } else {
            HashSet::new()
        };
        let stale: Vec<WindowHandle> = self
            .stackbar_windows
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .copied()
            .collect();
        for key in stale {
            self.remove_stackbar(key)?;
        }
        if !config.enabled {
            return Ok(());
        }

        for state in desired {
            if state.tabs.len() < 2 {
                continue;
            }
            let (x, y, width, height) = state.rect;
            if width <= 0 || height <= 0 {
                continue;
            }
            let titles: Vec<String> = state
                .tabs
                .iter()
                .map(|tab| {
                    self.windows
                        .get(tab)
                        .map(|info| info.title.clone())
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| "Untitled".to_string())
                })
                .collect();
            let focused_tab = state.focused_tab.min(state.tabs.len() - 1);

            if self.stackbar_windows.contains_key(&state.key) {
                self.update_stackbar(state.key, &titles, focused_tab, config)?;
                self.position_stackbar(state.key, x, y, width, config.height)?;
            } else {
                self.create_stackbar(state, titles, focused_tab, config)?;
            }
        }
        Ok(())
    }

    /// How high the strip of tabs above a window is, where it has one:
    /// the window's border, glow and shadow go around both.
    fn strip_above(&self, handle: WindowHandle) -> i32 {
        self.stackbar_windows
            .get(&handle)
            .map_or(0, |info| info.height)
    }

    /// What marks the tab of the window that is shown: the colour of that
    /// window's border, where it has one.
    fn stackbar_mark(&self, key: WindowHandle, config: &StackbarProfile) -> COLORREF {
        match self.border_windows.get(&key) {
            Some(border) => stackbar_color(border.current_color),
            None => stackbar_color(config.active_text_color),
        }
    }

    /// How far a window's strip reaches down behind it.
    fn stackbar_overlap_of(key: WindowHandle) -> i32 {
        // SAFETY: a plain query; 0 for a window that is gone.
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(key.to_hwnd()) };
        stackbar_overlap(dpi)
    }

    /// The strip is the upper end of its window's frame: its upper
    /// corners are round as a window's are, its lower end is straight
    /// (and lies behind the window).
    fn round_stackbar(stackbar_hwnd: HWND, width: i32, height: i32, radius: i32) {
        // SAFETY: the region is the window's once it is given to it.
        unsafe {
            let region = CreateRoundRectRgn(
                0,
                0,
                width + 1,
                height + radius * 2 + 1,
                radius * 2,
                radius * 2,
            );
            if !region.is_invalid() {
                let _ = SetWindowRgn(stackbar_hwnd, region, true);
            }
        }
    }

    /// The window a strip lies right behind: its window's border, where
    /// that has one (the border is drawn over the strip's edge, as it is
    /// over the window's), else the window.
    fn stackbar_goes_behind(&self, key: WindowHandle) -> HWND {
        self.border_windows
            .get(&key)
            .map(|border| border.border_hwnd.to_hwnd())
            // SAFETY: a plain query.
            .filter(|border| unsafe { IsWindow(*border).as_bool() })
            .unwrap_or_else(|| key.to_hwnd())
    }

    /// Create the stackbar window for one grouped container.
    fn create_stackbar(
        &mut self,
        state: &StackbarContainerState,
        titles: Vec<String>,
        focused_tab: usize,
        config: &StackbarProfile,
    ) -> Result<()> {
        unsafe {
            let key_hwnd = state.key.to_hwnd();
            if !IsWindow(key_hwnd).as_bool() {
                return Ok(());
            }
            let (x, y, width, _) = state.rect;
            let overlap = Self::stackbar_overlap_of(state.key);

            let class_name_wide: Vec<u16> = "Gevrek_Stackbar\0".encode_utf16().collect();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(stackbar_window_proc),
                hInstance: HINSTANCE(std::ptr::null_mut()),
                lpszClassName: windows::core::PCWSTR(class_name_wide.as_ptr()),
                hbrBackground: HBRUSH(std::ptr::null_mut()), // We paint manually
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                style: CS_HREDRAW | CS_VREDRAW,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc); // Ignore if already registered

            // Clickable (no WS_EX_TRANSPARENT) but never activatable, and kept
            // out of the taskbar/Alt-Tab like the border windows.
            let stackbar_hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                windows::core::PCWSTR(class_name_wide.as_ptr()),
                windows::core::w!(""),
                WS_POPUP,
                x,
                y,
                width,
                config.height + overlap,
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )?;
            Self::round_stackbar(stackbar_hwnd, width, config.height + overlap, overlap);

            let data = Box::new(StackbarWndData {
                titles,
                focused_tab,
                background: stackbar_color(config.background_color),
                active_text: stackbar_color(config.active_text_color),
                inactive_text: stackbar_color(config.inactive_text_color),
                mark: self.stackbar_mark(state.key, config),
                bar_height: config.height,
                font: stackbar_create_font(config.height, &config.font),
                font_height: config.height,
                font_family: config.font.clone(),
                key: state.key,
                click_queue: Arc::clone(&self.stackbar_click_queue),
            });
            SetWindowLongPtrW(stackbar_hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);

            // Place below the container's focused window (and its border):
            // the strip is reserved space, and what of it lies lower is
            // behind the window.
            SetWindowPos(
                stackbar_hwnd,
                self.stackbar_goes_behind(state.key),
                x,
                y,
                width,
                config.height + overlap,
                SWP_NOACTIVATE,
            )?;

            let stackbar_handle = WindowHandle::from_hwnd(stackbar_hwnd);
            self.stackbar_windows.insert(
                state.key,
                StackbarInfo {
                    stackbar_hwnd: stackbar_handle,
                    height: config.height,
                },
            );
            self.stackbar_hwnd_set.insert(stackbar_handle);
            // The window's frame goes around its strip from now on.
            self.reframe(state.key);

            if self.is_window_visible(state.key) {
                let _ = ShowWindow(stackbar_hwnd, SW_SHOWNOACTIVATE);
                let _ = UpdateWindow(stackbar_hwnd);
            }
        }
        Ok(())
    }

    /// Push new titles/colors/focus into an existing stackbar and repaint.
    fn update_stackbar(
        &mut self,
        key: WindowHandle,
        titles: &[String],
        focused_tab: usize,
        config: &StackbarProfile,
    ) -> Result<()> {
        let mark = self.stackbar_mark(key, config);
        // (Another height: the frame around it is another.)
        let reframe = match self.stackbar_windows.get_mut(&key) {
            Some(info) if info.height != config.height => {
                info.height = config.height;
                true
            }
            _ => false,
        };
        if reframe {
            self.reframe(key);
        }
        unsafe {
            let Some(info) = self.stackbar_windows.get(&key) else {
                return Ok(());
            };
            let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
            if !IsWindow(stackbar_hwnd).as_bool() {
                return Ok(());
            }
            let data_ptr = GetWindowLongPtrW(stackbar_hwnd, GWLP_USERDATA) as *mut StackbarWndData;
            if data_ptr.is_null() {
                return Ok(());
            }
            let data = &mut *data_ptr;
            let changed = data.titles != titles
                || data.focused_tab != focused_tab
                || data.background != stackbar_color(config.background_color)
                || data.active_text != stackbar_color(config.active_text_color)
                || data.inactive_text != stackbar_color(config.inactive_text_color)
                || data.mark != mark
                || data.font_height != config.height
                || data.font_family != config.font;
            data.mark = mark;
            data.bar_height = config.height;
            data.titles = titles.to_vec();
            data.focused_tab = focused_tab;
            data.background = stackbar_color(config.background_color);
            data.active_text = stackbar_color(config.active_text_color);
            data.inactive_text = stackbar_color(config.inactive_text_color);
            if data.font_height != config.height || data.font_family != config.font {
                let new_font = stackbar_create_font(config.height, &config.font);
                if !new_font.is_invalid() {
                    let _ = DeleteObject(data.font);
                    data.font = new_font;
                    data.font_height = config.height;
                    data.font_family = config.font.clone();
                }
            }
            if changed {
                let _ = InvalidateRect(stackbar_hwnd, None, true);
            }
        }
        Ok(())
    }

    /// Move/resize a stackbar to the top strip of its container rect and
    /// reconcile visibility with the key window.
    fn position_stackbar(
        &self,
        key: WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        unsafe {
            let Some(info) = self.stackbar_windows.get(&key) else {
                return Ok(());
            };
            let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
            if !IsWindow(stackbar_hwnd).as_bool() {
                return Ok(());
            }
            let key_hwnd = key.to_hwnd();
            // (As high as its tabs, and on behind the window's corners.)
            let overlap = Self::stackbar_overlap_of(key);
            let height = height + overlap;
            let mut current = RECT::default();
            GetWindowRect(stackbar_hwnd, &mut current)?;
            if IsWindow(key_hwnd).as_bool()
                && (current.left != x
                    || current.top != y
                    || current.right - current.left != width
                    || current.bottom - current.top != height)
            {
                SetWindowPos(
                    stackbar_hwnd,
                    self.stackbar_goes_behind(key),
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE,
                )?;
                if current.right - current.left != width || current.bottom - current.top != height {
                    Self::round_stackbar(stackbar_hwnd, width, height, overlap);
                }
            }
            if self.is_window_visible(key) {
                let _ = ShowWindow(stackbar_hwnd, SW_SHOWNOACTIVATE);
            } else {
                let _ = ShowWindow(stackbar_hwnd, SW_HIDE);
            }
        }
        Ok(())
    }

    /// Destroy the stackbar keyed by `key`, if any.
    pub fn remove_stackbar(&mut self, key: WindowHandle) -> Result<()> {
        unsafe {
            if let Some(info) = self.stackbar_windows.remove(&key) {
                self.stackbar_hwnd_set.remove(&info.stackbar_hwnd);
                let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
                if IsWindow(stackbar_hwnd).as_bool() {
                    DestroyWindow(stackbar_hwnd)?;
                }
                // (The window's frame is around the window alone again.)
                self.reframe(key);
            }
        }
        Ok(())
    }

    /// The frame of a window (border, glow, shadow) put where it belongs
    /// now, whenever it was last moved: its strip of tabs has come, gone,
    /// or is of another height.
    fn reframe(&mut self, key: WindowHandle) {
        if let Some(border) = self.border_windows.get_mut(&key) {
            let now = Instant::now();
            border.last_update = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        }
        let _ = self.update_border_position(key);
    }

    /// Hide the stackbar keyed by `key` (coupled to `hide_window`).
    pub fn hide_stackbar(&self, key: WindowHandle) {
        unsafe {
            if let Some(info) = self.stackbar_windows.get(&key) {
                let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
                if IsWindow(stackbar_hwnd).as_bool() {
                    let _ = ShowWindow(stackbar_hwnd, SW_HIDE);
                }
            }
        }
    }

    /// Show the stackbar keyed by `key` (coupled to `show_window`).
    fn show_stackbar(&self, key: WindowHandle) {
        unsafe {
            if let Some(info) = self.stackbar_windows.get(&key) {
                let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
                if IsWindow(stackbar_hwnd).as_bool() {
                    let _ = ShowWindow(stackbar_hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
    }

    /// Hide every stackbar (used during expose mode).
    pub fn hide_all_stackbars(&self) {
        unsafe {
            for info in self.stackbar_windows.values() {
                let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
                if IsWindow(stackbar_hwnd).as_bool() {
                    let _ = ShowWindow(stackbar_hwnd, SW_HIDE);
                }
            }
        }
    }

    /// Show every stackbar whose key window is visible (expose mode exit).
    pub fn show_all_stackbars(&self) {
        unsafe {
            for (key, info) in &self.stackbar_windows {
                if !self.is_window_visible(*key) {
                    continue;
                }
                let stackbar_hwnd = info.stackbar_hwnd.to_hwnd();
                if IsWindow(stackbar_hwnd).as_bool() {
                    let _ = ShowWindow(stackbar_hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
    }

    /// Destroy stackbars whose key or bar HWND no longer exists. Returns the
    /// number destroyed. Mirrors `cleanup_stale_borders`.
    pub fn cleanup_stale_stackbars(&mut self) -> Result<usize> {
        let stale_keys = self
            .stackbar_windows
            .iter()
            .filter_map(|(key, info)| unsafe {
                let key_valid = IsWindow(key.to_hwnd()).as_bool();
                let bar_valid = IsWindow(info.stackbar_hwnd.to_hwnd()).as_bool();
                (!key_valid || !bar_valid).then_some(*key)
            })
            .collect::<Vec<_>>();

        let mut removed = 0;
        for key in stale_keys {
            self.remove_stackbar(key)?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Destroy every stackbar owned by the WM (shutdown path).
    pub fn destroy_all_stackbars(&mut self) -> Result<usize> {
        let keys = self.stackbar_windows.keys().copied().collect::<Vec<_>>();
        let mut destroyed = 0;
        for key in keys {
            self.remove_stackbar(key)?;
            destroyed += 1;
        }
        Ok(destroyed)
    }

    /// Drain tab clicks queued by the stackbar window procedures. Each entry
    /// is `(key window, tab index)`; the runtime dispatches the group-focus
    /// path for them.
    pub fn poll_stackbar_clicks(&self) -> Vec<(WindowHandle, usize)> {
        match self.stackbar_click_queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        }
    }

    // ===== Windows 11 Native Border Color (DWMWA_BORDER_COLOR) =====
    // These use DWM's built-in border color API - much simpler and more reliable than overlay windows.
    // Only available on Windows 11 Build 22000+. Falls back gracefully on older versions.

    /// Set the native DWM border color on a window (Windows 11+)
    /// Uses DWMWA_BORDER_COLOR (attribute 34) for a thin, perfectly z-ordered border
    /// that is composited by DWM itself - no overlay window needed.
    pub fn set_native_border_color(
        &self,
        handle: WindowHandle,
        r: u8,
        g: u8,
        b: u8,
    ) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(false);
            }

            // COLORREF format: 0x00BBGGRR
            let color: u32 = (b as u32) << 16 | (g as u32) << 8 | (r as u32);

            let result = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(34), // DWMWA_BORDER_COLOR
                &color as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            if result.is_ok() {
                log::trace!(
                    "Set native border color ({},{},{}) on window {:?}",
                    r,
                    g,
                    b,
                    handle
                );
                Ok(true)
            } else {
                log::trace!("DWMWA_BORDER_COLOR not supported (requires Windows 11 22000+)");
                Ok(false)
            }
        }
    }

    /// Remove the native DWM border color (restore system default)
    pub fn reset_native_border_color(&self, handle: WindowHandle) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(false);
            }

            // DWMWA_COLOR_DEFAULT = 0xFFFFFFFF
            let color: u32 = 0xFFFFFFFF;

            let result = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(34), // DWMWA_BORDER_COLOR
                &color as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            Ok(result.is_ok())
        }
    }

    /// Remove native DWM border entirely (no border visible)
    pub fn remove_native_border(&self, handle: WindowHandle) -> Result<bool> {
        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(false);
            }

            // DWMWA_COLOR_NONE = 0xFFFFFFFE
            let color: u32 = 0xFFFFFFFE;

            let result = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(34), // DWMWA_BORDER_COLOR
                &color as *const _ as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            Ok(result.is_ok())
        }
    }

    /// Set both native border color AND corner preference on a window (Windows 11+)
    /// Convenience method for full DWM border customization
    pub fn set_native_border_style(
        &self,
        handle: WindowHandle,
        r: u8,
        g: u8,
        b: u8,
        corner: WindowCorner,
    ) -> Result<bool> {
        let color_set = self.set_native_border_color(handle, r, g, b)?;

        unsafe {
            let hwnd = handle.to_hwnd();
            if !IsWindow(hwnd).as_bool() {
                return Ok(false);
            }

            let corner_value = corner as i32;
            let _ = super::reach::dwm_set_window_attribute(
                hwnd,
                DWMWINDOWATTRIBUTE(33), // DWMWA_WINDOW_CORNER_PREFERENCE
                &corner_value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );
        }

        Ok(color_set)
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Context for EnumWindows callback
/// Longest a responsiveness probe may wait for a window's thread.
const RESPONSIVE_PROBE_TIMEOUT_MS: u32 = 40;
/// How long a probe result is reused (animation frames re-check windows
/// many times per second).
const RESPONSIVE_PROBE_TTL: Duration = Duration::from_millis(250);

/// Last probe per window: when, and whether it did not answer.
type ResponsivenessCache = HashMap<isize, (Instant, bool)>;

thread_local! {
    static RESPONSIVENESS: RefCell<ResponsivenessCache> = RefCell::new(HashMap::new());
}

/// True when a call that waits on the window's thread could block the WM:
/// the thread does not answer a probe message within 40 ms.
///
/// `IsHungAppWindow` alone is not enough: Windows reports a window as hung
/// only after ~5 s without response, and a synchronous call made to an app
/// that stops responding inside that window waits until the app recovers,
/// possibly forever. A responsive thread answers the probe in microseconds;
/// results are cached briefly so animation frames do not probe every tick.
fn is_hung(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_NULL,
    };
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return false;
        }
        if IsHungAppWindow(hwnd).as_bool() {
            return true;
        }
    }
    // A window of a program with more rights takes no message from this
    // one, the one asked here among them: it would always read as hung.
    // What Windows itself says of it (above) is all there is to know.
    if crate::elevated::window_is_beyond_reach(hwnd.0 as isize) {
        return false;
    }
    let key = hwnd.0 as isize;
    let now = Instant::now();
    let cached = RESPONSIVENESS.with(|cache| {
        cache
            .borrow()
            .get(&key)
            .filter(|(at, _)| now.duration_since(*at) < RESPONSIVE_PROBE_TTL)
            .map(|(_, unresponsive)| *unresponsive)
    });
    if let Some(unresponsive) = cached {
        return unresponsive;
    }
    let answered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_NULL,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            RESPONSIVE_PROBE_TIMEOUT_MS,
            None,
        )
    }
    .0 != 0;
    let unresponsive = !answered && unsafe { IsWindow(hwnd) }.as_bool();
    RESPONSIVENESS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|_, (at, _)| now.duration_since(*at) < RESPONSIVE_PROBE_TTL);
        cache.insert(key, (now, unresponsive));
    });
    if unresponsive {
        log::trace!(
            "Window {:?} is not responding; using non-blocking calls",
            hwnd
        );
    }
    unresponsive
}

/// `SetWindowPos` flags for a managed window: a hung window gets
/// `SWP_ASYNCWINDOWPOS`, so the call is posted instead of blocking the WM loop
/// until the app recovers. Responsive windows stay synchronous, because layout
/// code reads rectangles back right after positioning.
fn nonblocking_swp(hwnd: HWND, flags: SET_WINDOW_POS_FLAGS) -> SET_WINDOW_POS_FLAGS {
    if move_must_be_posted(hwnd) {
        flags | SWP_ASYNCWINDOWPOS
    } else {
        flags
    }
}

/// How long after a move was sent to a window by post the next ones are
/// sent by post as well, when the window is not seen to have carried it
/// out (a program may put its window elsewhere than it was asked to).
const POSTED_MOVE_LIMIT: Duration = Duration::from_secs(2);

/// The last move sent to each window by post that it has not been seen
/// to carry out: its handle, the place asked for (the window's whole
/// rectangle: x, y, width, height), and when.
type PostedMoves = HashMap<isize, ((i32, i32, i32, i32), Instant)>;

thread_local! {
    static POSTED_MOVES: RefCell<PostedMoves> = RefCell::new(HashMap::new());
}

/// A move was sent to this window by post (`SWP_ASYNCWINDOWPOS`).
fn note_posted_move(hwnd: HWND, outer: (i32, i32, i32, i32)) {
    POSTED_MOVES.with(|moves| {
        moves
            .borrow_mut()
            .insert(hwnd.0 as isize, (outer, Instant::now()));
    });
}

/// Whether a move that was sent by post still stands between a window and
/// the next one: it was sent `since` ago, asked for `asked`, and the
/// window is at `now` (`None`: it is gone).
fn posted_move_stands(
    asked: (i32, i32, i32, i32),
    now: Option<(i32, i32, i32, i32)>,
    since: Duration,
) -> bool {
    now.is_some_and(|now| now != asked) && since < POSTED_MOVE_LIMIT
}

/// Whether the next move of this window has to be sent by post: the
/// window does not answer, or a move sent to it by post before may still
/// lie in its queue.
///
/// A move sent by post is carried out when the window's program gets to
/// it; one sent the plain way is carried out before every posted one that
/// still waits, however much later it was sent. A window of a program
/// that was busy for a moment, sent its first place by post and its next
/// ones the plain way, took the later places first and the first place
/// last, and stayed there: where its workspace's way in began, beside the
/// display. So the moves of one window are all sent the same way until
/// the posted one has been carried out: posted ones keep their order.
fn move_must_be_posted(hwnd: HWND) -> bool {
    if is_hung(hwnd) {
        return true;
    }
    let key = hwnd.0 as isize;
    let Some((asked, at)) = POSTED_MOVES.with(|moves| moves.borrow().get(&key).copied()) else {
        return false;
    };
    let mut rect = RECT::default();
    // SAFETY: plain questions about a window that may be gone.
    let now = unsafe {
        (IsWindow(hwnd).as_bool() && GetWindowRect(hwnd, &mut rect).is_ok()).then(|| {
            (
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            )
        })
    };
    if posted_move_stands(asked, now, at.elapsed()) {
        return true;
    }
    POSTED_MOVES.with(|moves| moves.borrow_mut().remove(&key));
    false
}

/// How a minimized window is brought back. Without the keyboard
/// (`SW_SHOWNOACTIVATE`), except one that was maximized before it was
/// minimized: that way of showing would bring it back at its plain size,
/// and Windows has no way to bring a window back maximized that leaves
/// the keyboard where it is.
fn restore_command(to_maximized: bool) -> SHOW_WINDOW_CMD {
    if to_maximized {
        SW_RESTORE
    } else {
        SW_SHOWNOACTIVATE
    }
}

/// `ShowWindow` that never blocks on a hung window. Kept synchronous for
/// responsive windows: a posted restore/maximize would be processed after the
/// layout's (sent) positioning and undo it.
fn show_window_nonblocking(hwnd: HWND, command: SHOW_WINDOW_CMD) {
    if is_hung(hwnd) {
        super::reach::show_window_async(hwnd, command);
    } else {
        super::reach::show_window(hwnd, command);
    }
}

struct EnumContext<'a> {
    windows: &'a mut HashMap<WindowHandle, WindowInfo>,
}

/// The window that takes the keyboard when `hwnd` is to have it: `hwnd`
/// itself, or, where it is disabled because a dialog of its own waits for
/// an answer, that dialog.
fn window_to_activate(hwnd: HWND) -> HWND {
    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    // SAFETY: plain questions about windows that may be gone meanwhile.
    unsafe {
        if IsWindowEnabled(hwnd).as_bool() {
            return hwnd;
        }
        let popup = GetLastActivePopup(hwnd);
        if popup.is_invalid() || popup == hwnd || !IsWindow(popup).as_bool() {
            return hwnd;
        }
        popup
    }
}

/// How far up from a window its owners are followed (a dialog's own
/// dialog's dialog): enough for any program, and an end to a ring of
/// owners should Windows ever hand one out.
const OWNER_DEPTH: usize = 8;

/// Whether a window of this class that belongs to another is hidden and
/// shown with it. None of gevrek's own is, and not the window a console
/// keeps for the programs that run in a terminal.
///
/// Every tab of Windows Terminal has such a window (nothing of it is
/// seen), and the console tells the terminal when it is hidden or shown:
/// the terminal then minimizes itself, or comes back. Hidden with its
/// terminal as a workspace went, each terminal of that workspace
/// minimized itself a moment later, and Windows gave the keyboard to
/// another window each time: the focus went to and fro on the workspace
/// that had come.
fn goes_with_its_owner(class: &str) -> bool {
    !class.starts_with("Gevrek_") && class != "PseudoConsoleWindow"
}

/// The top-level windows on the screen that belong to `owner`, directly or
/// through one another, lowest first. None of gevrek's own.
fn owned_windows_on_screen(owner: HWND) -> Vec<HWND> {
    struct Search {
        owner: HWND,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        // (Most windows belong to none: one question and on to the next.)
        let Some(mut above) = GetWindow(hwnd, GW_OWNER)
            .ok()
            .filter(|above| !above.is_invalid())
        else {
            return true.into();
        };
        if hwnd == search.owner || !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        for _ in 0..OWNER_DEPTH {
            if above == search.owner {
                if goes_with_its_owner(&read_window_class(hwnd)) {
                    search.found.push(hwnd);
                }
                break;
            }
            match GetWindow(above, GW_OWNER) {
                Ok(next) if !next.is_invalid() => above = next,
                _ => break,
            }
        }
        true.into()
    }
    let mut search = Search {
        owner,
        found: Vec::new(),
    };
    // SAFETY: the callback runs before the call returns and gets a pointer
    // to `search`, which outlives it.
    unsafe {
        if !IsWindow(owner).as_bool() {
            return Vec::new();
        }
        let _ = EnumWindows(Some(each), LPARAM(&mut search as *mut Search as isize));
    }
    // (EnumWindows goes from the top down.)
    search.found.reverse();
    search.found
}

/// The windows above a window in its chain of owners, nearest first.
fn owners_of(window: HWND) -> Vec<HWND> {
    let mut owners = Vec::new();
    let mut above = window;
    for _ in 0..OWNER_DEPTH {
        // SAFETY: a plain question; a window that is gone has no owner.
        match unsafe { GetWindow(above, GW_OWNER) } {
            Ok(next) if !next.is_invalid() && next != window && !owners.contains(&next) => {
                owners.push(next);
                above = next;
            }
            _ => break,
        }
    }
    owners
}

/// Every top-level window made by the part of its program that made
/// `window` (its thread), shown or not. None for the program that is the
/// desktop itself.
fn thread_windows(window: HWND) -> Vec<HWND> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam.0 as *mut Vec<HWND>);
        found.push(hwnd);
        true.into()
    }
    let mut found: Vec<HWND> = Vec::new();
    // SAFETY: the callback runs before the call returns and gets a pointer
    // to `found`, which outlives it; the rest are plain questions.
    unsafe {
        let thread = GetWindowThreadProcessId(window, None);
        let shell = GetShellWindow();
        if thread == 0
            || (!shell.is_invalid() && GetWindowThreadProcessId(shell, None) == thread)
        {
            return Vec::new();
        }
        let _ = EnumThreadWindows(
            thread,
            Some(each),
            LPARAM(&mut found as *mut Vec<HWND> as isize),
        );
    }
    found
}

/// What is known of the windows of gevrek's that the program of a window
/// has (those made by the same part of it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Program {
    /// One of them is not hidden by gevrek: the program is on the screen.
    in_sight: bool,
    /// One that gevrek has hidden.
    hidden: Option<WindowHandle>,
}

/// A window that was shown just now, as far as it matters for whether it
/// is hidden with something gevrek has hidden (`hide_late_arrival`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LateArrival {
    /// gevrek has it as hidden already (it was hidden with its owner a
    /// moment ago, and this is word of an earlier showing).
    already_hidden: bool,
    /// One of gevrek's own, or of the classes menus and tooltips are made
    /// of: gone in a moment by itself, and often kept and shown again by
    /// its program, so that one hidden now would come back as a tooltip
    /// of long ago.
    passes_in_a_moment: bool,
    /// A window gevrek has in a workspace or is about to (the engine says
    /// where it goes: `join_parent_out_of_sight`).
    gevreks_own: bool,
    kind: crate::window_kind::Kind,
    /// It has a size.
    sized: bool,
}

impl LateArrival {
    /// The window it is hidden with, if any. `owners` are the windows
    /// above it, nearest first, each with whether gevrek has it hidden;
    /// `program` is asked only where the window belongs to none.
    ///
    /// What belongs to a window goes with that window: hidden where the
    /// nearest window above it that gevrek has hidden is found, whatever
    /// else of the program is in sight (a dialog of the editor that is put
    /// away, while another window of the same program is worked in). A
    /// window that was shown again with its owner a moment ago is not
    /// touched: its owner is not hidden. What belongs to no window (a
    /// notification the program draws itself) goes only when the program
    /// has a window gevrek has hidden and none in sight.
    fn goes_with(
        &self,
        owners: &[(WindowHandle, bool)],
        program: impl FnOnce() -> Program,
    ) -> Option<WindowHandle> {
        use crate::window_kind::Kind;
        if self.already_hidden || self.passes_in_a_moment || self.gevreks_own {
            return None;
        }
        if !owners.is_empty() {
            return owners
                .iter()
                .find(|(_, hidden)| *hidden)
                .map(|(owner, _)| *owner);
        }
        if self.kind != Kind::Passing || !self.sized {
            return None;
        }
        let program = program();
        if program.in_sight {
            return None;
        }
        program.hidden
    }
}

#[cfg(test)]
mod late_arrival_tests {
    use super::{LateArrival, Program, WindowHandle};
    use crate::window_kind::Kind;

    const EDITOR: WindowHandle = WindowHandle(1);
    const FIND: WindowHandle = WindowHandle(2);

    fn arrival(kind: Kind) -> LateArrival {
        LateArrival {
            already_hidden: false,
            passes_in_a_moment: false,
            gevreks_own: false,
            kind,
            sized: true,
        }
    }

    fn nobody() -> Program {
        panic!("the program is not asked about where a window has an owner")
    }

    #[test]
    fn a_dialog_goes_with_the_hidden_window_it_belongs_to() {
        let dialog = arrival(Kind::Owned);
        assert_eq!(dialog.goes_with(&[(EDITOR, true)], nobody), Some(EDITOR));
        // A dialog's own dialog: with the nearest window above that is hidden.
        assert_eq!(
            dialog.goes_with(&[(FIND, false), (EDITOR, true)], nobody),
            Some(EDITOR)
        );
        assert_eq!(
            dialog.goes_with(&[(FIND, true), (EDITOR, true)], nobody),
            Some(FIND)
        );
        // A palette that is kept out of the taskbar and has an owner too.
        assert_eq!(
            arrival(Kind::Passing).goes_with(&[(EDITOR, true)], nobody),
            Some(EDITOR)
        );
    }

    #[test]
    fn what_belongs_to_a_window_in_sight_stays() {
        let dialog = arrival(Kind::Owned);
        // Its owner is on the screen: also the moment after gevrek showed
        // both again.
        assert_eq!(dialog.goes_with(&[(EDITOR, false)], nobody), None);
        assert_eq!(
            dialog.goes_with(&[(FIND, false), (EDITOR, false)], nobody),
            None
        );
    }

    #[test]
    fn what_gevrek_hid_already_or_has_as_its_own_or_passes_is_left() {
        let hidden = [(EDITOR, true)];
        let again = LateArrival {
            already_hidden: true,
            ..arrival(Kind::Owned)
        };
        assert_eq!(again.goes_with(&hidden, nobody), None);
        let own = LateArrival {
            gevreks_own: true,
            ..arrival(Kind::Ordinary)
        };
        assert_eq!(own.goes_with(&hidden, nobody), None);
        let menu = LateArrival {
            passes_in_a_moment: true,
            ..arrival(Kind::Passing)
        };
        assert_eq!(menu.goes_with(&hidden, nobody), None);
    }

    #[test]
    fn a_notification_goes_only_when_all_of_its_program_is_hidden() {
        let notice = arrival(Kind::Passing);
        let away = || Program {
            in_sight: false,
            hidden: Some(EDITOR),
        };
        assert_eq!(notice.goes_with(&[], away), Some(EDITOR));
        // Another window of the program is in sight: it stays.
        let here = || Program {
            in_sight: true,
            hidden: Some(EDITOR),
        };
        assert_eq!(notice.goes_with(&[], here), None);
        // The program has no window of gevrek's at all.
        assert_eq!(notice.goes_with(&[], Program::default), None);
        // A window of its own that belongs to none is the engine's, or
        // nobody's: never hidden here.
        for kind in [Kind::Ordinary, Kind::Beside, Kind::Owned] {
            assert_eq!(arrival(kind).goes_with(&[], away), None, "{kind:?}");
        }
        // Nor one without a size.
        let empty = LateArrival {
            sized: false,
            ..notice
        };
        assert_eq!(empty.goes_with(&[], away), None);
    }
}

/// What a program shows beside its windows that belongs to none of them
/// and is no window to gevrek (`window_kind::Kind::Passing`): a
/// notification it draws itself, a menu, a window it keeps for its own
/// ends. Only those made by the same part of the program as `window` (its
/// thread: Windows' own taskbar and desktop are made by another part of
/// the program that shows the folders), and only when `in_sight` says of
/// no other window made by it that it is a window of gevrek's still to be
/// seen: then the program has left the screen, and these go with it.
///
/// Not what is made of the classes of the system's menus and tooltips
/// (`window_kind::is_a_popups_class`): such a window is gone in a moment
/// by itself, and many programs keep theirs and only hide it, so that one
/// shown again later would be a tooltip of long ago.
fn companions_on_screen(window: HWND, in_sight: impl Fn(HWND) -> bool) -> Vec<HWND> {
    struct Search {
        window: HWND,
        others: Vec<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if hwnd != search.window && IsWindowVisible(hwnd).as_bool() {
            search.others.push(hwnd);
        }
        true.into()
    }
    let mut search = Search {
        window,
        others: Vec::new(),
    };
    // SAFETY: the callback runs before the call returns and gets a pointer
    // to `search`, which outlives it; the rest are plain questions about
    // windows that may be gone.
    unsafe {
        if !IsWindow(window).as_bool() {
            return Vec::new();
        }
        let thread = GetWindowThreadProcessId(window, None);
        // (Nothing of the program that is the desktop itself.)
        let shell = GetShellWindow();
        if thread == 0
            || (!shell.is_invalid() && GetWindowThreadProcessId(shell, None) == thread)
        {
            return Vec::new();
        }
        let _ = EnumThreadWindows(
            thread,
            Some(each),
            LPARAM(&mut search as *mut Search as isize),
        );
    }
    if search.others.iter().any(|other| in_sight(*other)) {
        return Vec::new();
    }
    search
        .others
        .into_iter()
        .filter(|hwnd| {
            // SAFETY: plain questions about a window that may be gone.
            let (owned, sized) = unsafe {
                let mut rect = RECT::default();
                (
                    GetWindow(*hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()),
                    GetWindowRect(*hwnd, &mut rect).is_ok()
                        && rect.right > rect.left
                        && rect.bottom > rect.top,
                )
            };
            let class = read_window_class(*hwnd);
            // (What belongs to a window goes with that window.)
            !owned
                && sized
                && !class.starts_with("Gevrek_")
                && !crate::window_kind::is_a_popups_class(&class)
                && crate::window_kind::of_window(hwnd.0 as isize)
                    == crate::window_kind::Kind::Passing
        })
        .collect()
}

/// Whether the panel of this instance's shell (`gevrek-shell`) is on the
/// screen. Its surfaces are left out of everything the window manager
/// lists (their class begins with `Gevrek_`), so it is looked for by its
/// class: `Gevrek_Shell_` and the surface's name
/// (`gevrek_shellkit::win::place::class_of`), which has the instance in it.
pub fn shell_panel_is_shown() -> bool {
    let class: Vec<u16> = format!(
        "Gevrek_Shell_{}",
        gevrek_model::paths::panel_surface(&gevrek_model::paths::instance())
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect();
    unsafe {
        let mut after = HWND::default();
        // One for each display; any that is shown counts.
        while let Ok(found) =
            FindWindowExW(None, after, windows::core::PCWSTR(class.as_ptr()), None)
        {
            if found.is_invalid() {
                break;
            }
            if IsWindowVisible(found).as_bool() {
                return true;
            }
            after = found;
        }
    }
    false
}

/// Top-level frame class of UWP apps.
const UWP_FRAME_CLASS: &str = "ApplicationFrameWindow";

/// Process id of the app hosted by a UWP frame: the owner of its CoreWindow
/// child, when that is a different process than the frame host.
fn uwp_app_pid(frame: HWND) -> Option<u32> {
    let core = unsafe {
        FindWindowExW(
            frame,
            None,
            windows::core::w!("Windows.UI.Core.CoreWindow"),
            None,
        )
    }
    .ok()
    .filter(|core| !core.is_invalid())?;
    let mut frame_pid = 0;
    let mut app_pid = 0;
    unsafe {
        GetWindowThreadProcessId(frame, Some(&mut frame_pid));
        GetWindowThreadProcessId(core, Some(&mut app_pid));
    }
    (app_pid != 0 && app_pid != frame_pid).then_some(app_pid)
}

/// Window class name, read live.
fn read_window_class(hwnd: HWND) -> String {
    let mut buf: [u16; 256] = [0; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Window title, read live. For another process's window this reads the
/// stored caption without sending it a message, so it cannot block.
fn read_window_title(hwnd: HWND) -> String {
    let mut buf: [u16; 512] = [0; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Full executable path of a process (empty when access is denied).
fn process_image_path(pid: u32) -> String {
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut path_buf: [u16; 1024] = [0; 1024];
        let mut size = path_buf.len() as u32;
        let path = if QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path_buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok()
        {
            String::from_utf16_lossy(&path_buf[..size as usize])
        } else {
            String::new()
        };
        let _ = windows::Win32::Foundation::CloseHandle(process);
        path
    }
}

impl WindowManager {
    /// Rule-matching facts for a window: cached identity plus cheap live
    /// state. `None` only for a window that is gone.
    ///
    /// None of it is asked of the window's program (Windows keeps all of
    /// it itself), so it is read of a window whose program does not
    /// answer at the moment too. It used not to be: such a window had no
    /// facts, no rule spoke of it, and it was taken for a program's own
    /// window. A program is busy for a moment just when it opens a menu:
    /// the menu was then given a tile's place and a border.
    pub fn rule_window_info(&self, handle: WindowHandle) -> Option<RuleWindowInfo> {
        let hwnd = handle.to_hwnd();
        // SAFETY: a plain question.
        if !unsafe { IsWindow(hwnd) }.as_bool() {
            return None;
        }
        let identity = self.window_identity(handle);
        let exe = identity
            .path
            .rsplit('\\')
            .next()
            .unwrap_or_default()
            .to_string();
        unsafe {
            let parent = GetParent(hwnd);
            let has_parent = parent.is_ok_and(|parent| !parent.is_invalid());
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            const WS_EX_TOOLWINDOW: i32 = 0x00000080;
            const WS_EX_NOACTIVATE: i32 = 0x08000000;
            const WS_EX_LAYERED: i32 = 0x00080000;
            let owner = GetWindow(hwnd, GW_OWNER);
            let has_owner = owner.is_ok_and(|owner| !owner.is_invalid());
            let mut cloaked: u32 = 0;
            let is_cloaked = DwmGetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(14), // DWMWA_CLOAKED
                &mut cloaked as *mut _ as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
                // (Not one gevrek keeps from sight until it is placed.)
                && !super::veil::holds(hwnd);

            Some(RuleWindowInfo {
                handle,
                title: read_window_title(hwnd),
                class: identity.class,
                app_id: gevrek_engine::contract::app_id_of_exe(&exe),
                exe,
                path: identity.path,
                has_parent,
                is_visible: IsWindowVisible(hwnd).as_bool(),
                is_tool_window: (ex_style & WS_EX_TOOLWINDOW) != 0,
                is_noactivate: (ex_style & WS_EX_NOACTIVATE) != 0,
                has_owner,
                is_cloaked,
                is_layered: (ex_style & WS_EX_LAYERED) != 0,
                tags: BTreeSet::new(),
                workspace: None,
                workspace_index: None,
            })
        }
    }
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = &mut *(lparam.0 as *mut EnumContext);
    if let Some(info) = eligible_window_info(hwnd) {
        if info.is_minimized {
            log::debug!(
                "📦 Found minimized window: '{}' ({:?})",
                info.title,
                info.handle
            );
        }
        context.windows.insert(info.handle, info);
    }
    true.into()
}

/// The registry's admission filter, shared by the startup enumeration and
/// incremental tracking so both agree on which windows are tracked.
/// Minimized windows are included (so they can be restored); invisible,
/// tool, no-activate, cloaked, owned, untitled, and Gevrek windows are not.
unsafe fn eligible_window_info(hwnd: HWND) -> Option<WindowInfo> {
    // CRITICAL: Skip our own border windows FIRST (before any other checks)
    {
        let mut class_name: [u16; 256] = [0; 256];
        let class_len = GetClassNameW(hwnd, &mut class_name);
        if class_len > 0 {
            let class_str = String::from_utf16_lossy(&class_name[0..class_len as usize]);
            if class_str.starts_with("Gevrek_") {
                return None; // Skip all Gevrek windows
            }
        }
    }

    // Skip invisible windows (but NOT minimized windows - we want to restore those)
    // IsIconic returns true for minimized windows, IsWindowVisible returns false for them
    // We want to include minimized windows so we can restore them
    let is_minimized = IsIconic(hwnd).as_bool();
    if !IsWindowVisible(hwnd).as_bool() && !is_minimized {
        return None;
    }

    // Skip windows with WS_EX_TOOLWINDOW or WS_EX_NOACTIVATE (toolbars, tooltips, etc.)
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    const WS_EX_TOOLWINDOW: i32 = 0x00000080;
    const WS_EX_NOACTIVATE: i32 = 0x08000000;
    if (ex_style & WS_EX_TOOLWINDOW) != 0 || (ex_style & WS_EX_NOACTIVATE) != 0 {
        return None;
    }

    // Skip cloaked windows (windows on other virtual desktops)
    let mut cloaked: u32 = 0;
    let is_cloaked = DwmGetWindowAttribute(
        hwnd,
        DWMWINDOWATTRIBUTE(14), // DWMWA_CLOAKED
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    )
    .is_ok()
        && cloaked != 0
        // (Not one gevrek keeps from sight until it is placed: `veil`.)
        && !super::veil::holds(hwnd);
    if is_cloaked {
        return None;
    }

    // Skip windows with owners (dialog boxes, popups that shouldn't be tiled separately)
    let owner = GetWindow(hwnd, GW_OWNER);
    if owner.is_ok() && !owner.unwrap().is_invalid() {
        return None;
    }

    // Skip windows with no title (system windows, etc.)
    let mut text_len = GetWindowTextLengthW(hwnd);
    if text_len == 0 {
        return None;
    }

    text_len += 1;
    let mut text: Vec<u16> = vec![0; text_len as usize];
    GetWindowTextW(hwnd, &mut text);

    let title = String::from_utf16_lossy(&text)
        .trim_end_matches('\0')
        .to_string();

    // Skip windows with very short titles (likely system windows)
    if title.len() < 2 {
        return None;
    }

    let mut rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut rect);

    // Skip windows with zero or negative size (but NOT minimized windows)
    // Minimized windows often have weird rect values, so we skip this check for them
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if !is_minimized && (width <= 0 || height <= 0) {
        return None;
    }

    let handle = WindowHandle::from_hwnd(hwnd);

    Some(WindowInfo {
        handle,
        title,
        x: rect.left,
        y: rect.top,
        width: if is_minimized { 800 } else { width }, // Use default size for minimized
        height: if is_minimized { 600 } else { height },
        is_visible: !is_minimized, // Minimized windows are not visible
        is_minimized,              // Use the actual minimized state
        is_maximized: IsZoomed(hwnd).as_bool(),
    })
}

unsafe extern "system" fn border_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            // Get window dimensions
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_err() {
                let _ = ValidateRect(hwnd, None);
                return LRESULT(0);
            }

            // Begin paint
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            if !hdc.is_invalid() {
                // Get border color from window user data
                let color_ref = GetWindowLongW(hwnd, GWLP_USERDATA) as u32;
                let color = if color_ref == 0 {
                    COLORREF(0x000000FF) // Default red (debug)
                } else {
                    COLORREF(color_ref)
                };

                // Fill the ENTIRE window with the border color.
                // The border window sits behind the target window in z-order,
                // so the interior (behind target) is hidden. Only edges extending
                // past the target window are visible = the border effect.
                // During resize, any temporarily exposed interior shows the border
                // color (not black), which is much less jarring.
                let brush = CreateSolidBrush(color);
                FillRect(hdc, &rect, brush);
                let _ = DeleteObject(brush);

                let _ = EndPaint(hwnd, &ps);
            }

            LRESULT(0)
        }
        WM_ERASEBKGND => {
            // Fill background with border color IMMEDIATELY on resize.
            // This runs before WM_PAINT and prevents black flash in newly exposed areas.
            let hdc = HDC(wparam.0 as *mut _);
            if !hdc.is_invalid() {
                let color_ref = GetWindowLongW(hwnd, GWLP_USERDATA) as u32;
                let color = if color_ref == 0 {
                    COLORREF(0x000000FF)
                } else {
                    COLORREF(color_ref)
                };
                let mut rect = RECT::default();
                if GetClientRect(hwnd, &mut rect).is_ok() {
                    let brush = CreateSolidBrush(color);
                    FillRect(hdc, &rect, brush);
                    let _ = DeleteObject(brush);
                }
            }
            LRESULT(1)
        }
        WM_DESTROY => {
            // Clean up when border window is destroyed
            SetWindowLongW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Window procedure for stackbar tab strips. Paint/click state lives in a
/// heap-allocated [`StackbarWndData`] whose pointer is stored in
/// `GWLP_USERDATA` by `create_stackbar` and reclaimed here on `WM_DESTROY`.
unsafe extern "system" fn stackbar_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StackbarWndData;
            if data_ptr.is_null() {
                let _ = ValidateRect(hwnd, None);
                return LRESULT(0);
            }
            let data = &*data_ptr;

            let mut client = RECT::default();
            if GetClientRect(hwnd, &mut client).is_err() {
                let _ = ValidateRect(hwnd, None);
                return LRESULT(0);
            }

            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                // Bar background.
                let brush = CreateSolidBrush(data.background);
                FillRect(hdc, &client, brush);
                let _ = DeleteObject(brush);

                let width = client.right - client.left;
                // (The tabs are as high as the strip above the window; what
                // of the strip's own window lies lower is behind the
                // program's window, and only its colour is seen there.)
                let height = data.bar_height.min(client.bottom - client.top);
                let tabs = stackbar_tab_rects(width, data.titles.len());

                let old_font = if data.font.is_invalid() {
                    HGDIOBJ::default()
                } else {
                    SelectObject(hdc, data.font)
                };
                SetBkMode(hdc, TRANSPARENT);

                for (index, ((tab_x, tab_width), title)) in
                    tabs.iter().zip(data.titles.iter()).enumerate()
                {
                    let focused = index == data.focused_tab;
                    SetTextColor(
                        hdc,
                        if focused {
                            data.active_text
                        } else {
                            data.inactive_text
                        },
                    );
                    let mut tab_rect = RECT {
                        left: *tab_x,
                        top: 0,
                        right: tab_x + tab_width,
                        bottom: height,
                    };
                    let mut title_wide: Vec<u16> =
                        title.encode_utf16().chain(std::iter::once(0)).collect();
                    DrawTextW(
                        hdc,
                        &mut title_wide,
                        &mut tab_rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                    );
                    if focused {
                        // The tab of the window that is shown is marked in
                        // that window's border colour.
                        let accent = CreateSolidBrush(data.mark);
                        let underline = RECT {
                            left: *tab_x,
                            top: height - 3,
                            right: tab_x + tab_width,
                            bottom: height,
                        };
                        FillRect(hdc, &underline, accent);
                        let _ = DeleteObject(accent);
                    }
                }

                if !data.font.is_invalid() {
                    SelectObject(hdc, old_font);
                }
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StackbarWndData;
            if !data_ptr.is_null() {
                let hdc = HDC(wparam.0 as *mut _);
                if !hdc.is_invalid() {
                    let mut rect = RECT::default();
                    if GetClientRect(hwnd, &mut rect).is_ok() {
                        let brush = CreateSolidBrush((*data_ptr).background);
                        FillRect(hdc, &rect, brush);
                        let _ = DeleteObject(brush);
                    }
                }
            }
            LRESULT(1)
        }
        WM_LBUTTONUP => {
            // Map the click x-coordinate to a tab and queue a focus request;
            // the runtime loop drains the queue and dispatches the group
            // focus path (same as FocusGroupIndex).
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StackbarWndData;
            if !data_ptr.is_null() {
                let data = &*data_ptr;
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let mut client = RECT::default();
                if GetClientRect(hwnd, &mut client).is_ok() {
                    if let Some(index) =
                        stackbar_tab_index_at(client.right - client.left, data.titles.len(), x)
                    {
                        if let Ok(mut queue) = data.click_queue.lock() {
                            queue.push((data.key, index));
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StackbarWndData;
            if !data_ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let data = Box::from_raw(data_ptr);
                if !data.font.is_invalid() {
                    let _ = DeleteObject(data.font);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod alpha_compatibility_tests {
    use super::{window_alpha_action, WindowAlphaAction};

    #[test]
    fn opaque_windows_always_use_native_composition() {
        assert_eq!(
            window_alpha_action(255, false, false),
            WindowAlphaAction::RestoreNative
        );
        assert_eq!(
            window_alpha_action(255, true, true),
            WindowAlphaAction::RestoreNative
        );
    }

    #[test]
    fn application_owned_layered_windows_are_never_reconfigured() {
        assert_eq!(
            window_alpha_action(180, true, false),
            WindowAlphaAction::SkipApplicationOwnedLayered
        );
        assert_eq!(
            window_alpha_action(180, true, true),
            WindowAlphaAction::ApplyLayered
        );
        assert_eq!(
            window_alpha_action(180, false, false),
            WindowAlphaAction::ApplyLayered
        );
    }
}

#[cfg(test)]
mod restore_tests {
    use super::{goes_with_its_owner, restore_command};
    use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, SW_SHOWNOACTIVATE};

    #[test]
    fn a_minimized_window_is_brought_back_without_the_keyboard() {
        assert_eq!(restore_command(false), SW_SHOWNOACTIVATE);
    }

    #[test]
    fn the_window_a_console_keeps_is_not_hidden_with_its_terminal() {
        assert!(!goes_with_its_owner("PseudoConsoleWindow"));
        assert!(!goes_with_its_owner("Gevrek_Border"));
        assert!(goes_with_its_owner("#32770"), "a dialog goes with its window");
    }

    #[test]
    fn one_that_was_maximized_comes_back_maximized() {
        assert_eq!(restore_command(true), SW_RESTORE);
    }
}

#[cfg(test)]
mod border_geometry_tests {
    use super::{border_rect_around_frame, border_rect_for_window, visible_frame_rect};

    #[test]
    fn visible_frame_subtracts_frame_insets() {
        // Window rect (100, 100)-(500, 400) with 7px invisible margins
        // left/right and 0px top/bottom (typical Windows 10/11 values).
        let frame = visible_frame_rect((100, 100, 500, 400), (7, 0, 7, 0));
        assert_eq!(frame, (107, 100, 386, 300));
    }

    #[test]
    fn visible_frame_with_zero_insets_is_window_rect() {
        let frame = visible_frame_rect((10, 20, 310, 220), (0, 0, 0, 0));
        assert_eq!(frame, (10, 20, 300, 200));
    }

    #[test]
    fn border_rect_grows_around_frame() {
        // 4px border with 2px outward offset around a 300x200 frame at (10, 20).
        let rect = border_rect_around_frame((10, 20, 300, 200), 4, 2);
        assert_eq!(rect, (4, 14, 312, 212));
    }

    #[test]
    fn border_rect_with_negative_offset_hugs_window_edge() {
        // Negative offset pulls the border back over the window edge; the
        // visible band stays border_width wide.
        let rect = border_rect_around_frame((10, 20, 300, 200), 4, -2);
        assert_eq!(rect, (8, 18, 304, 204));
    }

    #[test]
    fn border_rect_with_zero_width_collapses_to_frame() {
        let rect = border_rect_around_frame((10, 20, 300, 200), 0, 0);
        assert_eq!(rect, (10, 20, 300, 200));
    }

    #[test]
    fn border_rect_with_zero_width_and_offset_expands_by_offset() {
        let rect = border_rect_around_frame((10, 20, 300, 200), 0, 5);
        assert_eq!(rect, (5, 15, 310, 210));
    }

    #[test]
    fn border_rect_for_window_combines_insets_and_growth() {
        // Same inputs as the live GetWindowRect path: insets shrink to the
        // visible frame first, then the border grows around it.
        let rect = border_rect_for_window((100, 100, 500, 400), (7, 0, 7, 0), 4, -1);
        // visible frame = (107, 100, 386, 300); grow = 4 + (-1) = 3
        assert_eq!(rect, (104, 97, 392, 306));
    }

    #[test]
    fn explicit_and_live_paths_agree_for_same_frame() {
        // The explicit path passes the layout rect (already the visible
        // frame); the live path derives the frame from GetWindowRect +
        // insets. Given consistent inputs both must produce the same border.
        let layout_rect = (107, 100, 386, 300);
        let explicit = border_rect_around_frame(layout_rect, 4, -1);
        let live = border_rect_for_window((100, 100, 500, 400), (7, 0, 7, 0), 4, -1);
        assert_eq!(explicit, live);
    }
}

#[cfg(test)]
mod stackbar_geometry_tests {
    use super::{
        invisible_border_content_rect, stackbar_content_rect, stackbar_tab_index_at,
        stackbar_tab_rects,
    };

    #[test]
    fn a_workspace_slides_in_from_a_side_where_no_other_display_is() {
        let left = (0, 0, 2560, 1440);
        let right = (2560, 0, 2560, 1440);
        let window_on_left = [(20, 60, 1200, 1300)];
        let window_on_right = [(2580, 60, 1200, 1300)];
        // Two displays side by side. From the right, on the left display:
        // that is where the other display is; from the left it is free.
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right],
                &window_on_left,
                (2560, 0)
            ),
            Some((-2560, 0))
        );
        // From the left, on the left display: free, as asked.
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right],
                &window_on_left,
                (-2560, 0)
            ),
            Some((-2560, 0))
        );
        // On the right display it is the other way round.
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right],
                &window_on_right,
                (-2560, 0)
            ),
            Some((2560, 0))
        );
        // One display: as asked.
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left],
                &window_on_left,
                (2560, 0)
            ),
            Some((2560, 0))
        );
        // A display on either side: from below (or above), by its height.
        let middle = [(2580, 60, 1200, 1300)];
        let third = (5120, 0, 2560, 1440);
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right, third],
                &middle,
                (2560, 0)
            ),
            Some((0, 1440))
        );
        // And one below as well: from above.
        let below = (2560, 1440, 2560, 1440);
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right, third, below],
                &middle,
                (2560, 0)
            ),
            Some((0, -1440))
        );
        // A display on every side: it does not slide.
        let above = (2560, -1440, 2560, 1440);
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(
                &[left, right, third, below, above],
                &middle,
                (2560, 0)
            ),
            None
        );
        assert_eq!(
            crate::dwm::window_manager::slide_clear_of_displays(&[left], &[], (2560, 0)),
            None
        );
    }

    #[test]
    fn invisible_borders_reserve_a_band_on_every_edge() {
        // Window rect (10, 20, 400, 300) with an 8px invisible border: the
        // content shrinks by 8px on every side and nothing is drawn.
        let content = invisible_border_content_rect((10, 20, 400, 300), 8);
        assert_eq!(content, (18, 28, 384, 284));
    }

    #[test]
    fn invisible_border_reservation_is_clamped_to_the_window_size() {
        // A band larger than half the window must not produce a negative or
        // zero content rect; it degrades to a 1px-wide content.
        let content = invisible_border_content_rect((0, 0, 10, 300), 40);
        assert_eq!(content, (4, 40, 2, 220));
        assert_eq!(
            invisible_border_content_rect((0, 0, 100, 1), 40),
            (40, 0, 20, 1)
        );
    }

    #[test]
    fn invisible_borders_without_size_reserve_nothing() {
        assert_eq!(
            invisible_border_content_rect((10, 20, 400, 300), 0),
            (10, 20, 400, 300)
        );
        assert_eq!(
            invisible_border_content_rect((10, 20, 400, 300), -5),
            (10, 20, 400, 300)
        );
    }

    #[test]
    fn a_workspace_gives_way_as_its_style_says_where_the_displays_allow() {
        use gevrek_model::settings::WorkspaceTransitionStyle as Style;
        use super::super::window_manager::{workspace_ways, WorkspaceWays};
        let one = [(0, 0, 1920, 1080)];
        let incoming = [(10, 10, 900, 1000)];
        let outgoing = [(10, 10, 1900, 1000)];
        // One display, sideways: the new one comes from the right, the
        // old one goes out to the left.
        assert_eq!(
            workspace_ways(Style::Slide, &one, &incoming, &outgoing, (1920, 0)),
            WorkspaceWays {
                come: Some((1920, 0)),
                go: Some((-1920, 0)),
                fade: false
            }
        );
        // Up and down: by the display's height, the way the offset points.
        assert_eq!(
            workspace_ways(Style::SlideVertical, &one, &incoming, &outgoing, (-1920, 0)),
            WorkspaceWays {
                come: Some((0, -1080)),
                go: Some((0, 1080)),
                fade: false
            }
        );
        // A fade moves nothing.
        assert_eq!(
            workspace_ways(Style::Fade, &one, &incoming, &outgoing, (1920, 0)),
            WorkspaceWays {
                come: None,
                go: None,
                fade: true
            }
        );
        // A display to the right: the new one cannot come from there, so
        // it comes from the left; the old one would cross the other
        // display on its way out to the right, and goes out downwards.
        // (It was hidden at once before: beside another display the old
        // workspace never moved, and a change to a workspace with no
        // window on it showed nothing at all.)
        let two = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        assert_eq!(
            workspace_ways(Style::Slide, &two, &incoming, &outgoing, (1920, 0)),
            WorkspaceWays {
                come: Some((-1920, 0)),
                go: Some((0, 1080)),
                fade: false
            }
        );
        // Asked the other way: the same, for the same reason.
        assert_eq!(
            workspace_ways(Style::Slide, &two, &incoming, &outgoing, (-1920, 0)),
            WorkspaceWays {
                come: Some((-1920, 0)),
                go: Some((0, 1080)),
                fade: false
            }
        );
        // To a workspace without windows, beside another display: the old
        // one still goes (out the way that is clear, or else downwards).
        assert!(workspace_ways(Style::Slide, &two, &[], &outgoing, (1920, 0)).go.is_some());
        assert!(workspace_ways(Style::Slide, &two, &[], &outgoing, (-1920, 0)).go.is_some());
        // A display on every side but above: upwards.
        let around = [
            (0, 0, 1920, 1080),
            (1920, 0, 1920, 1080),
            (-1920, 0, 1920, 1080),
            (0, 1080, 1920, 1080),
        ];
        assert_eq!(
            workspace_ways(Style::Slide, &around, &[], &outgoing, (1920, 0)).go,
            Some((0, -1080))
        );
        // To a workspace without windows: nothing comes, the old one goes.
        assert_eq!(
            workspace_ways(Style::Slide, &one, &[], &outgoing, (1920, 0)),
            WorkspaceWays {
                come: None,
                go: Some((-1920, 0)),
                fade: false
            }
        );
        // From one without: nothing goes.
        assert_eq!(
            workspace_ways(Style::Slide, &one, &incoming, &[], (1920, 0)).go,
            None
        );
    }

    #[test]
    fn a_window_is_cut_to_the_part_of_it_on_its_display() {
        use super::super::window_manager::part_on_display;
        let display = (0, 0, 2560, 1440);
        // Wholly on it: no shape.
        assert_eq!(part_on_display((100, 100, 900, 800), display), None);
        // Past the right edge by 300: its left 500 are kept.
        assert_eq!(
            part_on_display((2060, 100, 2860, 900), display),
            Some((0, 0, 500, 800))
        );
        // Past the left edge by 200: what is right of that is kept.
        assert_eq!(
            part_on_display((-200, 50, 600, 850), display),
            Some((200, 0, 800, 800))
        );
        // On a display that does not begin at nothing.
        assert_eq!(
            part_on_display((2400, 0, 3200, 700), (2560, 0, 5120, 1440)),
            Some((160, 0, 800, 700))
        );
        // Wholly outside: nothing of it is kept.
        assert_eq!(
            part_on_display((3000, 0, 3800, 700), display),
            Some((0, 0, 0, 700))
        );
    }

    #[test]
    fn columns_are_whole_only_where_another_display_lies_beside() {
        use super::super::window_manager::columns_must_be_whole;
        let one = (0, 0, 2560, 1440);
        assert!(!columns_must_be_whole(&[]));
        assert!(!columns_must_be_whole(&[one]));
        // Beside each other, also partly.
        assert!(columns_must_be_whole(&[one, (2560, 0, 2560, 1440)]));
        assert!(columns_must_be_whole(&[one, (-1920, 900, 1920, 1080)]));
        // One above the other: a column past the side is on neither.
        assert!(!columns_must_be_whole(&[one, (0, -1440, 2560, 1440)]));
        assert!(!columns_must_be_whole(&[one, (300, 1440, 1920, 1080)]));
        // Three, two of them beside each other.
        assert!(columns_must_be_whole(&[
            one,
            (0, 1440, 2560, 1440),
            (2560, 1440, 1920, 1080)
        ]));
    }

    #[test]
    fn a_windows_frame_goes_around_its_strip_of_tabs() {
        use super::super::window_manager::{border_rect_with_strip, stackbar_overlap};
        // A border at y 96, 408 high, around a window with a strip of 28:
        // it begins that much higher and is that much higher.
        assert_eq!(border_rect_with_strip(96, 408, 28), (68, 436));
        // No strip (or nonsense): as it is.
        assert_eq!(border_rect_with_strip(96, 408, 0), (96, 408));
        assert_eq!(border_rect_with_strip(96, 408, -5), (96, 408));
        // The strip reaches behind the window by the radius of a window's
        // corners: eight pixels, more on a scaled display.
        assert_eq!(stackbar_overlap(96), 8);
        assert_eq!(stackbar_overlap(144), 12);
        assert_eq!(stackbar_overlap(192), 16);
        assert_eq!(stackbar_overlap(0), 8);
    }

    #[test]
    fn stackbar_reserves_the_top_strip() {
        // Container rect (10, 20, 400, 300) with a 40px bar: the windows get
        // the rect minus the strip.
        let content = stackbar_content_rect((10, 20, 400, 300), 40);
        assert_eq!(content, (10, 60, 400, 260));
    }

    #[test]
    fn stackbar_reservation_is_clamped_to_the_container_height() {
        // A bar taller than the container must not produce a negative or
        // zero content height; it degrades to a 1px content rect.
        let content = stackbar_content_rect((0, 0, 400, 30), 40);
        assert_eq!(content, (0, 29, 400, 1));
    }

    #[test]
    fn stackbar_without_height_reserves_nothing() {
        assert_eq!(
            stackbar_content_rect((10, 20, 400, 300), 0),
            (10, 20, 400, 300)
        );
        assert_eq!(
            stackbar_content_rect((10, 20, 400, 300), -5),
            (10, 20, 400, 300)
        );
        // Degenerate container: never reserves, never goes negative.
        assert_eq!(stackbar_content_rect((0, 0, 100, 1), 40), (0, 0, 100, 1));
    }

    #[test]
    fn tabs_divide_the_bar_evenly() {
        assert_eq!(
            stackbar_tab_rects(300, 3),
            vec![(0, 100), (100, 100), (200, 100)]
        );
    }

    #[test]
    fn tab_remainder_pixels_go_to_the_first_tabs() {
        // 302 / 3 = 100 remainder 2: the first two tabs absorb it and the
        // tabs still cover the bar exactly.
        assert_eq!(
            stackbar_tab_rects(302, 3),
            vec![(0, 101), (101, 101), (202, 100)]
        );
    }

    #[test]
    fn tabs_degenerate_inputs_are_empty() {
        assert!(stackbar_tab_rects(300, 0).is_empty());
        assert!(stackbar_tab_rects(0, 3).is_empty());
        assert!(stackbar_tab_rects(-10, 3).is_empty());
    }

    #[test]
    fn hit_test_matches_the_tab_rects() {
        for width in [40, 300, 301, 302] {
            for count in 1..=5usize {
                let tabs = stackbar_tab_rects(width, count);
                for (index, (offset, tab_width)) in tabs.iter().enumerate() {
                    // Every pixel inside the tab maps back to its index.
                    for x in *offset..*offset + *tab_width {
                        assert_eq!(
                            stackbar_tab_index_at(width, count, x),
                            Some(index),
                            "x={x} width={width} count={count}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hit_test_rejects_points_outside_the_bar() {
        assert_eq!(stackbar_tab_index_at(300, 3, -1), None);
        assert_eq!(stackbar_tab_index_at(300, 3, 300), None);
        assert_eq!(stackbar_tab_index_at(300, 3, 5000), None);
        assert_eq!(stackbar_tab_index_at(300, 0, 10), None);
        assert_eq!(stackbar_tab_index_at(0, 3, 0), None);
    }
}

#[cfg(test)]
mod title_bar_tests {
    use super::{WindowHandle, WindowManager};

    #[test]
    fn invalid_windows_fail_without_leaking_hidden_state() {
        let mut window_manager = WindowManager::new();
        let handle = WindowHandle(0x5EAD);

        assert!(window_manager.toggle_title_bar(handle).is_err());
        assert!(window_manager.set_title_bar_visible(handle, false).is_err());
        assert!(window_manager.set_title_bar_visible(handle, true).is_err());
        assert!(!window_manager.is_title_bar_hidden(handle));
    }
}

#[cfg(test)]
mod exit_animation_tests {
    use super::{
        move_must_be_posted, note_posted_move, posted_move_stands, ExitRestore, WindowHandle,
        WindowManager, POSTED_MOVE_LIMIT,
    };
    use crate::dwm::container::Rect;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    };

    /// A real top-level window that is never shown.
    fn hidden_window() -> WindowHandle {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("STATIC"),
                windows::core::w!("gevrek exit animation test window"),
                WINDOW_STYLE::default(),
                0,
                0,
                10,
                10,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create hidden test window");
        WindowHandle(hwnd.0 as isize)
    }

    #[test]
    fn a_window_that_refuses_to_close_gets_its_appearance_back() {
        let mut window_manager = WindowManager::new();
        let survivor = hidden_window();
        let target = Rect::new(40, 50, 300, 200);
        let now = Instant::now();
        let restore = ExitRestore {
            rect: target,
            alpha: 255,
        };
        window_manager
            .close_checks
            .insert(survivor, (now + Duration::from_secs(1), restore));

        // Not due yet: nothing happens.
        window_manager.undo_cancelled_exits(now);
        assert!(window_manager.close_checks.contains_key(&survivor));

        window_manager.undo_cancelled_exits(now + Duration::from_secs(1));
        assert!(window_manager.close_checks.is_empty());
        assert_eq!(window_manager.get_window_rect(survivor).unwrap(), target);

        unsafe {
            let _ = DestroyWindow(HWND(survivor.0 as _));
        }
    }

    #[test]
    fn a_new_window_with_an_entrance_animation_still_reaches_its_tile() {
        use crate::dwm::TransitionScope;
        use crate::settings::{
            AnimationChannelProfile, AnimationCurve, AnimationProfile, WindowTransitionStyle,
        };
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(AnimationProfile {
            enabled: true,
            layout: AnimationChannelProfile::new(true, 40, AnimationCurve::Linear),
            window: AnimationChannelProfile::new(true, 40, AnimationCurve::Linear),
            window_enter_style: WindowTransitionStyle::Scale,
            ..AnimationProfile::default()
        });
        let window = hidden_window();
        let tile = Rect::new(12, 12, 600, 400);

        // Adoption tiles the window (animated), then its entrance starts.
        window_manager
            .apply_animated_rectangles(&[(window, tile)], TransitionScope::Layout)
            .unwrap();
        assert!(window_manager
            .start_configured_window_enter(window)
            .unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        while window_manager.active_animation_count() > 0 && Instant::now() < deadline {
            window_manager.tick_runtime_animations().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    /// A shown window, far beyond every display (shown, but seen nowhere).
    fn shown_window_out_of_sight() -> WindowHandle {
        use windows::Win32::UI::WindowsAndMessaging::{
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows::core::w!("STATIC"),
                windows::core::w!("gevrek stand-in test window"),
                WS_POPUP | WS_VISIBLE,
                -30000,
                -30000,
                200,
                150,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create shown test window");
        WindowHandle(hwnd.0 as isize)
    }

    fn layered_alpha(window: WindowHandle) -> Option<u8> {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetLayeredWindowAttributes, GetWindowLongW, GWL_EXSTYLE,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS, WS_EX_LAYERED,
        };
        let mut alpha = 0u8;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
        unsafe {
            let layered = GetWindowLongW(HWND(window.0 as _), GWL_EXSTYLE) & WS_EX_LAYERED.0 as i32;
            if layered == 0 {
                return None;
            }
            GetLayeredWindowAttributes(
                HWND(window.0 as _),
                None,
                Some(&mut alpha),
                Some(&mut flags),
            )
            .ok()?;
        }
        Some(alpha)
    }

    fn scale_entrance_profile() -> crate::settings::AnimationProfile {
        use crate::settings::{
            AnimationChannelProfile, AnimationCurve, AnimationProfile, WindowTransitionStyle,
        };
        AnimationProfile {
            enabled: true,
            layout: AnimationChannelProfile::new(true, 60, AnimationCurve::Linear),
            // (Long enough that a test finds it under way on a machine
            // that is busy with the other tests.)
            window: AnimationChannelProfile::new(true, 400, AnimationCurve::Linear),
            window_enter_style: WindowTransitionStyle::Scale,
            ..AnimationProfile::default()
        }
    }

    fn run_animations_out(window_manager: &mut WindowManager) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while window_manager.active_animation_count() > 0 && Instant::now() < deadline {
            window_manager.tick_runtime_animations().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_window_that_comes_in_stays_in_its_place_while_its_picture_grows() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(scale_entrance_profile());
        let window = shown_window_out_of_sight();
        let tile = Rect::new(-30000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, tile)]).unwrap();

        assert!(window_manager.start_configured_window_enter(window).unwrap());
        assert!(window_manager.stand_ins.has(window));
        // The window is in its place at once, at its size, and unseen.
        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        assert_eq!(layered_alpha(window), Some(0));
        assert_eq!(window_manager.window_alpha(window), 255);

        std::thread::sleep(Duration::from_millis(10));
        window_manager.tick_runtime_animations().unwrap();
        // Midway it has not been moved or sized: only its picture has.
        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        let picture = window_manager.stand_ins.rect(window).unwrap();
        assert!(picture.width < tile.width && picture.height < tile.height);

        // A strength asked for meanwhile is the window's once it is there.
        window_manager.set_window_alpha(window, 200).unwrap();
        assert_eq!(layered_alpha(window), Some(0));

        run_animations_out(&mut window_manager);
        assert!(!window_manager.stand_ins.has(window));
        assert_eq!(window_manager.stand_ins.count(), 0);
        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        assert_eq!(layered_alpha(window), Some(200));
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    /// What was posted to this thread's windows is carried out (a window
    /// hidden by a message is hidden once it has had the message).
    fn messages_are_heard() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    fn workspace_slide_profile(pictures: bool, rest: bool) -> crate::settings::AnimationProfile {
        use crate::settings::{
            AnimationChannelProfile, AnimationCurve, AnimationProfile, WorkspaceTransitionStyle,
        };
        AnimationProfile {
            enabled: true,
            workspace: AnimationChannelProfile::new(true, 400, AnimationCurve::Linear),
            workspace_style: WorkspaceTransitionStyle::Slide,
            workspace_slides_as_pictures: pictures,
            workspace_slides_without_shadows: rest,
            ..AnimationProfile::default()
        }
    }

    fn is_shown(window: WindowHandle) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        unsafe { IsWindowVisible(HWND(window.0 as _)).as_bool() }
    }

    /// A window that was moved behind gevrek's back while it was hidden
    /// (by a move its program got to late: one sent by post, which Windows
    /// carries out after every move sent the plain way since) is where it
    /// belongs when it is shown again, and that is where gevrek says it
    /// is meanwhile. (It was shown where its workspace's way in began,
    /// beside the display: on a workspace whose windows float, gone.)
    #[test]
    fn a_hidden_window_is_shown_again_where_it_belongs() {
        let _alone = crate::runtime::test_support::alone_with_the_journal();
        use gevrek_engine::contract::WindowQueries;
        use windows::Win32::UI::WindowsAndMessaging::{
            DestroyWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
        };
        let mut window_manager = WindowManager::new();
        let window = shown_window_out_of_sight();
        let hwnd = HWND(window.0 as _);
        let home = Rect::new(-30000, -30000, 600, 400);
        let astray = Rect::new(-27000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, home)]).unwrap();
        assert_eq!(window_manager.hidden_place(window), None);

        window_manager.hide_window(window).unwrap();
        messages_are_heard();
        assert!(!is_shown(window));
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                astray.x,
                astray.y,
                astray.width,
                astray.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        assert_eq!(window_manager.get_window_rect(window).unwrap(), astray);
        assert_eq!(window_manager.hidden_place(window), Some(home));
        assert_eq!(WindowQueries::rect(&window_manager, window).unwrap(), home);
        // Hidden a second time, it still belongs where it did.
        window_manager.hide_window(window).unwrap();
        assert_eq!(window_manager.hidden_place(window), Some(home));

        window_manager.show_window(window).unwrap();
        messages_are_heard();
        assert!(is_shown(window));
        assert_eq!(window_manager.get_window_rect(window).unwrap(), home);
        assert_eq!(window_manager.hidden_place(window), None);

        // Where gevrek puts a hidden window is where it belongs from then
        // on (a tile it is given while its workspace is away).
        let tile = Rect::new(-29000, -29000, 500, 300);
        window_manager.hide_window(window).unwrap();
        messages_are_heard();
        window_manager
            .move_and_resize_window(window, tile.x, tile.y, tile.width, tile.height)
            .unwrap();
        assert_eq!(window_manager.hidden_place(window), Some(tile));
        window_manager.show_window(window).unwrap();
        messages_are_heard();
        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// A window that is asked to hide again while it is on its way out
    /// with its workspace belongs where it did before it set out, not
    /// where its way out ends.
    #[test]
    fn a_window_hidden_again_on_its_way_out_keeps_its_place() {
        let _alone = crate::runtime::test_support::alone_with_the_journal();
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(false, false));
        let window = shown_window_out_of_sight();
        let home = Rect::new(-30000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, home)]).unwrap();

        window_manager.workspace_gives_way();
        window_manager.hide_window(window).unwrap();
        let _ = window_manager.begin_workspace_transition(&[], 800, 0);
        assert!(window_manager.going_out.leaving.contains_key(&window));
        std::thread::sleep(Duration::from_millis(30));
        window_manager.tick_runtime_animations().unwrap();
        assert_ne!(window_manager.get_window_rect(window).unwrap(), home);

        window_manager.workspace_gives_way();
        window_manager.hide_window(window).unwrap();
        assert_eq!(window_manager.going_out.waiting, vec![(window, home)]);
        assert_eq!(window_manager.hidden_place(window), Some(home));

        window_manager.show_window(window).unwrap();
        messages_are_heard();
        assert_eq!(window_manager.get_window_rect(window).unwrap(), home);
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    /// Where nothing moves, the windows of a workspace that gives way
    /// stay shown until those of the one that comes are there: hidden at
    /// once, the bare desktop was seen between the two.
    #[test]
    fn where_nothing_moves_a_workspace_stays_until_the_next_is_there() {
        let _alone = crate::runtime::test_support::alone_with_the_journal();
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        let mut window_manager = WindowManager::new();
        use super::HELD_FOR_THE_NEXT;
        // (As with the workspace's animation switched off.)
        window_manager.set_animation_profile(crate::settings::AnimationProfile {
            workspace: crate::settings::AnimationChannelProfile::new(
                false,
                400,
                crate::settings::AnimationCurve::Linear,
            ),
            ..workspace_slide_profile(false, false)
        });
        assert!(!window_manager.animation_scheduler.moves(super::TransitionScope::Workspace));
        let window = shown_window_out_of_sight();
        let home = Rect::new(-30000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, home)]).unwrap();

        window_manager.workspace_gives_way();
        window_manager.hide_window(window).unwrap();
        let _ = window_manager.begin_workspace_transition(&[], 800, 0);
        window_manager.tick_runtime_animations().unwrap();
        messages_are_heard();
        assert!(is_shown(window), "still there while the next one comes");

        std::thread::sleep(HELD_FOR_THE_NEXT + Duration::from_millis(20));
        window_manager.tick_runtime_animations().unwrap();
        messages_are_heard();
        assert!(!is_shown(window), "and hidden when it is");
        assert!(window_manager.going_out.leaving.is_empty());
        assert_eq!(window_manager.hidden_place(window), Some(home));

        // Shown again before its time was over, it stays.
        window_manager.show_window(window).unwrap();
        window_manager.workspace_gives_way();
        window_manager.hide_window(window).unwrap();
        let _ = window_manager.begin_workspace_transition(&[], 800, 0);
        window_manager.show_window(window).unwrap();
        std::thread::sleep(HELD_FOR_THE_NEXT + Duration::from_millis(20));
        window_manager.tick_runtime_animations().unwrap();
        messages_are_heard();
        assert!(is_shown(window));
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    /// What a program shows beside its windows that belongs to none of
    /// them (a notification it draws itself) goes with the last of its
    /// windows that is in sight, and stays while another one is.
    #[test]
    fn what_a_program_shows_beside_its_windows_goes_with_the_last_of_them() {
        let (own, beside) = (hidden_window(), shown_window_out_of_sight());
        let (own, beside) = (HWND(own.0 as _), HWND(beside.0 as _));
        assert_eq!(super::companions_on_screen(own, |_| false), vec![beside]);
        // Another window of the program is still to be seen.
        assert!(super::companions_on_screen(own, |other| other == beside).is_empty());
        // Nothing goes with itself, and nothing that is not shown.
        assert!(super::companions_on_screen(beside, |_| false).is_empty());
        unsafe {
            let _ = DestroyWindow(own);
            let _ = DestroyWindow(beside);
        }
    }

    /// The moves of one window are sent the same way until one sent by
    /// post has been carried out.
    #[test]
    fn a_posted_move_stands_until_it_is_carried_out() {
        let asked = (10, 20, 300, 200);
        let short = Duration::from_millis(50);
        // Not there yet: the next one is posted as well.
        assert!(posted_move_stands(asked, Some((0, 0, 300, 200)), short));
        // Carried out, or the window is gone: no longer.
        assert!(!posted_move_stands(asked, Some(asked), short));
        assert!(!posted_move_stands(asked, None, short));
        // A program that put its window elsewhere than asked is not
        // waited for without end.
        assert!(!posted_move_stands(asked, Some((0, 0, 300, 200)), POSTED_MOVE_LIMIT));
    }

    /// A window that was sent a move by post gets its next move by post
    /// too, while the first has not been carried out.
    #[test]
    fn a_window_with_a_posted_move_under_way_is_moved_by_post() {
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        let window = shown_window_out_of_sight();
        let hwnd = HWND(window.0 as _);
        assert!(!move_must_be_posted(hwnd));
        // (Asked for a place it is not at.)
        note_posted_move(hwnd, (-29000, -29000, 200, 150));
        assert!(move_must_be_posted(hwnd));
        // (It is where it was asked to be: carried out.)
        note_posted_move(hwnd, (-30000, -30000, 200, 150));
        assert!(!move_must_be_posted(hwnd));
        assert!(!move_must_be_posted(hwnd));
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// A window that is minimized "is" where the system keeps its mark:
    /// far off every display, as small as a title. Going out with its
    /// workspace and coming with it, it is moved by nothing. (Taken as
    /// the place it has, that was where it was put: brought back while
    /// its workspace came, or after it had gone, it was very small and
    /// out of sight, whatever had been put right before.)
    #[test]
    fn a_minimized_window_is_not_moved_when_its_workspace_goes_and_comes() {
        // (It hides a window, which the process's journal of hidden
        // windows then names: no test that shows what the journal names
        // may run meanwhile.)
        let _alone = crate::runtime::test_support::alone_with_the_journal();
        use windows::Win32::UI::WindowsAndMessaging::{
            DestroyWindow, IsIconic, ShowWindow, SW_MINIMIZE, SW_RESTORE, SW_SHOWNA,
        };
        let home = Rect::new(-30000, -30000, 600, 400);
        let settle = |window_manager: &mut WindowManager| {
            let deadline = Instant::now() + Duration::from_millis(1200);
            while Instant::now() < deadline {
                window_manager.tick_runtime_animations().unwrap();
                messages_are_heard();
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        for (pictures, comes) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut window_manager = WindowManager::new();
            window_manager.set_animation_profile(workspace_slide_profile(pictures, false));
            let window = shown_window_out_of_sight();
            let hwnd = HWND(window.0 as _);
            window_manager.batch_move_resize(&[(window, home)]).unwrap();
            unsafe {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            messages_are_heard();
            assert!(unsafe { IsIconic(hwnd) }.as_bool());
            if comes {
                // Its workspace comes, with the place the system says
                // the window is at; and the window is brought back (as
                // one that is given the keyboard is).
                let said = window_manager.get_window_rect(window).unwrap();
                assert_ne!(said, home);
                let _ = window_manager.begin_workspace_transition(&[(window, said)], 800, 0);
                unsafe {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                settle(&mut window_manager);
            } else {
                // Its workspace goes; later the window is brought back.
                window_manager.workspace_gives_way();
                window_manager.hide_window(window).unwrap();
                let _ = window_manager.begin_workspace_transition(&[], 800, 0);
                settle(&mut window_manager);
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOWNA);
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                settle(&mut window_manager);
            }
            assert_eq!(
                window_manager.get_window_rect(window).unwrap(),
                home,
                "pictures: {pictures}, comes: {comes}"
            );
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    #[test]
    fn a_workspace_gives_way_as_pictures_and_its_windows_stay_where_they_are() {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNA};
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(true, true));
        let (going, coming) = (shown_window_out_of_sight(), shown_window_out_of_sight());
        let home = Rect::new(-30000, -30000, 600, 400);
        let tile = Rect::new(-30000, -29000, 500, 300);
        window_manager
            .batch_move_resize(&[(going, home), (coming, tile)])
            .unwrap();
        unsafe {
            let _ = ShowWindow(HWND(coming.0 as _), SW_HIDE);
        }

        // As a change of workspace goes: the old one's window is asked to
        // hide and stays shown, the new one's is laid out while hidden,
        // the change begins, the new one's window is shown.
        window_manager.workspace_gives_way();
        window_manager.hide_window(going).unwrap();
        assert!(is_shown(going));
        assert!(window_manager
            .begin_workspace_transition(&[(coming, tile)], 800, 0)
            .unwrap());
        unsafe {
            let _ = ShowWindow(HWND(coming.0 as _), SW_SHOWNA);
        }
        // Both are pictures of themselves; neither window has moved, and
        // neither is seen itself. Their shadow and glow rest.
        for (window, place) in [(going, home), (coming, tile)] {
            assert!(window_manager.stand_ins.has(window));
            assert_eq!(window_manager.get_window_rect(window).unwrap(), place);
            assert_eq!(layered_alpha(window), Some(0));
            assert!(window_manager.effects_resting.contains(&window));
        }
        let (went_from, came_from) = (
            window_manager.stand_ins.rect(going).unwrap(),
            window_manager.stand_ins.rect(coming).unwrap(),
        );
        assert_eq!(went_from, home);
        assert_ne!(came_from, tile);

        std::thread::sleep(Duration::from_millis(30));
        window_manager.tick_runtime_animations().unwrap();
        // Under way: the pictures have moved, the windows have not.
        assert_ne!(window_manager.stand_ins.rect(going).unwrap(), went_from);
        assert_ne!(window_manager.stand_ins.rect(coming).unwrap(), came_from);
        assert_eq!(window_manager.get_window_rect(going).unwrap(), home);
        assert_eq!(window_manager.get_window_rect(coming).unwrap(), tile);

        let deadline = Instant::now() + Duration::from_secs(4);
        while window_manager.active_animation_count() > 0 && Instant::now() < deadline {
            window_manager.tick_runtime_animations().unwrap();
            messages_are_heard();
            std::thread::sleep(Duration::from_millis(5));
        }
        // Over: the new one's window is seen itself, in its place; the old
        // one's is hidden where it was, as solid as it was for the next
        // time; no picture is left, and the effects are back.
        assert_eq!(window_manager.stand_ins.count(), 0);
        assert!(window_manager.pictures_going.is_empty());
        assert!(window_manager.effects_resting.is_empty());
        assert!(is_shown(coming));
        assert_eq!(layered_alpha(coming), None);
        assert_eq!(window_manager.get_window_rect(coming).unwrap(), tile);
        assert!(!is_shown(going));
        assert_eq!(layered_alpha(going), None);
        assert_eq!(window_manager.get_window_rect(going).unwrap(), home);
        unsafe {
            let _ = DestroyWindow(HWND(going.0 as _));
            let _ = DestroyWindow(HWND(coming.0 as _));
        }
    }

    #[test]
    fn an_overview_shows_pictures_of_the_windows_and_leaves_them_as_they_are() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(false, false));
        let (one, two) = (shown_window_out_of_sight(), shown_window_out_of_sight());
        let (home_one, home_two) = (
            Rect::new(-30000, -30000, 900, 700),
            Rect::new(-29000, -30000, 600, 400),
        );
        window_manager
            .batch_move_resize(&[(one, home_one), (two, home_two)])
            .unwrap();
        let area = Rect::new(-30000, -30000, 1600, 900);

        assert!(window_manager.toggle_window_overview(&[one, two], area));
        assert!(window_manager.window_overview_is_shown());
        // Each is a picture of itself, which starts where the window is;
        // the windows are unseen, where they were and as large.
        for (window, home) in [(one, home_one), (two, home_two)] {
            assert_eq!(window_manager.stand_ins.rect(window), Some(home));
            assert_eq!(window_manager.get_window_rect(window).unwrap(), home);
            assert_eq!(layered_alpha(window), Some(0));
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        while (window_manager.animation_scheduler.rect_target(one).is_some()
            || window_manager.animation_scheduler.rect_target(two).is_some())
            && Instant::now() < deadline
        {
            window_manager.tick_runtime_animations().unwrap();
            messages_are_heard();
            std::thread::sleep(Duration::from_millis(5));
        }
        // The pictures have arrived: small, in the area, and they stay
        // for as long as the overview does.
        for _ in 0..5 {
            window_manager.tick_runtime_animations().unwrap();
            messages_are_heard();
        }
        for (window, home) in [(one, home_one), (two, home_two)] {
            let picture = window_manager.stand_ins.rect(window).unwrap();
            assert!(picture.width < home.width && picture.height < home.height);
            assert!(picture.x >= area.x && picture.x + picture.width <= area.x + area.width);
            assert!(picture.y >= area.y && picture.y + picture.height <= area.y + area.height);
            assert_eq!(window_manager.get_window_rect(window).unwrap(), home);
            assert_eq!(layered_alpha(window), Some(0));
        }
        assert!(window_manager.window_overview_is_shown());

        // Put away: the pictures go back, and the windows are seen
        // themselves again, where they always were.
        assert!(!window_manager.toggle_window_overview(&[one, two], area));
        assert!(!window_manager.window_overview_is_shown());
        let deadline = Instant::now() + Duration::from_secs(4);
        while window_manager.active_animation_count() > 0 && Instant::now() < deadline {
            window_manager.tick_runtime_animations().unwrap();
            messages_are_heard();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(window_manager.stand_ins.count(), 0);
        assert!(window_manager.overview_leaving.is_empty());
        assert!(window_manager.effects_resting.is_empty());
        for (window, home) in [(one, home_one), (two, home_two)] {
            assert!(is_shown(window));
            assert_eq!(layered_alpha(window), None);
            assert_eq!(window_manager.get_window_rect(window).unwrap(), home);
        }
        unsafe {
            let _ = DestroyWindow(HWND(one.0 as _));
            let _ = DestroyWindow(HWND(two.0 as _));
        }
    }

    #[test]
    fn a_window_that_is_being_hidden_is_out_of_sight_for_its_border() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(false, false));
        let window = shown_window_out_of_sight();
        let home = Rect::new(-30000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, home)]).unwrap();
        assert!(!window_manager.kept_out_of_sight(window));

        // Hidden, as a window that leaves a workspace is: asked for, and
        // not done yet (the message is not heard here). Windows still
        // calls it visible; for its border it is out of sight already.
        window_manager.hide_window(window).unwrap();
        assert!(is_shown(window));
        assert!(window_manager.kept_out_of_sight(window));
        messages_are_heard();
        assert!(!is_shown(window));
        assert!(window_manager.kept_out_of_sight(window));

        // Shown again, it is to be seen, and its border with it.
        window_manager.show_window(window).unwrap();
        messages_are_heard();
        assert!(!window_manager.kept_out_of_sight(window));

        // One that goes out with its workspace keeps its border on the
        // way, and loses it when it is hidden at the end of it.
        window_manager.workspace_gives_way();
        window_manager.hide_window(window).unwrap();
        assert!(is_shown(window));
        assert!(!window_manager.kept_out_of_sight(window));
        window_manager.stop_going_out(window, true);
        assert!(window_manager.kept_out_of_sight(window));
        messages_are_heard();
        assert!(!is_shown(window));

        window_manager.show_window(window).unwrap();
        messages_are_heard();
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    #[test]
    fn an_overview_ends_when_a_window_of_it_goes() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(false, false));
        let (one, two) = (shown_window_out_of_sight(), shown_window_out_of_sight());
        let area = Rect::new(-30000, -30000, 1600, 900);
        assert!(window_manager.toggle_window_overview(&[one, two], area));
        // Nothing to show: no overview.
        let mut other = WindowManager::new();
        assert!(!other.toggle_window_overview(&[], area));
        unsafe {
            let _ = DestroyWindow(HWND(two.0 as _));
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        while (window_manager.window_overview_is_shown()
            || window_manager.active_animation_count() > 0)
            && Instant::now() < deadline
        {
            window_manager.tick_runtime_animations().unwrap();
            messages_are_heard();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!window_manager.window_overview_is_shown());
        assert_eq!(window_manager.stand_ins.count(), 0);
        assert_eq!(layered_alpha(one), None);
        unsafe {
            let _ = DestroyWindow(HWND(one.0 as _));
        }
    }

    #[test]
    fn a_workspace_that_is_not_to_go_as_pictures_moves_its_windows() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(workspace_slide_profile(false, false));
        let coming = shown_window_out_of_sight();
        let tile = Rect::new(-30000, -29000, 500, 300);
        window_manager.batch_move_resize(&[(coming, tile)]).unwrap();
        assert!(window_manager
            .begin_workspace_transition(&[(coming, tile)], 800, 0)
            .unwrap());
        // The window itself was put aside to come in, seen as it is.
        assert!(!window_manager.stand_ins.has(coming));
        assert_ne!(window_manager.get_window_rect(coming).unwrap(), tile);
        assert_eq!(layered_alpha(coming), None);
        assert!(window_manager.effects_resting.is_empty());
        run_animations_out(&mut window_manager);
        assert_eq!(window_manager.get_window_rect(coming).unwrap(), tile);
        unsafe {
            let _ = DestroyWindow(HWND(coming.0 as _));
        }
    }

    #[test]
    fn a_window_that_comes_in_and_is_laid_out_anew_goes_to_its_new_place() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(scale_entrance_profile());
        let window = shown_window_out_of_sight();
        let tile = Rect::new(-30000, -30000, 600, 400);
        let new_tile = Rect::new(-30000, -30000, 300, 400);
        window_manager.batch_move_resize(&[(window, tile)]).unwrap();
        assert!(window_manager.start_configured_window_enter(window).unwrap());

        window_manager
            .apply_animated_rectangles(&[(window, new_tile)], crate::dwm::TransitionScope::Layout)
            .unwrap();
        // Still a picture; the window itself went to its new place at once.
        assert!(window_manager.stand_ins.has(window));
        assert_eq!(window_manager.get_window_rect(window).unwrap(), new_tile);

        run_animations_out(&mut window_manager);
        assert!(!window_manager.stand_ins.has(window));
        assert_eq!(window_manager.get_window_rect(window).unwrap(), new_tile);
        // Solid again as it was: gevrek's see-through style is gone.
        assert_eq!(layered_alpha(window), None);
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    #[test]
    fn a_window_hidden_while_it_comes_in_is_itself_again() {
        let mut window_manager = WindowManager::new();
        window_manager.set_animation_profile(scale_entrance_profile());
        let window = shown_window_out_of_sight();
        let tile = Rect::new(-30000, -30000, 600, 400);
        window_manager.batch_move_resize(&[(window, tile)]).unwrap();
        assert!(window_manager.start_configured_window_enter(window).unwrap());

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            let _ = ShowWindow(HWND(window.0 as _), SW_HIDE);
        }
        window_manager.tick_runtime_animations().unwrap();
        assert!(!window_manager.stand_ins.has(window));
        assert_eq!(layered_alpha(window), None);
        assert_eq!(window_manager.get_window_rect(window).unwrap(), tile);
        run_animations_out(&mut window_manager);
        assert_eq!(window_manager.stand_ins.count(), 0);
        unsafe {
            let _ = DestroyWindow(HWND(window.0 as _));
        }
    }

    #[test]
    fn a_window_that_closed_is_simply_forgotten() {
        let mut window_manager = WindowManager::new();
        let closed = hidden_window();
        unsafe {
            let _ = DestroyWindow(HWND(closed.0 as _));
        }
        let now = Instant::now();
        window_manager.close_checks.insert(
            closed,
            (
                now,
                ExitRestore {
                    rect: Rect::new(0, 0, 10, 10),
                    alpha: 255,
                },
            ),
        );
        window_manager.undo_cancelled_exits(now);
        assert!(window_manager.close_checks.is_empty());
    }
}

#[cfg(test)]
mod registry_tests {
    use super::{WindowHandle, WindowInfo, WindowManager};
    use crate::dwm::WindowEvent;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE,
    };

    struct HiddenWindow(WindowHandle);

    impl HiddenWindow {
        fn create() -> Self {
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    windows::core::w!("STATIC"),
                    windows::core::w!("gevrek registry test"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("create hidden test window");
            Self(WindowHandle(hwnd.0 as isize))
        }

        fn destroy(&self) {
            unsafe {
                let _ = DestroyWindow(HWND(self.0 .0 as _));
            }
        }
    }

    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            self.destroy();
        }
    }

    fn register(window_manager: &mut WindowManager, handle: WindowHandle, title: &str) {
        window_manager.windows.insert(
            handle,
            WindowInfo {
                handle,
                title: title.to_string(),
                x: 0,
                y: 0,
                width: 10,
                height: 10,
                is_visible: false,
                is_minimized: false,
                is_maximized: false,
            },
        );
    }

    #[test]
    fn a_title_change_updates_only_that_windows_title() {
        let mut window_manager = WindowManager::new();
        let window = HiddenWindow::create();
        register(&mut window_manager, window.0, "old title");

        unsafe {
            SetWindowTextW(HWND(window.0 .0 as _), windows::core::w!("new title")).unwrap();
        }
        window_manager.apply_window_event(&WindowEvent::MetadataChanged(window.0));
        assert_eq!(
            window_manager.get_window_info(window.0).unwrap().title,
            "new title"
        );
    }

    #[test]
    fn ineligible_windows_are_not_admitted_by_events() {
        let mut window_manager = WindowManager::new();
        // Never shown, so it fails the same filter the enumeration uses.
        let window = HiddenWindow::create();
        window_manager.apply_window_event(&WindowEvent::Created(window.0));
        window_manager.apply_window_event(&WindowEvent::MetadataChanged(window.0));
        assert!(window_manager.get_window_info(window.0).is_none());
    }

    #[test]
    fn hidden_windows_survive_pruning_and_dead_ones_do_not() {
        let mut window_manager = WindowManager::new();
        let hidden = HiddenWindow::create();
        let dead = HiddenWindow::create();
        register(&mut window_manager, hidden.0, "on another workspace");
        register(
            &mut window_manager,
            dead.0,
            "closed while events were missed",
        );
        dead.destroy();

        assert_eq!(window_manager.prune_dead_windows(), 1);
        assert!(window_manager.get_window_info(hidden.0).is_some());
        assert!(window_manager.get_window_info(dead.0).is_none());
    }

    #[test]
    fn destroy_events_forget_the_window_and_its_identity() {
        let mut window_manager = WindowManager::new();
        let window = HiddenWindow::create();
        register(&mut window_manager, window.0, "short lived");
        assert_eq!(window_manager.get_window_class(window.0), "Static");
        assert!(window_manager.identities.borrow().contains_key(&window.0));

        window_manager.apply_window_event(&WindowEvent::Destroyed(window.0));
        assert!(window_manager.get_window_info(window.0).is_none());
        assert!(!window_manager.identities.borrow().contains_key(&window.0));
    }

    #[test]
    fn identity_is_resolved_once_and_names_this_process() {
        let window_manager = WindowManager::new();
        let window = HiddenWindow::create();
        let identity = window_manager.window_identity(window.0);
        assert_eq!(identity.pid, std::process::id());
        let exe = std::env::current_exe().unwrap();
        assert!(
            identity.path.eq_ignore_ascii_case(&exe.to_string_lossy()),
            "{} vs {}",
            identity.path,
            exe.display()
        );
        assert_eq!(window_manager.window_identity(window.0), identity);
        assert_eq!(window_manager.identities.borrow().len(), 1);
    }
}

#[cfg(test)]
mod unresponsive_window_tests {
    use super::*;
    use std::time::Instant;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };

    /// A window whose thread stops pumping messages for `stall`, like an app
    /// that freezes right after its window appeared (before Windows' own
    /// 5 s "Not Responding" detection kicks in).
    fn frozen_window(stall: Duration) -> (WindowHandle, std::thread::JoinHandle<()>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    windows::core::w!("STATIC"),
                    windows::core::w!("gevrek frozen window"),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    300,
                    200,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("create window");
            sender.send(hwnd.0 as isize).unwrap();
            std::thread::sleep(stall);
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        });
        (WindowHandle(receiver.recv().unwrap()), owner)
    }

    type Operation = Box<dyn Fn(&mut WindowManager, WindowHandle)>;

    #[test]
    fn calls_that_wait_on_a_window_never_block_on_a_frozen_one() {
        let operations: Vec<(&str, Operation)> = vec![
            (
                "set_window_alpha",
                Box::new(|wm, w| {
                    let _ = wm.set_window_alpha(w, 200);
                }),
            ),
            (
                "focus_window",
                Box::new(|wm, w| {
                    let _ = wm.focus_window(w);
                }),
            ),
            (
                "remove_min_max_buttons",
                Box::new(|wm, w| {
                    let _ = wm.remove_min_max_buttons(w);
                }),
            ),
            (
                "batch_move_resize",
                Box::new(|wm, w| {
                    let _ = wm.batch_move_resize(&[(w, crate::dwm::Rect::new(10, 10, 400, 300))]);
                }),
            ),
        ];
        for (name, operation) in operations {
            let mut window_manager = WindowManager::new();
            let (window, owner) = frozen_window(Duration::from_millis(1500));
            let started = Instant::now();
            operation(&mut window_manager, window);
            let elapsed = started.elapsed();
            owner.join().unwrap();
            assert!(
                elapsed < Duration::from_millis(500),
                "{name} blocked for {elapsed:?} on a frozen window"
            );
        }
    }
}

#[cfg(test)]
mod desktop_band_tests {
    use super::desktop_band_class;

    /// A tiled window put below the floating ones is never put below
    /// these: below them the desktop covers it.
    #[test]
    fn the_desktop_begins_at_explorer_or_at_a_surface_of_the_shell() {
        for class in [
            "Progman",
            "WorkerW",
            "Gevrek_Shell_gevrek-zones",
            "Gevrek_Shell_gevrek-zones.dev",
        ] {
            assert!(desktop_band_class(class), "{class}");
        }
        for class in [
            "Chrome_WidgetWin_1",
            "Gevrek_ShellPopup_gevrek-zones",
            "Gevrek_ShellHint_gevrek-zones",
            "Gevrek_ShellBackdrop_gevrek-zones",
            "Gevrek_Border",
            "Shell_TrayWnd",
        ] {
            assert!(!desktop_band_class(class), "{class}");
        }
    }
}
