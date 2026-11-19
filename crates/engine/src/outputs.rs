//! Display outputs and their workspace rings (backend-neutral).
//! Enumeration and the OS lookups live in the Windows backend (core
//! `dwm::monitor`, `MonitorManagerWin32`).

use super::container::Rect;
use super::ring::Ring;
use super::workspace::Workspace;
use anyhow::Result;
use gevrek_model::settings::ReservedArea;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[path = "outputs_reconcile.rs"]
mod reconcile;
pub use reconcile::MonitorChanges;

// What a display says of itself and the id made of it: in the model, so
// that the shell reads it without the engine. Known here by its old name.
pub use gevrek_model::edid;

pub use gevrek_model::ids::OutputId as MonitorHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub handle: MonitorHandle,
    pub name: String,
    /// Device interface path of the display (`\\?\DISPLAY#...`), stable for
    /// a physical display on a port across reconnects, unlike `handle`.
    /// Falls back to the GDI device name (`\\.\DISPLAY1`).
    #[serde(default)]
    pub device_path: String,
    /// Who the display is, wherever it is plugged in: made of its EDID
    /// (maker, product code, serial number: `GSM-5B7F-0004E2A1`), so the
    /// same on another port or connector and on the other system. A display
    /// that gives no serial number ends in `-noserial`, and one of two such
    /// twins in `#1`, `#2` by where they are plugged in; one without an
    /// EDID is known by its place alone (`port:HDMI-A-1`). See [`edid`].
    ///
    /// Nothing is found by it yet: workspaces, profiles and what is kept
    /// over a restart go by `device_path` and the name.
    #[serde(default)]
    pub stable_id: String,
    pub is_primary: bool,

    // Full monitor rectangle
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,

    // Work area (excludes taskbar)
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: i32,
    pub work_height: i32,

    // DPI scaling
    pub dpi_scale: f32,
}

/// A display's name without the number that tells it from another of the
/// same model (`Monitor ACM2702 (2)` -> `Monitor ACM2702`).
pub fn name_without_number(name: &str) -> &str {
    let name = name.trim();
    let Some(open) = name.rfind(" (") else {
        return name;
    };
    let number = &name[open + 2..];
    match number.strip_suffix(')') {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => {
            name[..open].trim_end()
        }
        _ => name,
    }
}

/// Give displays that have the same name a number each, so that every name
/// means one display: `Monitor ACM2702 (1)`, `Monitor ACM2702 (2)`. The
/// numbers go by the device path, which stays with the port the display is
/// plugged into, so the same display keeps its number over a restart and
/// when the displays are arranged anew. A name that is alone is left as it
/// is.
pub fn number_same_names(monitors: &mut [MonitorInfo]) {
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, monitor) in monitors.iter().enumerate() {
        by_name
            .entry(monitor.name.trim().to_lowercase())
            .or_default()
            .push(index);
    }
    for mut twins in by_name.into_values().filter(|twins| twins.len() > 1) {
        twins.sort_by(|a, b| {
            let (a, b) = (&monitors[*a], &monitors[*b]);
            a.device_path
                .to_lowercase()
                .cmp(&b.device_path.to_lowercase())
                .then((a.x, a.y).cmp(&(b.x, b.y)))
                .then(a.handle.0.cmp(&b.handle.0))
        });
        for (number, index) in twins.into_iter().enumerate() {
            let name = monitors[index].name.trim().to_string();
            monitors[index].name = format!("{name} ({})", number + 1);
        }
    }
}

/// Tell displays apart that have the same id: two of one model that give no
/// serial number (or the same one) have the same EDID to the last byte.
/// Each gets a number after its id, `GSM-5B7F-noserial#1` and `#2`, by the
/// device path as in `number_same_names`. That number stays with the port
/// (it is all there is to go by), and a twin that is left alone, the other
/// unplugged, has the id without a number. An id that is alone is left as
/// it is.
pub fn number_same_stable_ids(monitors: &mut [MonitorInfo]) {
    // (The rule itself is written once, beside the ids: `edid`.)
    let mut ids: Vec<String> = monitors.iter().map(|monitor| monitor.stable_id.clone()).collect();
    let places: Vec<(String, i32, i32, isize)> = monitors
        .iter()
        .map(|monitor| (monitor.device_path.clone(), monitor.x, monitor.y, monitor.handle.0))
        .collect();
    edid::number_same_ids(&mut ids, &places);
    for (monitor, id) in monitors.iter_mut().zip(ids) {
        monitor.stable_id = id;
    }
}

impl MonitorInfo {
    /// Whether `wanted` (from the configuration or a command) means this
    /// display: its name, whatever the case, or the name its model has
    /// when there are several of it (which then means each of them, as it
    /// did before they had numbers).
    pub fn is_called(&self, wanted: &str) -> bool {
        let wanted = wanted.trim();
        self.name.trim().eq_ignore_ascii_case(wanted)
            || name_without_number(&self.name).eq_ignore_ascii_case(wanted)
            // (What the state tells as the display's connector: on Windows
            // the device's path, which is what a panel names its display
            // by when it asks for one of that display's workspaces.)
            || (!self.device_path.is_empty() && self.device_path.eq_ignore_ascii_case(wanted))
    }

    /// Get the usable tiling area (respecting taskbar)
    pub fn tiling_rect(&self) -> (i32, i32, i32, i32) {
        (self.work_x, self.work_y, self.work_width, self.work_height)
    }

    /// Get work area as Rect for layout calculations
    pub fn work_area_rect(&self) -> Rect {
        Rect::new(self.work_x, self.work_y, self.work_width, self.work_height)
    }
}

/// Area of the intersection of two `(x, y, width, height)` rectangles.
fn overlap_area(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> i64 {
    let width = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let height = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if width <= 0 || height <= 0 {
        0
    } else {
        i64::from(width) * i64::from(height)
    }
}

/// The monitor a window overlaps most (ties go to the first), if any.
fn overlay_owner(
    window: (i32, i32, i32, i32),
    monitors: impl Iterator<Item = (MonitorHandle, (i32, i32, i32, i32))>,
) -> Option<MonitorHandle> {
    let mut best: Option<(MonitorHandle, i64)> = None;
    for (handle, rect) in monitors {
        let area = overlap_area(window, rect);
        if area > 0 && best.is_none_or(|(_, best_area)| area > best_area) {
            best = Some((handle, area));
        }
    }
    best.map(|(handle, _)| handle)
}

/// The settings a `CreateWorkspace` node gives a workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSettings {
    pub layout: crate::layouts::LayoutType,
    pub container_padding_x: i32,
    pub container_padding_y: i32,
    pub workspace_padding_x: i32,
    pub workspace_padding_y: i32,
    pub border_enabled: bool,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_style: gevrek_model::graph::BorderStyle,
    pub active_border_color: (u8, u8, u8),
    pub inactive_border_color: (u8, u8, u8),
    pub active_border_alpha: u8,
    pub inactive_border_alpha: u8,
}

impl WorkspaceSettings {
    pub fn apply_to(&self, workspace: &mut Workspace) {
        workspace.layout = self.layout;
        // Workspace padding (outer margin around the workspace)
        workspace.padding = crate::container::Padding::new(
            self.workspace_padding_x,
            self.workspace_padding_y,
            self.workspace_padding_x,
            self.workspace_padding_y,
        );
        // Container padding (gaps between windows)
        workspace.container_padding = crate::container::Padding::new(
            self.container_padding_x,
            self.container_padding_y,
            self.container_padding_x,
            self.container_padding_y,
        );
        workspace.border_enabled = self.border_enabled;
        workspace.border_width = self.border_width;
        workspace.border_offset = self.border_offset;
        workspace.border_style = self.border_style;
        workspace.active_border_color = self.active_border_color;
        workspace.inactive_border_color = self.inactive_border_color;
        workspace.active_border_alpha = self.active_border_alpha;
        workspace.inactive_border_alpha = self.inactive_border_alpha;
    }
}

/// Why a window is dropped from the workspaces by `remove_stale_windows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleWindow {
    /// The window no longer exists.
    Destroyed,
    /// One of the WM's own border windows (must never be managed).
    Border,
}

/// The title of the window that lists the shortcuts
/// (`gevrek-studio --shortcuts`, `gevrek_studio::shortcuts::TITLE`).
pub const SHORTCUTS_WINDOW_TITLE: &str = "Gevrek Shortcuts";
/// How long the bar's place is kept before a bar has been seen.
const BAR_START_GRACE: std::time::Duration = std::time::Duration::from_secs(20);

/// One mode a display offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_millihertz: i32,
    /// The one the display asks for.
    pub preferred: bool,
    /// The one it is in.
    pub current: bool,
}

/// What a backend that sets modes knows about a display beyond its place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputDetail {
    /// In thousandths of a hertz; 0 when not known.
    pub refresh_millihertz: i32,
    pub modes: Vec<OutputMode>,
}

pub struct Outputs {
    /// What the backend said about its displays' modes (see [`OutputDetail`]).
    details: HashMap<MonitorHandle, OutputDetail>,
    monitors: HashMap<MonitorHandle, MonitorInfo>,
    primary_monitor: Option<MonitorHandle>,
    /// Workspaces per monitor (monitor_handle -> Ring<Workspace>)
    workspaces: HashMap<MonitorHandle, Ring<Workspace>>,
    /// Default number of workspaces to create per monitor
    /// Next workspace ID
    next_workspace_id: usize,
    /// Previously focused workspace per monitor, used by the "back" selector.
    previous_workspace: HashMap<MonitorHandle, usize>,
    /// User-reserved edges applied after Windows' taskbar work area.
    reserved_areas: HashMap<MonitorHandle, ReservedArea>,
    /// The place kept for gevrek's bar on every monitor, if it has one.
    bar: Option<gevrek_model::settings::PanelProfile>,
    /// The displays are measured in their own pixels (Windows), not in
    /// pixels that already count the scale in (a compositor): the lengths
    /// the configuration gives as they are seen - the panel's height and
    /// margin, the gaps, the borders, the tab strip - are then so many
    /// more pixels on a display with a larger scale.
    lengths_in_display_pixels: bool,
    /// Until then the bar's place is kept although no bar is seen yet
    /// (the bar starts after the window manager).
    bar_expected_until: Option<std::time::Instant>,
    /// The shell's panel is on the screen, as the backend says: its
    /// surfaces are no windows to the engine, so they are not among those
    /// it could look through itself.
    panel_seen: bool,
    /// Global work-area offset applied to every monitor after the Windows
    /// work area and the per-monitor reservations.
    global_work_area_offset: ReservedArea,
    /// Room the shell keeps for what it draws on the desktop (zones that
    /// tiled windows keep off), by the display's lasting id, in the
    /// lengths of the configuration. Apart from the person's own
    /// reservations above, which it must not overwrite; taken off after
    /// the panel's room. Empty where the shell's surfaces ask the
    /// compositor for their room themselves (`shell_room.rs`).
    shell_room: HashMap<String, ReservedArea>,
    /// Monitor index preferences: stable display name → desired index in
    /// the ordered monitor ring, applied after discovery ordering.
    index_preferences: Vec<(String, usize)>,
    /// Workspaces of displays that went away, by stable display key,
    /// restored when the display comes back.
    detached: HashMap<String, reconcile::DetachedMonitor>,
}

/// Reorder an already discovery-sorted monitor list so displays named in
/// `preferences` land at their desired ring index.
///
/// Deterministic rules: name matching is case-insensitive; the first
/// preference wins when several entries name the same display; a desired
/// index past the end of the ring clamps to the last slot; monitors
/// competing for one slot keep their relative discovery order (the earlier
/// one takes the contested slot, the later one the next free slot);
/// displays without a preference fill the remaining slots in discovery
/// order.
fn apply_monitor_index_preferences(
    monitors: Vec<MonitorInfo>,
    preferences: &[(String, usize)],
) -> Vec<MonitorInfo> {
    if preferences.is_empty() || monitors.is_empty() {
        return monitors;
    }
    let len = monitors.len();
    let desired_index = |monitor: &MonitorInfo| -> Option<usize> {
        preferences
            .iter()
            .find(|(name, _)| monitor.is_called(name))
            .map(|(_, index)| (*index).min(len - 1))
    };
    let mut preferred: Vec<(usize, MonitorInfo)> = Vec::new();
    let mut remainder: Vec<MonitorInfo> = Vec::new();
    for monitor in monitors {
        match desired_index(&monitor) {
            Some(index) => preferred.push((index, monitor)),
            None => remainder.push(monitor),
        }
    }
    // Stable sort keeps discovery order among equal desired indices.
    preferred.sort_by_key(|(index, _)| *index);
    let mut slots: Vec<Option<MonitorInfo>> = std::iter::repeat_with(|| None).take(len).collect();
    for (index, monitor) in preferred {
        let slot = (index..len)
            .find(|slot| slots[*slot].is_none())
            .or_else(|| (0..index).find(|slot| slots[*slot].is_none()))
            .expect("a monitor ring always has a free slot for a preferred monitor");
        slots[slot] = Some(monitor);
    }
    let mut remainder = remainder.into_iter();
    slots
        .into_iter()
        .map(|slot| {
            slot.unwrap_or_else(|| {
                remainder
                    .next()
                    .expect("every remaining slot has an unpreferred monitor")
            })
        })
        .collect()
}

impl Outputs {
    /// Drop destroyed windows and the backend's own decoration windows
    /// from every workspace. Returns how many were removed.
    pub fn cleanup_stale_windows(&mut self, desktop: &dyn crate::contract::Desktop) -> usize {
        self.remove_stale_windows(|window| {
            if desktop.is_decoration(window) {
                Some(StaleWindow::Border)
            } else if !desktop.exists(window) {
                Some(StaleWindow::Destroyed)
            } else {
                None
            }
        })
    }

    /// The work area of an output minus Gevrek's own overlay bars.
    pub fn get_adjusted_work_area(
        &self,
        monitor: MonitorHandle,
        desktop: &dyn crate::contract::Desktop,
    ) -> Option<Rect> {
        self.work_area_and_shell_room(monitor, desktop)
            .map(|(work_area, _)| work_area)
    }

    /// How much the shell's room takes off each side of a display's work
    /// area, in the work area's own units (pixels where displays are
    /// measured in them), and never more than was there to take: what
    /// [`Outputs::get_adjusted_work_area`] is smaller by for it.
    pub fn shell_room_taken(
        &self,
        monitor: MonitorHandle,
        desktop: &dyn crate::contract::Desktop,
    ) -> ReservedArea {
        self.work_area_and_shell_room(monitor, desktop)
            .map_or(ReservedArea::default(), |(_, taken)| taken)
    }

    /// The work area of an output, and what the shell's room took of it.
    fn work_area_and_shell_room(
        &self,
        monitor: MonitorHandle,
        desktop: &dyn crate::contract::Desktop,
    ) -> Option<(Rect, ReservedArea)> {
        // The bar's place comes from the configuration (`set_bar`), so it is
        // free before the bar is there.
        let mut work_area = self.work_area_excluding_overlays(monitor, &[])?;
        if let Some(bar) = self.bar_on(monitor).filter(|_| self.bar_is_kept(desktop)) {
            // (As many pixels as the panel is high on this display.)
            let room = self.bar_pixels(monitor, bar.reserved());
            let reserved = room.min((work_area.height - 1).max(0));
            use gevrek_model::settings::PanelEdge;
            if bar.edge.is_upright() {
                let reserved = room.min((work_area.width - 1).max(0));
                if bar.edge == PanelEdge::Left {
                    work_area.x += reserved;
                }
                work_area.width -= reserved;
            } else {
                if bar.edge == PanelEdge::Top {
                    work_area.y += reserved;
                }
                work_area.height -= reserved;
            }
        }
        // What the shell keeps for its zones, from the sides of what the
        // panel leaves (as many pixels as it is on this display).
        let room = self.shell_room_of(monitor);
        let left = ReservedArea {
            left: self.bar_pixels(monitor, room.left),
            top: self.bar_pixels(monitor, room.top),
            right: self.bar_pixels(monitor, room.right),
            bottom: self.bar_pixels(monitor, room.bottom),
        }
        .apply_to(work_area);
        let taken = ReservedArea {
            left: left.x - work_area.x,
            top: left.y - work_area.y,
            right: work_area.x + work_area.width - left.x - left.width,
            bottom: work_area.y + work_area.height - left.y - left.height,
        };
        Some((left, taken))
    }

    /// The room the shell keeps on a display now, in the lengths of the
    /// configuration. Nothing for a display it named no room on.
    pub fn shell_room_of(&self, monitor: MonitorHandle) -> ReservedArea {
        self.get_monitor(monitor)
            .filter(|display| !display.stable_id.is_empty())
            .and_then(|display| self.shell_room.get(&display.stable_id))
            .copied()
            .unwrap_or_default()
    }

    /// The shell keeps this room now, and no other (an empty list gives
    /// all of it back: the shell is gone, or what it last said is no
    /// longer to be trusted). Room on a display that is not there is
    /// kept in mind and counts for nothing until that display comes.
    /// The answer: the displays whose room is another than it was, which
    /// are to be laid out again.
    pub fn set_shell_room(&mut self, rooms: &[crate::shell_room::ShellRoom]) -> Vec<MonitorHandle> {
        let before: Vec<(MonitorHandle, ReservedArea)> = self
            .monitors
            .keys()
            .map(|handle| (*handle, self.shell_room_of(*handle)))
            .collect();
        self.shell_room = crate::shell_room::by_display(rooms);
        let mut changed: Vec<MonitorHandle> = before
            .into_iter()
            .filter(|(handle, room)| self.shell_room_of(*handle) != *room)
            .map(|(handle, _)| handle)
            .collect();
        changed.sort_by_key(|handle| handle.0);
        changed
    }

    /// Whether the bar's place is kept now: the configuration has a bar, and
    /// a bar is running or may still be starting. (No bar after a while -
    /// only the window manager was started, or the bar ended - and the
    /// place is given to the windows.)
    pub fn bar_is_kept(&self, _desktop: &dyn crate::contract::Desktop) -> bool {
        if self.bar.is_none() {
            return false;
        }
        if self
            .bar_expected_until
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            return true;
        }
        // (The bar of before the shell was known by its windows' title;
        // it is gone, and the shell's panel says itself that it is there.)
        self.panel_seen
    }

    /// The backend says whether the shell's panel is on the screen (where
    /// the panel does not ask for its own room: on Windows it is put where
    /// this keeps room for it). Returns whether that is news.
    pub fn set_panel_seen(&mut self, seen: bool) -> bool {
        let news = self.panel_seen != seen;
        self.panel_seen = seen;
        news
    }

    /// Do not wait for a bar to show up: its place is kept from the moment
    /// its windows are there, and not before. (Where the panel asks for
    /// its own room, as on Linux, nothing has to be held for it.)
    pub fn expect_no_bar(&mut self) {
        self.bar_expected_until = None;
    }

    /// Keep a place for the bar on every monitor (`None`, or a bar that is
    /// not enabled: no place). Returns whether that changed anything.
    pub fn set_bar(&mut self, bar: Option<gevrek_model::settings::PanelProfile>) -> bool {
        let bar = bar.filter(|bar| bar.enabled);
        if self.bar == bar {
            return false;
        }
        self.bar = bar;
        // A bar that is to come gets a while to show up.
        self.bar_expected_until = self
            .bar
            .as_ref()
            .map(|_| std::time::Instant::now() + BAR_START_GRACE);
        true
    }

    /// Where the bar is on a monitor: inside the area the system and the
    /// user's reservations leave, at its edge.
    /// A backend tells what modes a display offers and which it is in.
    pub fn set_detail(&mut self, monitor: MonitorHandle, detail: OutputDetail) {
        self.details.insert(monitor, detail);
    }

    pub fn forget_detail(&mut self, monitor: MonitorHandle) {
        self.details.remove(&monitor);
    }

    pub fn detail(&self, monitor: MonitorHandle) -> Option<&OutputDetail> {
        self.details.get(&monitor)
    }

    /// The panel of a display: the configuration's, unless it says that
    /// this display has none (`PanelOnDisplay::enabled`). No room is kept
    /// there then, and no panel has a place.
    /// The backend measures the displays in their own pixels: the panel
    /// is as high, and the gaps and borders of a layout as wide, as the
    /// display's scale makes them (see the field).
    pub fn set_lengths_in_display_pixels(&mut self, on: bool) {
        self.lengths_in_display_pixels = on;
    }

    /// A length of the panel (its height, its margin), as the
    /// configuration gives it, in the pixels the display `monitor` is
    /// measured in.
    fn bar_pixels(&self, monitor: MonitorHandle, length: i32) -> i32 {
        (length as f32 * self.length_scale(monitor)).round() as i32
    }

    /// How many pixels of the display `monitor` one pixel of the
    /// configuration is: the display's scale where the displays are
    /// measured in their own pixels, 1 elsewhere.
    pub fn length_scale(&self, monitor: MonitorHandle) -> f32 {
        self.get_monitor(monitor)
            .filter(|_| self.lengths_in_display_pixels)
            .map_or(1.0, |display| display.dpi_scale)
            .clamp(0.5, 8.0)
    }

    fn bar_on(&self, monitor: MonitorHandle) -> Option<&gevrek_model::settings::PanelProfile> {
        let bar = self.bar.as_ref()?;
        let none_here = self.get_monitor(monitor).is_some_and(|display| {
            bar.displays.iter().any(|entry| {
                !entry.enabled && !entry.name.trim().is_empty() && display.is_called(&entry.name)
            })
        });
        (!none_here).then_some(bar)
    }

    pub fn bar_rect(&self, monitor: MonitorHandle) -> Option<Rect> {
        let bar = self.bar_on(monitor)?;
        let area = self.get_configured_work_area(monitor)?;
        let margin = self.bar_pixels(monitor, bar.margin.min(100) as i32);
        let height = self.bar_pixels(monitor, bar.height.clamp(16, 200) as i32);
        use gevrek_model::settings::PanelEdge;
        // (`height` is the panel's thickness: across a panel that goes
        // down a side.)
        Some(match bar.edge {
            PanelEdge::Top | PanelEdge::Bottom => Rect::new(
                area.x + margin,
                if bar.edge == PanelEdge::Top {
                    area.y + margin
                } else {
                    area.y + area.height - margin - height
                },
                (area.width - margin * 2).max(1),
                height,
            ),
            PanelEdge::Left | PanelEdge::Right => Rect::new(
                if bar.edge == PanelEdge::Left {
                    area.x + margin
                } else {
                    area.x + area.width - margin - height
                },
                area.y + margin,
                height,
                (area.height - margin * 2).max(1),
            ),
        })
    }

    /// Apply the focused workspace's layout on an output.
    pub fn apply_workspace_layout(
        &mut self,
        monitor: MonitorHandle,
        desktop: &dyn crate::contract::Desktop,
    ) {
        let Some(work_area) = self.get_adjusted_work_area(monitor, desktop) else {
            return;
        };
        if let Some(workspace) = self.get_focused_workspace_mut(monitor) {
            workspace.apply_layout(work_area);
            log::debug!(
                "Applied layout {:?} to workspace {} on monitor {:?}",
                workspace.layout,
                workspace.name,
                monitor
            );
        }
    }

    pub fn new() -> Self {
        Self::with_workspace_count(1) // Default 1 workspace per monitor (BSP)
    }

    /// The count has no effect: workspaces come from the configuration
    /// (or `ensure_default_workspaces`). Kept for its callers.
    pub fn with_workspace_count(_count: usize) -> Self {
        Self {
            monitors: HashMap::new(),
            primary_monitor: None,
            workspaces: HashMap::new(),
            next_workspace_id: 0,
            previous_workspace: HashMap::new(),
            reserved_areas: HashMap::new(),
            details: HashMap::new(),
            bar: None,
            lengths_in_display_pixels: false,
            bar_expected_until: None,
            panel_seen: false,
            global_work_area_offset: ReservedArea::default(),
            shell_room: HashMap::new(),
            index_preferences: Vec::new(),
            detached: HashMap::new(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn add_test_monitor(&mut self, handle: MonitorHandle, name: &str) {
        self.monitors.insert(
            handle,
            MonitorInfo {
                handle,
                name: name.to_string(),
                device_path: String::new(),
                stable_id: edid::stable_id_of_place(name),
                is_primary: self.primary_monitor.is_none(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                work_x: 0,
                work_y: 0,
                work_width: 1920,
                work_height: 1040,
                dpi_scale: 1.0,
            },
        );
        self.primary_monitor.get_or_insert(handle);
        self.workspaces.entry(handle).or_default();
    }

    /// Install a monitor set for display purposes only (no workspace changes).
    pub fn set_monitors_only(&mut self, current: Vec<MonitorInfo>) {
        self.primary_monitor = current
            .iter()
            .find(|info| info.is_primary)
            .map(|info| info.handle);
        self.monitors = current
            .into_iter()
            .map(|info| (info.handle, info))
            .collect();
    }

    /// Install a freshly enumerated monitor set (without creating workspaces).
    /// Workspaces should be created by OnStartup nodes or ensure_default_workspaces()
    pub fn replace_monitors(&mut self, current: Vec<MonitorInfo>) {
        // log::info!("=== Refreshing Monitor Configuration ===");

        self.monitors.clear();
        self.primary_monitor = None;

        self.monitors = current
            .into_iter()
            .map(|info| (info.handle, info))
            .collect();

        // Find primary monitor
        for (handle, info) in &self.monitors {
            if info.is_primary {
                self.primary_monitor = Some(*handle);
                log::info!(
                    "Primary monitor: {} ({}x{})",
                    info.name,
                    info.width,
                    info.height
                );
            }
        }

        // Remove workspaces for monitors that no longer exist
        self.workspaces
            .retain(|handle, _| self.monitors.contains_key(handle));
        self.reserved_areas
            .retain(|handle, _| self.monitors.contains_key(handle));

        // Initialize empty workspace storage for newly detected monitors
        for (handle, info) in &self.monitors {
            if !self.workspaces.contains_key(handle) {
                log::debug!(
                    "Initializing workspace storage for monitor '{}' (handle {:?})",
                    info.name,
                    handle
                );
                self.workspaces.insert(*handle, crate::ring::Ring::new());
            }
        }

        log::info!("Detected {} monitor(s)", self.monitors.len());
        // for (handle, info) in &self.monitors {
        //     let ws_count = self.workspaces.get(handle).map(|r| r.len()).unwrap_or(0);
        //     log::info!("  Monitor '{}': {}x{} at ({}, {}), Work area: {}x{} at ({}, {}), {} workspaces",
        //         info.name,
        //         info.width, info.height, info.x, info.y,
        //         info.work_width, info.work_height, info.work_x, info.work_y,
        //         ws_count);
        // }
    }

    pub fn get_monitor_count(&self) -> usize {
        self.monitors.len()
    }

    pub fn get_monitor(&self, handle: MonitorHandle) -> Option<&MonitorInfo> {
        self.monitors.get(&handle)
    }

    pub fn get_all_monitors(&self) -> Vec<MonitorInfo> {
        self.get_all_monitors_ordered()
    }

    pub fn get_all_monitors_ordered(&self) -> Vec<MonitorInfo> {
        let mut monitors = self.monitors.values().cloned().collect::<Vec<_>>();
        monitors.sort_by_key(|m| (!m.is_primary, m.x, m.y, m.name.clone()));
        apply_monitor_index_preferences(monitors, &self.index_preferences)
    }

    /// Replace the monitor index preferences applied after discovery
    /// ordering. Entries are
    /// `(stable display name, desired ring index)`.
    pub fn set_monitor_index_preferences(&mut self, preferences: Vec<(String, usize)>) {
        self.index_preferences = preferences;
    }

    pub fn get_first_secondary_monitor_handle(&self) -> Option<MonitorHandle> {
        self.get_all_monitors_ordered()
            .into_iter()
            .find(|m| !m.is_primary)
            .map(|m| m.handle)
    }

    pub fn get_monitors_with_handles(&self) -> Vec<(MonitorHandle, MonitorInfo)> {
        self.monitors.iter().map(|(h, i)| (*h, i.clone())).collect()
    }

    pub fn get_primary_monitor(&self) -> Option<MonitorInfo> {
        self.primary_monitor
            .and_then(|h| self.monitors.get(&h).cloned())
    }

    /// Get primary monitor handle
    pub fn get_primary_monitor_handle(&self) -> Option<MonitorHandle> {
        self.primary_monitor
    }

    // ===== Workspace Management =====

    /// Get workspaces for a monitor
    pub fn get_workspaces(&self, monitor: MonitorHandle) -> Option<&Ring<Workspace>> {
        self.workspaces.get(&monitor)
    }

    /// Get mutable workspaces for a monitor
    pub fn get_workspaces_mut(&mut self, monitor: MonitorHandle) -> Option<&mut Ring<Workspace>> {
        // (Whoever lays a workspace out gets it here: it is told the
        // scale of the display it is on now, see `Workspace::scale`.)
        let scale = self.length_scale(monitor);
        let workspaces = self.workspaces.get_mut(&monitor)?;
        for workspace in workspaces.iter_mut() {
            workspace.scale = scale;
        }
        Some(workspaces)
    }

    /// Get the focused workspace on a monitor
    pub fn get_focused_workspace(&self, monitor: MonitorHandle) -> Option<&Workspace> {
        self.workspaces.get(&monitor)?.focused()
    }

    /// Get mutable focused workspace on a monitor
    pub fn get_focused_workspace_mut(&mut self, monitor: MonitorHandle) -> Option<&mut Workspace> {
        self.get_workspaces_mut(monitor)?.focused_mut()
    }

    /// Move the focused workspace from one monitor to another and focus it
    /// there. If it was the source monitor's only workspace, materialize a
    /// replacement so every attached monitor remains usable.
    pub fn move_focused_workspace(
        &mut self,
        source: MonitorHandle,
        target: MonitorHandle,
    ) -> Result<usize> {
        if source == target {
            anyhow::bail!("Source and target monitor are the same");
        }
        if !self.monitors.contains_key(&source) || !self.monitors.contains_key(&target) {
            anyhow::bail!("Source or target monitor is no longer attached");
        }
        if self
            .workspaces
            .get(&source)
            .is_none_or(|workspaces| workspaces.is_empty())
        {
            anyhow::bail!("The source monitor has no active workspace");
        }

        if self.workspaces[&source].len() == 1 {
            let existing_names = self.workspaces[&source]
                .iter()
                .map(|workspace| workspace.name.to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>();
            let mut ordinal = 1;
            let replacement_name = loop {
                let name = format!("Workspace {ordinal}");
                if !existing_names.contains(&name.to_ascii_lowercase()) {
                    break name;
                }
                ordinal += 1;
            };
            let replacement = Workspace::new(self.next_workspace_id, replacement_name);
            self.next_workspace_id += 1;
            self.workspaces
                .get_mut(&source)
                .expect("source workspace ring was validated")
                .push(replacement);
        }

        let moved = self
            .workspaces
            .get_mut(&source)
            .and_then(Ring::remove)
            .ok_or_else(|| anyhow::anyhow!("The source workspace disappeared"))?;
        let target_ring = self
            .workspaces
            .get_mut(&target)
            .ok_or_else(|| anyhow::anyhow!("The target monitor has no workspace ring"))?;
        target_ring.push(moved);
        let target_index = target_ring.len() - 1;
        target_ring.focus_at(target_index);
        self.previous_workspace.remove(&source);
        self.previous_workspace.remove(&target);
        Ok(target_index)
    }

    /// Exchange the focused workspaces of two monitors while preserving each
    /// monitor's workspace-ring position.
    pub fn swap_focused_workspaces(
        &mut self,
        first: MonitorHandle,
        second: MonitorHandle,
    ) -> Result<()> {
        if first == second {
            anyhow::bail!("Both monitor selectors resolve to the same monitor");
        }
        if self
            .workspaces
            .get(&first)
            .and_then(Ring::focused)
            .is_none()
            || self
                .workspaces
                .get(&second)
                .and_then(Ring::focused)
                .is_none()
        {
            anyhow::bail!("Both monitors must have an active workspace");
        }

        let mut second_ring = self
            .workspaces
            .remove(&second)
            .expect("second workspace ring was validated");
        let first_ring = self
            .workspaces
            .get_mut(&first)
            .expect("first workspace ring was validated");
        std::mem::swap(
            first_ring
                .focused_mut()
                .expect("first focused workspace was validated"),
            second_ring
                .focused_mut()
                .expect("second focused workspace was validated"),
        );
        self.workspaces.insert(second, second_ring);
        self.previous_workspace.remove(&first);
        self.previous_workspace.remove(&second);
        Ok(())
    }

    /// Switch to next workspace on a monitor
    pub fn focus_next_workspace(&mut self, monitor: MonitorHandle) {
        if let Some((current, len)) = self
            .workspaces
            .get(&monitor)
            .map(|workspaces| (workspaces.focused_idx(), workspaces.len()))
        {
            if len > 0 {
                self.focus_workspace_at(monitor, (current + 1) % len);
            }
        }
    }

    /// Switch to previous workspace on a monitor
    /// Forget which workspace was shown before on a monitor (it is gone).
    pub fn forget_previous_workspace(&mut self, monitor: MonitorHandle) {
        self.previous_workspace.remove(&monitor);
    }

    pub fn focus_previous_workspace(&mut self, monitor: MonitorHandle) {
        if let Some((current, len)) = self
            .workspaces
            .get(&monitor)
            .map(|workspaces| (workspaces.focused_idx(), workspaces.len()))
        {
            if len > 0 {
                self.focus_workspace_at(monitor, (current + len - 1) % len);
            }
        }
    }

    /// Focus specific workspace by index on a monitor
    pub fn focus_workspace_at(&mut self, monitor: MonitorHandle, idx: usize) -> bool {
        if let Some(workspaces) = self.workspaces.get_mut(&monitor) {
            let previous = workspaces.focused_idx();
            let result = workspaces.focus_at(idx);
            if result {
                if previous != idx {
                    self.previous_workspace.insert(monitor, previous);
                }
                log::info!(
                    "Switched to workspace {} on monitor {:?}",
                    workspaces
                        .focused()
                        .map(|w| w.name.as_str())
                        .unwrap_or("unknown"),
                    monitor
                );
            }
            result
        } else {
            false
        }
    }

    /// Create a new workspace on a monitor
    /// Returns the index of the newly created workspace, or None if monitor not found
    pub fn create_workspace(
        &mut self,
        monitor: MonitorHandle,
        name: String,
        layout: crate::layouts::LayoutType,
    ) -> Option<usize> {
        if let Some(workspaces) = self.workspaces.get_mut(&monitor) {
            let mut workspace =
                crate::workspace::Workspace::new(self.next_workspace_id, name.clone());
            workspace.layout = layout;
            workspaces.push(workspace);
            let new_index = workspaces.len() - 1; // Index of the newly added workspace
            self.next_workspace_id += 1;
            log::info!(
                "Created workspace '{}' with layout {:?} at index {} on monitor {:?}",
                name,
                layout,
                new_index,
                monitor
            );
            Some(new_index)
        } else {
            log::warn!("Cannot create workspace: monitor {:?} not found", monitor);
            None
        }
    }

    /// Remove a workspace only when it is inactive, empty, and another
    /// workspace remains on the monitor.
    ///
    /// This is the lifecycle primitive used by non-persistent workspace
    /// profiles. The checks live here so no caller can accidentally remove the
    /// active workspace or orphan its windows.
    pub fn remove_inactive_empty_workspace(
        &mut self,
        monitor: MonitorHandle,
        index: usize,
    ) -> bool {
        let Some(workspaces) = self.workspaces.get_mut(&monitor) else {
            return false;
        };
        if workspaces.len() <= 1 || workspaces.focused_idx() == index {
            return false;
        }
        if workspaces
            .get(index)
            .is_none_or(|workspace| !workspace.all_windows().is_empty())
        {
            return false;
        }
        let Some(workspace) = workspaces.remove_at(index) else {
            return false;
        };
        // The workspace that was visited before: none if it was this one,
        // one place earlier if it came after it.
        match self.previous_workspace.get(&monitor).copied() {
            Some(previous) if previous == index => {
                self.previous_workspace.remove(&monitor);
            }
            Some(previous) if previous > index => {
                self.previous_workspace.insert(monitor, previous - 1);
            }
            _ => {}
        }
        log::info!(
            "Removed inactive empty workspace '{}' from monitor {:?}",
            workspace.name,
            monitor
        );
        true
    }

    /// Put a monitor's workspaces in an order: those `place` gives a
    /// place, by it; the others after them, as they were among each other.
    /// The workspace that is shown stays the one shown, and the one that
    /// was visited before stays that. Returns whether anything moved.
    pub fn order_workspaces(
        &mut self,
        monitor: MonitorHandle,
        place: impl Fn(&crate::workspace::Workspace) -> Option<usize>,
    ) -> bool {
        let Some(workspaces) = self.workspaces.get_mut(&monitor) else {
            return false;
        };
        let before: Vec<usize> = workspaces.iter().map(|workspace| workspace.id).collect();
        let shown = workspaces.focused().map(|workspace| workspace.id);
        let previous = self
            .previous_workspace
            .get(&monitor)
            .and_then(|index| before.get(*index).copied());
        // (A stable sort: what has no place keeps its order, after the
        // rest.)
        workspaces
            .elements_mut()
            .sort_by_key(|workspace| place(workspace).map_or((1, 0), |place| (0, place)));
        let after: Vec<usize> = workspaces.iter().map(|workspace| workspace.id).collect();
        if after == before {
            return false;
        }
        let at = |id: usize| after.iter().position(|other| *other == id);
        if let Some(index) = shown.and_then(at) {
            workspaces.focus_at(index);
        }
        match previous.and_then(at) {
            Some(index) => {
                self.previous_workspace.insert(monitor, index);
            }
            None => {
                self.previous_workspace.remove(&monitor);
            }
        }
        true
    }

    /// Close the focused workspace when it is empty and another workspace
    /// remains on the monitor. Returns the
    /// closed workspace's name. Focus advances to the workspace that shifts
    /// into the freed slot.
    pub fn close_focused_empty_workspace(&mut self, monitor: MonitorHandle) -> Option<String> {
        let workspaces = self.workspaces.get_mut(&monitor)?;
        if workspaces.len() <= 1 {
            return None;
        }
        let index = workspaces.focused_idx();
        if workspaces
            .get(index)
            .is_none_or(|workspace| !workspace.all_windows().is_empty())
        {
            return None;
        }
        let removed = workspaces.remove_at(index)?;
        self.previous_workspace.remove(&monitor);
        log::info!(
            "Closed empty workspace '{}' on monitor {:?}",
            removed.name,
            monitor
        );
        Some(removed.name)
    }

    /// Create a new workspace on a monitor with custom padding
    /// Returns the index of the newly created workspace, or None if monitor not found
    pub fn create_workspace_with_padding(
        &mut self,
        monitor: MonitorHandle,
        name: String,
        settings: &WorkspaceSettings,
    ) -> Option<usize> {
        let WorkspaceSettings {
            layout,
            container_padding_x,
            container_padding_y,
            workspace_padding_x,
            workspace_padding_y,
            ..
        } = *settings;
        if let Some(workspaces) = self.workspaces.get_mut(&monitor) {
            let mut workspace =
                crate::workspace::Workspace::new(self.next_workspace_id, name.clone());
            settings.apply_to(&mut workspace);
            workspaces.push(workspace);
            let new_index = workspaces.len() - 1; // Index of the newly added workspace
            self.next_workspace_id += 1;
            log::info!("Created workspace '{}' with layout {:?}, container padding (x:{}, y:{}), and workspace padding (x:{}, y:{}) at index {} on monitor {:?}",
                name, layout, container_padding_x, container_padding_y, workspace_padding_x, workspace_padding_y, new_index, monitor);
            Some(new_index)
        } else {
            log::warn!("Cannot create workspace: monitor {:?} not found", monitor);
            None
        }
    }

    /// Monitor and ring index of the workspace with this (unique) id.
    pub fn find_workspace_by_id(&self, id: usize) -> Option<(MonitorHandle, usize)> {
        self.workspaces.iter().find_map(|(monitor, workspaces)| {
            workspaces
                .elements()
                .iter()
                .position(|workspace| workspace.id == id)
                .map(|index| (*monitor, index))
        })
    }

    /// Index of the workspace named `name` (case-insensitive) on `monitor`.
    pub fn find_workspace_index(&self, monitor: MonitorHandle, name: &str) -> Option<usize> {
        self.workspaces
            .get(&monitor)?
            .elements()
            .iter()
            .position(|workspace| workspace.name.eq_ignore_ascii_case(name))
    }

    /// Give an existing workspace new node settings; its windows, focus,
    /// and resize state are kept.
    pub fn update_workspace_settings(
        &mut self,
        monitor: MonitorHandle,
        index: usize,
        settings: &WorkspaceSettings,
    ) -> bool {
        let Some(workspace) = self
            .workspaces
            .get_mut(&monitor)
            .and_then(|workspaces| workspaces.get_mut(index))
        else {
            return false;
        };
        settings.apply_to(workspace);
        true
    }

    /// Clean up stale windows from all workspaces
    /// Returns the number of windows removed
    pub fn remove_stale_windows(
        &mut self,
        stale: impl Fn(crate::WindowHandle) -> Option<StaleWindow>,
    ) -> usize {
        let mut total_removed = 0;
        for (monitor, workspaces_ring) in self.workspaces.iter_mut() {
            for workspace in workspaces_ring.elements_mut() {
                let workspace_windows: Vec<_> = workspace.all_windows();
                for window in workspace_windows {
                    let reason = stale(window);
                    let is_border = reason == Some(StaleWindow::Border);
                    let window_exists = reason != Some(StaleWindow::Destroyed);

                    if reason.is_some() && workspace.remove_window(window) {
                        if is_border {
                            log::warn!("⚠️  Removed BORDER window {:?} from workspace '{}' on monitor {:?} (should never happen!)",
                                    window, workspace.name, monitor);
                        } else if !window_exists {
                            log::info!("Removed non-existent/destroyed window {:?} from workspace '{}' on monitor {:?}",
                                    window, workspace.name, monitor);
                        }
                        total_removed += 1;
                    }
                }
            }
        }
        total_removed
    }

    /// Reset all workspaces (called during hot-reload to clear old workspaces before recreating)
    pub fn reset_workspaces(&mut self) {
        log::info!("Resetting all workspaces");
        for (monitor, workspaces) in self.workspaces.iter_mut() {
            let count = workspaces.len();
            workspaces.clear();
            log::debug!("Cleared {} workspaces from monitor {:?}", count, monitor);
        }
        self.next_workspace_id = 1; // Reset workspace ID counter
        self.previous_workspace.clear();
    }

    pub fn clear_reserved_areas(&mut self) {
        self.reserved_areas.clear();
    }

    pub fn set_reserved_area(&mut self, monitor: MonitorHandle, reserved: ReservedArea) -> bool {
        if !self.monitors.contains_key(&monitor) {
            return false;
        }
        self.reserved_areas.insert(monitor, reserved);
        true
    }

    pub fn get_reserved_area(&self, monitor: MonitorHandle) -> ReservedArea {
        self.reserved_areas
            .get(&monitor)
            .copied()
            .unwrap_or_default()
    }

    /// Set the global work-area offset applied to every monitor after the
    /// per-monitor reservations.
    pub fn set_global_work_area_offset(&mut self, offset: ReservedArea) {
        self.global_work_area_offset = offset;
    }

    pub fn global_work_area_offset(&self) -> ReservedArea {
        self.global_work_area_offset
    }

    pub fn get_configured_work_area(&self, monitor: MonitorHandle) -> Option<Rect> {
        let monitor_info = self.get_monitor(monitor)?;
        Some(
            self.global_work_area_offset.apply_to(
                self.get_reserved_area(monitor)
                    .apply_to(monitor_info.work_area_rect()),
            ),
        )
    }

    /// Work area of a monitor minus the overlay bars (the backend finds them).
    pub fn work_area_excluding_overlays(
        &self,
        monitor: MonitorHandle,
        overlays: &[Rect],
    ) -> Option<Rect> {
        let monitor_info = self.get_monitor(monitor)?;
        let mut work_area = self.get_configured_work_area(monitor)?;

        for window in overlays {
            {
                // The overlay belongs to the monitor it overlaps most. Its
                // center can fall outside every monitor (different heights or
                // offsets), and then no monitor reserved space for it.
                let overlay = (window.x, window.y, window.width, window.height);
                let is_on_monitor = overlay_owner(
                    overlay,
                    self.monitors
                        .values()
                        .map(|info| (info.handle, (info.x, info.y, info.width, info.height))),
                ) == Some(monitor);

                if is_on_monitor {
                    // Determine if overlay is at top or bottom
                    let overlay_at_top = window.y <= monitor_info.y + 5; // Within 5px of monitor top
                    let overlay_at_bottom =
                        (window.y + window.height) >= (monitor_info.y + monitor_info.height - 5);

                    if overlay_at_top {
                        // Overlay at top - reduce work area from top
                        let overlay_height = window.height;
                        work_area.y += overlay_height;
                        work_area.height -= overlay_height;
                        log::debug!(
                            "Adjusted work area for top overlay ({}px) on monitor {:?}",
                            overlay_height,
                            monitor
                        );
                    } else if overlay_at_bottom {
                        // Overlay at bottom - reduce work area from bottom
                        let overlay_height = window.height;
                        work_area.height -= overlay_height;
                        log::debug!(
                            "Adjusted work area for bottom overlay ({}px) on monitor {:?}",
                            overlay_height,
                            monitor
                        );
                    }
                }
            }
        }

        Some(work_area)
    }

    /// Find and remove a window from any workspace on any monitor
    /// Returns Some((monitor_handle, workspace_index)) if found and removed, None otherwise
    /// Check if a window is currently floating (not tiled)
    pub fn is_window_floating(&self, handle: crate::WindowHandle) -> bool {
        for workspaces_ring in self.workspaces.values() {
            for workspace in workspaces_ring.iter() {
                // Check if window is in floating_windows list
                if workspace.floating_windows.iter().any(|w| *w == handle) {
                    return true;
                }
            }
        }
        false
    }

    /// The windows that stand on a strip (the tiled windows of workspaces
    /// under the scrolling layout), each with the display its strip is
    /// on. Such a window may reach beyond its display's edge; it is the
    /// strip's display alone that shows it.
    pub fn strip_windows(&self) -> Vec<(crate::WindowHandle, MonitorHandle)> {
        let mut found = Vec::new();
        for (monitor, ring) in self.workspaces.iter() {
            for workspace in ring.iter() {
                if workspace.layout != crate::layouts::LayoutType::Scrolling {
                    continue;
                }
                for container in workspace.containers.iter() {
                    found.extend(
                        container
                            .all_windows()
                            .into_iter()
                            .map(|window| (window, *monitor)),
                    );
                }
            }
        }
        found
    }

    /// Find a window in any workspace on any monitor
    /// Returns Some((monitor_handle, workspace_index)) if found, None otherwise
    pub fn find_window(&self, handle: crate::WindowHandle) -> Option<(MonitorHandle, usize)> {
        for (monitor_handle, workspaces_ring) in self.workspaces.iter() {
            for (workspace_idx, workspace) in workspaces_ring.iter().enumerate() {
                // Check both tiled and floating windows
                let in_containers = workspace
                    .containers
                    .iter()
                    .any(|c| c.contains_window(handle));
                let in_floating = workspace.floating_windows.iter().any(|w| *w == handle);

                if in_containers || in_floating {
                    return Some((*monitor_handle, workspace_idx));
                }
            }
        }
        None
    }

    pub fn find_and_remove_window(
        &mut self,
        handle: crate::WindowHandle,
    ) -> Option<(MonitorHandle, usize)> {
        for (monitor_handle, workspaces_ring) in self.workspaces.iter_mut() {
            for (workspace_idx, workspace) in workspaces_ring.iter_mut().enumerate() {
                if workspace.remove_window(handle) {
                    log::debug!(
                        "Removed window from workspace '{}' (index {}) on monitor {:?}",
                        workspace.name,
                        workspace_idx,
                        monitor_handle
                    );
                    return Some((*monitor_handle, workspace_idx));
                }
            }
        }
        None
    }

    pub fn get_previous_workspace_index(&self, monitor: MonitorHandle) -> Option<usize> {
        self.previous_workspace.get(&monitor).copied()
    }
}

impl Default for Outputs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(handle: isize, name: &str, path: &str, x: i32) -> MonitorInfo {
        MonitorInfo {
            handle: MonitorHandle(handle),
            name: name.to_string(),
            device_path: path.to_string(),
            stable_id: String::new(),
            is_primary: handle == 1,
            x,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: x,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            dpi_scale: 1.0,
        }
    }

    #[test]
    fn the_bars_place_is_kept_while_the_backend_sees_the_shells_panel() {
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = Outputs::new();
        // No bar in the configuration: nothing is kept, panel or not.
        outputs.set_panel_seen(true);
        assert!(!outputs.bar_is_kept(&desktop));
        outputs.set_bar(Some(gevrek_model::settings::PanelProfile::default()));
        // The while a bar is given to show up is over, and none came.
        outputs.expect_no_bar();
        assert!(outputs.set_panel_seen(false), "that is news");
        assert!(!outputs.bar_is_kept(&desktop));
        // The panel is no window among the desktop's: the backend says so.
        assert!(outputs.set_panel_seen(true));
        assert!(!outputs.set_panel_seen(true), "said twice is no news");
        assert!(outputs.bar_is_kept(&desktop));
    }

    #[test]
    fn a_display_the_configuration_gives_no_panel_keeps_no_room_for_one() {
        use gevrek_model::settings::{PanelOnDisplay, PanelProfile};
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = Outputs::new();
        outputs.set_monitors_only(vec![
            named(1, "Monitor A", "DP-1", 0),
            named(2, "Monitor B", "DP-2", 1920),
        ]);
        let bar = PanelProfile {
            // (By the display's device, as well as by its name.)
            displays: vec![PanelOnDisplay {
                name: "dp-2".to_string(),
                enabled: false,
                ..PanelOnDisplay::default()
            }],
            ..PanelProfile::default()
        };
        let kept = bar.reserved();
        assert!(kept > 0);
        outputs.set_bar(Some(bar));
        outputs.set_panel_seen(true);
        let (first, second) = (MonitorHandle(1), MonitorHandle(2));
        // The first has its panel, and the windows the room beside it.
        assert!(outputs.bar_rect(first).is_some());
        let with = outputs.get_adjusted_work_area(first, &desktop).unwrap();
        assert_eq!(with.height, 1040 - kept);
        // The second has none: no place for one, and all the room.
        assert!(outputs.bar_rect(second).is_none());
        let without = outputs.get_adjusted_work_area(second, &desktop).unwrap();
        assert_eq!(without.height, 1040);
    }

    /// Where the displays are measured in their own pixels, the panel is
    /// as many pixels high as its display's scale makes it, and so much
    /// room is kept for it. (At 150 % what the panel shows was half as
    /// high again as the panel, and was cut off.)
    #[test]
    fn the_panel_is_as_high_as_its_displays_scale_makes_it() {
        use gevrek_model::settings::PanelProfile;
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = Outputs::new();
        let mut large = named(2, "Monitor B", "DP-2", 1920);
        large.dpi_scale = 1.5;
        outputs.set_monitors_only(vec![named(1, "Monitor A", "DP-1", 0), large]);
        let bar = PanelProfile {
            height: 36,
            margin: 4,
            ..PanelProfile::default()
        };
        outputs.set_bar(Some(bar));
        outputs.set_panel_seen(true);
        let (first, second) = (MonitorHandle(1), MonitorHandle(2));
        let room = |outputs: &Outputs, monitor| {
            1040 - outputs.get_adjusted_work_area(monitor, &desktop).unwrap().height
        };

        // Pixels that count the scale in already (a compositor): as given.
        assert_eq!(outputs.bar_rect(second).unwrap().height, 36);
        assert_eq!(room(&outputs, second), 40);

        // The displays' own pixels: by each display's scale.
        outputs.set_lengths_in_display_pixels(true);
        assert_eq!(outputs.bar_rect(first).unwrap().height, 36);
        assert_eq!(room(&outputs, first), 40);
        let place = outputs.bar_rect(second).unwrap();
        assert_eq!(place.height, 54);
        assert_eq!((place.x, place.y), (1920 + 6, 6), "its margin too");
        assert_eq!(room(&outputs, second), 60);
    }

    /// The gaps, the border and the tab strip of a layout are as many
    /// pixels as the scale of the display makes them, and follow the
    /// scale when it is changed while the windows are there; what the
    /// configuration says stays what it is.
    #[test]
    fn a_layout_is_laid_out_by_its_displays_scale() {
        use crate::container::Padding;
        use crate::layouts::LayoutType;
        let mut outputs = Outputs::with_workspace_count(0);
        let display = MonitorHandle(1);
        outputs.add_test_monitor(display, "Monitor A");
        outputs.set_lengths_in_display_pixels(true);
        outputs
            .create_workspace(display, "one".to_string(), LayoutType::Columns)
            .expect("a workspace");
        outputs.focus_workspace_at(display, 0);
        let area = Rect::new(0, 0, 1920, 1040);
        // The room at the left, between the two windows, and how wide the
        // border and how high the tab strip are.
        let laid_out = |outputs: &mut Outputs| {
            let workspace = outputs.get_focused_workspace_mut(display).unwrap();
            workspace.apply_layout(area);
            let rects: Vec<Rect> = workspace.containers.iter().map(|c| c.rect).collect();
            (
                rects[0].x,
                rects[1].x - (rects[0].x + rects[0].width),
                workspace.border_look(false).width,
                workspace.stackbar_seen().height,
            )
        };
        {
            let workspace = outputs.get_focused_workspace_mut(display).unwrap();
            workspace.padding = Padding::uniform(10);
            workspace.container_padding = Padding::uniform(8);
            workspace.border_width = 2;
            workspace.stackbar.height = 30;
            workspace.add_window(gevrek_model::ids::WindowId(1), true);
            workspace.add_window(gevrek_model::ids::WindowId(2), true);
        }
        assert_eq!(laid_out(&mut outputs), (14, 8, 2, 30));

        // The display is given another scale while the windows are there.
        let mut larger = outputs.get_monitor(display).cloned().unwrap();
        larger.dpi_scale = 1.5;
        assert!(outputs.reconcile_with(vec![larger]).geometry_changed);
        assert_eq!(laid_out(&mut outputs), (21, 12, 3, 45));
        let workspace = outputs.get_focused_workspace(display).unwrap();
        assert_eq!(workspace.padding, Padding::uniform(10), "as configured");
        assert_eq!(workspace.container_padding, Padding::uniform(8));

        // Pixels that count the scale in already: as given.
        outputs.set_lengths_in_display_pixels(false);
        assert_eq!(laid_out(&mut outputs), (14, 8, 2, 30));
    }

    #[test]
    fn displays_with_the_same_id_are_told_apart_by_where_they_are_plugged_in() {
        let with_id = |handle, path: &str, id: &str| {
            let mut display = named(handle, "LG ULTRAGEAR", path, 0);
            display.stable_id = id.to_string();
            display
        };
        let mut monitors = vec![
            with_id(1, "port-b", "GSM-5B7F-noserial"),
            with_id(2, "port-c", "GSM-5C4D-0004E2A1"),
            with_id(3, "port-a", "GSM-5B7F-noserial"),
            // (A display the backend gave no id.)
            with_id(4, "port-d", ""),
            with_id(5, "port-e", ""),
        ];
        number_same_stable_ids(&mut monitors);
        let ids: Vec<&str> = monitors
            .iter()
            .map(|monitor| monitor.stable_id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "GSM-5B7F-noserial#2",
                "GSM-5C4D-0004E2A1",
                "GSM-5B7F-noserial#1",
                "",
                ""
            ]
        );
    }

    #[test]
    fn a_test_monitor_is_known_by_its_place() {
        let mut outputs = Outputs::new();
        outputs.add_test_monitor(MonitorHandle(1), "Monitor A");
        let display = outputs.get_monitor(MonitorHandle(1)).unwrap();
        assert_eq!(display.stable_id, "port:Monitor A");
    }

    #[test]
    fn displays_of_one_model_get_a_number_each_and_keep_it() {
        let mut monitors = vec![
            named(1, "DELL U2720Q", "port-c", 0),
            named(2, "Laptop", "internal", 1920),
            named(3, "dell u2720q", "port-a", 3840),
            named(4, "DELL U2720Q", "port-b", 5760),
        ];
        number_same_names(&mut monitors);
        let names: Vec<&str> = monitors
            .iter()
            .map(|monitor| monitor.name.as_str())
            .collect();
        // By the port, not by the order they were found in or where they stand.
        assert_eq!(
            names,
            [
                "DELL U2720Q (3)",
                "Laptop",
                "dell u2720q (1)",
                "DELL U2720Q (2)"
            ]
        );

        // Found in another order, standing elsewhere: the same numbers.
        let mut again = vec![
            named(9, "DELL U2720Q", "port-b", 0),
            named(8, "DELL U2720Q", "port-a", 1920),
            named(7, "DELL U2720Q", "port-c", 3840),
        ];
        number_same_names(&mut again);
        let names: Vec<&str> = again.iter().map(|monitor| monitor.name.as_str()).collect();
        assert_eq!(
            names,
            ["DELL U2720Q (2)", "DELL U2720Q (1)", "DELL U2720Q (3)"]
        );
    }

    #[test]
    fn a_display_is_called_by_its_name_with_or_without_its_number() {
        let twin = named(1, "DELL U2720Q (2)", "port-b", 0);
        assert!(twin.is_called("dell u2720q (2)"));
        assert!(
            twin.is_called(" DELL U2720Q "),
            "the model's name means each of them"
        );
        assert!(!twin.is_called("DELL U2720Q (1)"));
        assert!(!twin.is_called("DELL"));
        let alone = named(2, "Laptop", "internal", 0);
        assert!(alone.is_called("laptop") && !alone.is_called("Laptop (1)"));

        assert_eq!(
            name_without_number("Monitor ACM2702 (12)"),
            "Monitor ACM2702"
        );
        // Only a number in brackets at the very end is one.
        assert_eq!(name_without_number("Screen (left)"), "Screen (left)");
        assert_eq!(name_without_number("Screen ()"), "Screen ()");
        assert_eq!(name_without_number("(2)"), "(2)");
    }

    #[test]
    fn an_overlay_belongs_to_the_monitor_it_overlaps_most() {
        let left = MonitorHandle(1);
        let right = MonitorHandle(2);
        // A taller right monitor, vertically offset.
        let monitors =
            || vec![(left, (0, 400, 1920, 1080)), (right, (1920, 0, 2560, 1440))].into_iter();
        // Bar along the top of the right monitor that also pokes left. Its
        // center (x 1800, y 15) lies on no monitor at all.
        assert_eq!(overlay_owner((1000, 0, 1600, 30), monitors()), Some(right));
        // Mostly on the left monitor.
        assert_eq!(overlay_owner((0, 400, 1960, 30), monitors()), Some(left));
        assert_eq!(overlay_owner((-500, -500, 10, 10), monitors()), None);
    }

    fn monitor(handle: isize, name: &str, is_primary: bool, x: i32, y: i32) -> MonitorInfo {
        MonitorInfo {
            handle: MonitorHandle(handle),
            name: name.to_string(),
            device_path: String::new(),
            stable_id: String::new(),
            is_primary,
            x,
            y,
            width: 1920,
            height: 1080,
            work_x: x,
            work_y: y,
            work_width: 1920,
            work_height: 1040,
            dpi_scale: 1.0,
        }
    }

    #[test]
    fn monitor_index_preference_reorders_the_ring_by_stable_name() {
        let mut manager = Outputs::with_workspace_count(1);
        manager
            .monitors
            .insert(MonitorHandle(20), monitor(20, "Right", false, 1920, 0));
        manager
            .monitors
            .insert(MonitorHandle(10), monitor(10, "Primary", true, 0, 0));
        manager
            .monitors
            .insert(MonitorHandle(30), monitor(30, "Left", false, -1920, 0));
        manager.primary_monitor = Some(MonitorHandle(10));

        // Without preferences the discovery order is unchanged.
        let discovery = manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();
        assert_eq!(
            discovery,
            vec![MonitorHandle(10), MonitorHandle(30), MonitorHandle(20)]
        );

        // Name matching is case-insensitive and moves the named display to
        // its slot; the rest fill the gaps in discovery order.
        manager.set_monitor_index_preferences(vec![("right".to_string(), 0)]);
        let reordered = manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();
        assert_eq!(
            reordered,
            vec![MonitorHandle(20), MonitorHandle(10), MonitorHandle(30)]
        );

        // An out-of-range index clamps to the last slot; unknown names are
        // ignored.
        manager.set_monitor_index_preferences(vec![
            ("Projector".to_string(), 1),
            ("Left".to_string(), 99),
        ]);
        let clamped = manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();
        assert_eq!(
            clamped,
            vec![MonitorHandle(10), MonitorHandle(20), MonitorHandle(30)]
        );
    }

    #[test]
    fn monitor_index_preference_conflicts_keep_discovery_order() {
        let mut manager = Outputs::with_workspace_count(1);
        manager
            .monitors
            .insert(MonitorHandle(20), monitor(20, "Right", false, 1920, 0));
        manager
            .monitors
            .insert(MonitorHandle(10), monitor(10, "Primary", true, 0, 0));
        manager
            .monitors
            .insert(MonitorHandle(30), monitor(30, "Left", false, -1920, 0));
        manager.primary_monitor = Some(MonitorHandle(10));

        // Two displays want slot 1: in discovery order Left comes before
        // Right, so Left keeps slot 1 and Right takes the next free slot.
        manager
            .set_monitor_index_preferences(vec![("Left".to_string(), 1), ("Right".to_string(), 1)]);
        let resolved = manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();
        assert_eq!(
            resolved,
            vec![MonitorHandle(10), MonitorHandle(30), MonitorHandle(20)]
        );

        // Swapping the preference order does not change the outcome: slot
        // conflicts resolve by discovery order, not profile order.
        manager
            .set_monitor_index_preferences(vec![("Right".to_string(), 1), ("Left".to_string(), 1)]);
        let resolved_again = manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();
        assert_eq!(resolved_again, resolved);
    }

    #[test]
    fn monitor_order_is_primary_then_stable_geometry() {
        let mut manager = Outputs::with_workspace_count(1);

        manager
            .monitors
            .insert(MonitorHandle(20), monitor(20, "right", false, 1920, 0));
        manager
            .monitors
            .insert(MonitorHandle(10), monitor(10, "primary", true, 0, 0));
        manager
            .monitors
            .insert(MonitorHandle(30), monitor(30, "left", false, -1920, 0));
        manager.primary_monitor = Some(MonitorHandle(10));

        let ordered_handles = manager
            .get_all_monitors()
            .into_iter()
            .map(|m| m.handle)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered_handles,
            vec![MonitorHandle(10), MonitorHandle(30), MonitorHandle(20)]
        );
        assert_eq!(
            manager.get_first_secondary_monitor_handle(),
            Some(MonitorHandle(30))
        );
    }

    #[test]
    fn configured_work_area_applies_reserved_edges_after_windows_work_area() {
        let mut manager = Outputs::with_workspace_count(1);
        let handle = MonitorHandle(10);
        manager
            .monitors
            .insert(handle, monitor(10, "primary", true, 100, 50));
        assert!(manager.set_reserved_area(
            handle,
            ReservedArea {
                left: 20,
                top: 30,
                right: 200,
                bottom: 10,
            },
        ));

        assert_eq!(
            manager.get_configured_work_area(handle),
            Some(Rect::new(120, 80, 1700, 1000))
        );
    }

    fn shell_room(display: &str, side: crate::shell_room::Side, depth: i32) -> crate::shell_room::ShellRoom {
        crate::shell_room::ShellRoom {
            display: display.to_string(),
            side,
            depth,
        }
    }

    /// Two displays with lasting ids, the second at one and a half.
    fn two_displays() -> Outputs {
        let mut outputs = Outputs::new();
        let mut first = named(1, "Monitor A", "DP-1", 0);
        first.stable_id = "edid:DEL-A1".to_string();
        let mut second = named(2, "Monitor B", "DP-2", 1920);
        second.stable_id = "edid:SAM-B2".to_string();
        second.dpi_scale = 1.5;
        outputs.set_monitors_only(vec![first, second]);
        outputs
    }

    #[test]
    fn room_the_shell_keeps_is_taken_off_the_display_it_names_and_given_back() {
        use crate::shell_room::Side;
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = two_displays();
        let (first, second) = (MonitorHandle(1), MonitorHandle(2));
        let whole = |outputs: &Outputs, monitor| outputs.get_adjusted_work_area(monitor, &desktop).unwrap();
        let (first_whole, second_whole) = (whole(&outputs, first), whole(&outputs, second));

        // A zone at the right of the second display: that display's
        // windows keep off it, and the first is as it was.
        let changed = outputs.set_shell_room(&[shell_room("edid:SAM-B2", Side::Right, 280)]);
        assert_eq!(changed, [second]);
        assert_eq!(whole(&outputs, first), first_whole);
        let kept = whole(&outputs, second);
        assert_eq!((kept.x, kept.y, kept.height), (second_whole.x, second_whole.y, second_whole.height));
        assert_eq!(kept.width, second_whole.width - 280);
        assert_eq!(outputs.shell_room_of(second).right, 280);
        // What was taken is said, for the shell to read back.
        assert_eq!(
            outputs.shell_room_taken(second, &desktop),
            ReservedArea {
                right: 280,
                ..ReservedArea::default()
            }
        );
        assert_eq!(outputs.shell_room_taken(first, &desktop), ReservedArea::default());

        // The same told again changes nothing; another side does.
        assert!(outputs
            .set_shell_room(&[shell_room("edid:SAM-B2", Side::Right, 280)])
            .is_empty());
        let changed = outputs.set_shell_room(&[
            shell_room("edid:SAM-B2", Side::Right, 280),
            shell_room("edid:DEL-A1", Side::Top, 96),
        ]);
        assert_eq!(changed, [first]);
        let kept = whole(&outputs, first);
        assert_eq!((kept.y, kept.height), (first_whole.y + 96, first_whole.height - 96));

        // The shell is gone, or what it said is stale: all given back.
        let changed = outputs.set_shell_room(&[]);
        assert_eq!(changed, [first, second]);
        assert_eq!(whole(&outputs, first), first_whole);
        assert_eq!(whole(&outputs, second), second_whole);
    }

    #[test]
    fn room_on_a_display_that_is_not_there_counts_for_nothing() {
        use crate::shell_room::Side;
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = two_displays();
        let before: Vec<Rect> = [1, 2]
            .map(|handle| outputs.get_adjusted_work_area(MonitorHandle(handle), &desktop).unwrap())
            .to_vec();
        let changed = outputs.set_shell_room(&[
            shell_room("edid:GONE-9", Side::Left, 400),
            shell_room("", Side::Top, 400),
        ]);
        assert!(changed.is_empty());
        let after: Vec<Rect> = [1, 2]
            .map(|handle| outputs.get_adjusted_work_area(MonitorHandle(handle), &desktop).unwrap())
            .to_vec();
        assert_eq!(after, before);
        // A display without a lasting id is never the one that is meant.
        let mut nameless = Outputs::new();
        nameless.set_monitors_only(vec![named(1, "Monitor A", "DP-1", 0)]);
        assert!(nameless.set_shell_room(&[shell_room("", Side::Top, 400)]).is_empty());
        assert_eq!(nameless.shell_room_of(MonitorHandle(1)), ReservedArea::default());
    }

    #[test]
    fn the_shells_room_lies_inside_the_panels_and_is_as_many_pixels_as_the_display_makes_it() {
        use crate::shell_room::Side;
        use gevrek_model::settings::PanelProfile;
        let desktop = crate::testing::FakeDesktop::new(MonitorHandle(1));
        let mut outputs = two_displays();
        outputs.set_bar(Some(PanelProfile::default()));
        outputs.set_panel_seen(true);
        let (first, second) = (MonitorHandle(1), MonitorHandle(2));
        let with_panel = outputs.get_adjusted_work_area(first, &desktop).unwrap();
        outputs.set_shell_room(&[
            shell_room("edid:DEL-A1", Side::Top, 100),
            shell_room("edid:SAM-B2", Side::Left, 100),
        ]);
        // From the side of what the panel leaves, not from the display's.
        let kept = outputs.get_adjusted_work_area(first, &desktop).unwrap();
        assert_eq!(kept.y, with_panel.y + 100);
        assert_eq!(kept.height, with_panel.height - 100);
        // Where displays are measured in their own pixels, a hundred of
        // the configuration's are a hundred and fifty on the second.
        let logical = outputs.get_adjusted_work_area(second, &desktop).unwrap();
        outputs.set_lengths_in_display_pixels(true);
        let pixels = outputs.get_adjusted_work_area(second, &desktop).unwrap();
        assert_eq!(logical.x, 1920 + 100);
        assert_eq!(pixels.x, 1920 + 150);
        // (Said in the work area's units: pixels there.)
        assert_eq!(outputs.shell_room_taken(second, &desktop).left, 150);
        // Never more than leaves the windows a pixel.
        outputs.set_shell_room(&[shell_room("edid:DEL-A1", Side::Left, 100_000)]);
        let least = outputs.get_adjusted_work_area(first, &desktop).unwrap();
        assert!(least.width >= 1 && least.height >= 1);
        // And no more is said to be taken than was there.
        assert_eq!(outputs.shell_room_taken(first, &desktop).left, 1920 - least.width);
    }

    #[test]
    fn global_work_area_offset_applies_after_monitor_reservations() {
        let mut manager = Outputs::with_workspace_count(1);
        let handle = MonitorHandle(10);
        manager
            .monitors
            .insert(handle, monitor(10, "primary", true, 100, 50));
        assert!(manager.set_reserved_area(
            handle,
            ReservedArea {
                left: 20,
                top: 30,
                right: 200,
                bottom: 10,
            },
        ));
        manager.set_global_work_area_offset(ReservedArea {
            left: 5,
            top: 10,
            right: 50,
            bottom: 100,
        });

        // Windows work area (1920x1040 at 100,50) → reservations → global
        // offset: x=100+20+5, y=50+30+10, w=1920-20-200-5-50,
        // h=1040-30-10-10-100.
        assert_eq!(
            manager.get_configured_work_area(handle),
            Some(Rect::new(125, 90, 1645, 890))
        );
        assert_eq!(manager.global_work_area_offset().bottom, 100);
    }

    #[test]
    fn moving_only_workspace_keeps_source_usable_and_focuses_target_copy() {
        let mut manager = Outputs::new();
        let primary = MonitorHandle(10);
        let secondary = MonitorHandle(20);
        manager.add_test_monitor(primary, "Primary");
        manager.add_test_monitor(secondary, "Secondary");
        manager
            .create_workspace(primary, "code".to_string(), crate::layouts::LayoutType::BSP)
            .unwrap();
        manager
            .create_workspace(
                secondary,
                "reference".to_string(),
                crate::layouts::LayoutType::Columns,
            )
            .unwrap();

        let target_index = manager.move_focused_workspace(primary, secondary).unwrap();

        assert_eq!(manager.get_workspaces(primary).unwrap().len(), 1);
        assert_eq!(
            manager
                .get_focused_workspace(secondary)
                .map(|workspace| workspace.name.as_str()),
            Some("code")
        );
        assert_eq!(
            manager.get_workspaces(secondary).unwrap().focused_idx(),
            target_index
        );
    }

    #[test]
    fn swapping_focused_workspaces_preserves_ring_focus_positions() {
        let mut manager = Outputs::new();
        let primary = MonitorHandle(10);
        let secondary = MonitorHandle(20);
        manager.add_test_monitor(primary, "Primary");
        manager.add_test_monitor(secondary, "Secondary");
        manager
            .create_workspace(primary, "code".to_string(), crate::layouts::LayoutType::BSP)
            .unwrap();
        manager
            .create_workspace(
                secondary,
                "reference".to_string(),
                crate::layouts::LayoutType::Columns,
            )
            .unwrap();

        manager.swap_focused_workspaces(primary, secondary).unwrap();

        assert_eq!(
            manager
                .get_focused_workspace(primary)
                .map(|workspace| workspace.name.as_str()),
            Some("reference")
        );
        assert_eq!(
            manager
                .get_focused_workspace(secondary)
                .map(|workspace| workspace.name.as_str()),
            Some("code")
        );
    }
}
