use super::container::{Container, Padding, Rect};
use super::layouts::{
    apply_layout_configured, bsp_resized_ratio, DwindleDirection, EdgeDirection, LayoutAxis,
    LayoutConfig, LayoutType, MasterPosition, NewWindowPlacement, BSP_DEFAULT_SPLIT_RATIO,
};
use super::ring::Ring;
use crate::WindowHandle;
use gevrek_model::graph::BorderStyle;
pub use gevrek_model::layout::WindowContainerBehaviour;
use gevrek_model::layout::WindowSizes;
use gevrek_model::settings::StackbarProfile;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

fn pseudotile_rect(tile: Rect, preferred_width: i32, preferred_height: i32) -> Rect {
    let width = preferred_width.min(tile.width).max(1);
    let height = preferred_height.min(tile.height).max(1);
    Rect::new(
        tile.x + (tile.width - width) / 2,
        tile.y + (tile.height - height) / 2,
        width,
        height,
    )
}

/// Mirror a rect across the workspace mid-axis.
/// `horizontal` mirrors left/right, `vertical` mirrors top/bottom.
fn flip_layout_rect(rect: Rect, work_area: Rect, horizontal: bool, vertical: bool) -> Rect {
    let x = if horizontal {
        work_area.x + (work_area.width - (rect.x - work_area.x) - rect.width)
    } else {
        rect.x
    };
    let y = if vertical {
        work_area.y + (work_area.height - (rect.y - work_area.y) - rect.height)
    } else {
        rect.y
    };
    Rect::new(x, y, rect.width, rect.height)
}

/// State for Expose mode (window overview)
#[derive(Debug, Clone)]
pub struct ExposeState {
    /// Saved window positions before entering expose mode
    pub original_rects: HashMap<WindowHandle, Rect>,
    /// Computed grid cell positions for each window
    pub grid_rects: HashMap<WindowHandle, Rect>,
    /// Currently promoted (centered) window
    pub promoted_window: Option<WindowHandle>,
    /// Current rect of the promoted window
    pub promoted_rect: Option<Rect>,
    /// Remembered promoted sizes per window — when a window is demoted, its actual
    /// size is saved here so it can be restored at the same size when re-promoted.
    pub promoted_rects: HashMap<WindowHandle, Rect>,
}

fn rotate_layout_rect(rect: Rect, work_area: Rect, rotation: u8) -> Rect {
    let width = work_area.width.max(1) as f32;
    let height = work_area.height.max(1) as f32;
    let u = (rect.x - work_area.x) as f32 / width;
    let v = (rect.y - work_area.y) as f32 / height;
    let uw = rect.width as f32 / width;
    let vh = rect.height as f32 / height;
    let (x, y, w, h) = match rotation % 4 {
        1 => (1.0 - v - vh, u, vh, uw),
        2 => (1.0 - u - uw, 1.0 - v - vh, uw, vh),
        3 => (v, 1.0 - u - uw, vh, uw),
        _ => (u, v, uw, vh),
    };
    Rect::new(
        work_area.x + (x * width).round() as i32,
        work_area.y + (y * height).round() as i32,
        (w * width).round().max(1.0) as i32,
        (h * height).round().max(1.0) as i32,
    )
}

impl ExposeState {
    /// Calculate grid positions for windows within the work area
    pub fn calculate_grid(
        windows: &[WindowHandle],
        work_area: Rect,
        padding: i32,
        outer_padding: i32,
    ) -> HashMap<WindowHandle, Rect> {
        let mut grid = HashMap::new();
        let n = windows.len();
        if n == 0 {
            return grid;
        }

        let cols = (n as f64).sqrt().ceil() as usize;
        let rows = (n as f64 / cols as f64).ceil() as usize;

        let usable_width = work_area.width - outer_padding * 2;
        let usable_height = work_area.height - outer_padding * 2;

        let cell_width = (usable_width - padding * (cols as i32 - 1)) / cols as i32;
        let cell_height = (usable_height - padding * (rows as i32 - 1)) / rows as i32;

        for (i, handle) in windows.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;

            let x = work_area.x + outer_padding + col as i32 * (cell_width + padding);
            let y = work_area.y + outer_padding + row as i32 * (cell_height + padding);

            grid.insert(*handle, Rect::new(x, y, cell_width, cell_height));
        }

        grid
    }
}

/// Workspace holds containers and manages their layout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// What the workspace is known by to what names it (keys, rules, a
    /// saved session). It is one of a kind on its display.
    pub name: String,
    /// What is shown of it, where that is something else than its name
    /// (`WorkspaceProfile::title`). Renaming a workspace gives it a title.
    #[serde(default)]
    pub title: Option<String>,
    /// The configuration has an entry for it on this display.
    #[serde(default)]
    pub configured: bool,
    /// It stays when it is left empty. One that does not is taken away
    /// when the display shows another and it has no window.
    #[serde(default)]
    pub persistent: bool,

    /// Containers for tiled windows
    pub containers: Ring<Container>,

    /// Floating windows (not managed by layout)
    pub floating_windows: Ring<WindowHandle>,

    /// Current layout type
    pub layout: LayoutType,

    /// Padding around workspace
    pub padding: Padding,

    /// Container padding
    pub container_padding: Padding,

    /// Whether tiling is enabled (if false, windows are not repositioned by
    /// the layout engine and keep their current positions)
    pub tiling_enabled: bool,

    /// Whether the workspace float override is active. While active, every
    /// window on the workspace (existing and newly added) floats, ignoring
    /// tiling. Disabling re-tiles exactly the windows the override floated.
    #[serde(default)]
    pub float_override: bool,

    /// Per-edge offset applied to the monitor work area before laying out
    /// this workspace. Composition
    /// order: Windows work area → monitor reservations → global offset →
    /// this workspace offset.
    #[serde(default)]
    pub work_area_offset: gevrek_model::settings::ReservedArea,

    /// Transparent border reservation copied from the global appearance
    /// profile: when enabled, tiled windows
    /// are shrunk by `size` on every edge and no border is drawn.
    #[serde(default)]
    pub invisible_borders: gevrek_model::settings::InvisibleBordersProfile,

    /// Windows the float override moved to the floating ring, so disabling
    /// the override re-tiles only those and leaves user-floated windows alone.
    #[serde(skip)]
    pub float_override_windows: HashSet<WindowHandle>,

    /// The windows the Floating layout floats: those that were tiled when
    /// it was chosen and those that came since. They are tiled again when
    /// another layout is chosen; a window floated by hand is not among
    /// them.
    pub floated_by_layout: HashSet<WindowHandle>,

    /// Windows that came to this workspace under the Floating layout and
    /// are still to be put in the middle of its display.
    pub arrivals: Vec<WindowHandle>,

    /// How many pixels of its display one pixel of the configuration is:
    /// the display's scale where displays are measured in their own
    /// pixels (Windows), 1 elsewhere. The display says it
    /// (`Outputs::get_workspaces_mut`); the gaps, the borders and the tab
    /// strip are kept as the configuration gives them and are so many
    /// pixels when they are laid out (`seen`).
    #[serde(skip)]
    pub scale: f32,

    /// ID for tracking
    pub id: usize,

    /// Next container ID
    next_container_id: usize,

    /// Border configuration
    pub border_enabled: bool,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_style: BorderStyle,
    pub active_border_color: (u8, u8, u8),
    pub inactive_border_color: (u8, u8, u8),
    pub active_border_alpha: u8,
    pub inactive_border_alpha: u8,

    /// Where the floating windows were before they were put out of the
    /// way (`ToggleFloatingAside`); empty when they are where they belong.
    #[serde(skip)]
    pub aside: HashMap<WindowHandle, Rect>,

    /// The border and glow of the workspace's floating windows, where the
    /// configuration sets them apart from the tiled ones.
    #[serde(default)]
    pub floating_look: gevrek_model::settings::FloatingAppearance,

    /// Whether each window's border was last drawn with the floating
    /// look, so that a window that starts or stops floating is dressed
    /// anew. A window that is not in here yet (it came from another
    /// workspace) is dressed at the next layout pass.
    #[serde(skip)]
    pub dressed_as_floating: HashMap<WindowHandle, bool>,

    /// Stackbar (tab strip) configuration for containers holding 2+ windows,
    /// copied from the global appearance profile.
    #[serde(default)]
    pub stackbar: StackbarProfile,

    /// Layout configuration for layouts that support it
    /// Master area ratio (0.1 to 0.9) - used by MasterStack, CenteredMaster, etc.
    pub master_ratio: f32,
    /// Number of master windows (1 to 4)
    pub master_count: usize,
    /// Persistent edge occupied by the master area in Master-Stack.
    #[serde(default)]
    pub master_position: MasterPosition,
    /// Placement policy for newly tiled windows in master-family layouts.
    #[serde(default)]
    pub new_window_placement: NewWindowPlacement,
    /// Whether a newly added tiled window gets its own container (Column) or
    /// appends to the focused container (Append).
    #[serde(default)]
    pub window_container_behaviour: WindowContainerBehaviour,
    /// Layout rotation (0 to 3) for rotating orientation
    pub layout_rotation: u8,
    /// Mirror the whole layout left/right.
    #[serde(default)]
    pub layout_flip_horizontal: bool,
    /// Mirror the whole layout top/bottom.
    #[serde(default)]
    pub layout_flip_vertical: bool,
    /// Layout resize lock: while true, tiled resize commands (resize-edge,
    /// resize-axis) and drag-driven ratio updates leave the layout ratios
    /// untouched.
    #[serde(default)]
    pub layout_locked: bool,
    /// Layout remembered by `toggle-monocle` so a second toggle restores it.
    /// Ephemeral runtime state, not serialized.
    #[serde(skip)]
    pub monocle_restore_layout: Option<LayoutType>,
    /// Default direction used when Dwindle creates a new split.
    #[serde(default)]
    pub dwindle_direction: DwindleDirection,
    /// Retain the concrete direction of existing Dwindle split depths.
    #[serde(default)]
    pub dwindle_preserve_split: bool,
    /// Direction history by Dwindle tree depth.
    #[serde(default)]
    pub dwindle_split_history: Vec<DwindleDirection>,
    /// Preferred client size for per-window pseudotiling.
    #[serde(skip)]
    pub pseudotile_sizes: HashMap<WindowHandle, (i32, i32)>,
    /// The smallest size each window's program lets it have, as the
    /// backend last said (only windows that have one): a layout gives
    /// such a window the room its neighbours can spare.
    #[serde(skip)]
    pub smallest_sizes: HashMap<WindowHandle, (i32, i32)>,

    /// Column/row ratios for flexible sizing (used by Columns/Rows layouts)
    /// Each value represents the relative width/height proportion for that container
    /// Empty means equal sizing (default behavior)
    #[serde(default)]
    pub split_ratios: Vec<f32>,

    /// Per-split ratios for the BSP layout:
    /// element `i` is the share of the pre-split region kept by container `i`
    /// when container `i + 1` is created. Empty means even splits (0.5).
    #[serde(default)]
    pub bsp_split_ratios: Vec<f32>,

    /// The sizes the configuration gives the windows: what a scrolling
    /// column is as wide as when it opens, what share Columns and Rows
    /// start from, what BSP and Spiral keep at a split. A window resized
    /// by hand keeps the size it was given.
    #[serde(default)]
    pub sizes: WindowSizes,

    /// The sizes as the configuration last gave them (`sizes` can have
    /// been changed since by hand: a place in view made wider).
    #[serde(default)]
    pub given_sizes: WindowSizes,

    /// The scrolling layout, where the places in view are sized: what the
    /// wheel or the fingers moved the view by that has not yet come to a
    /// column's step.
    #[serde(skip)]
    pub scroll_pending: i32,

    /// Under the scrolling layout: the first column in view. It moves no
    /// more than it must to keep the focused column in view
    /// (`layouts::scrolling`).
    #[serde(default)]
    pub scroll_first: usize,

    /// Under the scrolling layout: where the view is on the strip, in
    /// pixels (how much of the strip is left of it). It follows the
    /// focused column no further than it must, and can be moved by
    /// itself (`scroll_view`).
    #[serde(default)]
    pub scroll_offset: i32,

    /// Only whole columns are shown, and the view moves by whole columns:
    /// where the desktop cannot cut a window off at a display's edge
    /// (what the backend said when the workspace was made).
    #[serde(default = "crate::layouts::scrolling_whole_columns_only")]
    pub scroll_whole_columns: bool,

    /// The view was moved by itself and has not followed the keyboard
    /// since: it stays where it was put until the keyboard goes to
    /// another column.
    #[serde(skip)]
    pub scroll_held: Option<usize>,

    /// Which columns are out of view changed at the last layout pass (or
    /// windows that were out of view are no longer in columns), and the
    /// windows have not been shown and hidden to match yet. The executor
    /// does that for the workspace on screen and clears this.
    #[serde(skip)]
    pub visibility_stale: bool,

    /// Expose mode state (not serialized)
    #[serde(skip)]
    pub expose_state: Option<ExposeState>,
}

/// Remove split `position` of a split chain in which split `i` separates
/// container `i` from the containers after it. The last container shares the
/// last split, so removing it drops that one.
fn remove_split_at<T>(splits: &mut Vec<T>, position: usize) {
    if splits.is_empty() {
        return;
    }
    splits.remove(position.min(splits.len() - 1));
}

/// The border a window is drawn with (see [`Workspace::border_look`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderLook {
    pub enabled: bool,
    pub width: i32,
    pub offset: i32,
    pub style: BorderStyle,
    pub active_color: (u8, u8, u8),
    pub inactive_color: (u8, u8, u8),
    pub active_alpha: u8,
    pub inactive_alpha: u8,
    /// It is the floating look (the glow goes with it).
    pub floating: bool,
}

/// The number a name stands for: `3`, `Workspace 3`, and `Workspace` (1).
pub fn number_of(name: &str) -> Option<usize> {
    let name = name.trim();
    let digits = match name.get(..9) {
        Some(word) if word.eq_ignore_ascii_case("workspace") => name[9..].trim(),
        _ => name,
    };
    if digits.is_empty() {
        return (digits.len() != name.len()).then_some(1);
    }
    (digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
        .filter(|number| *number > 0)
}

impl Workspace {
    /// Whether the workspace is the one meant by a name: its own, or what
    /// it is shown by. (Among a display's workspaces the one of that name
    /// comes before one that is only shown by it: `position_called`.)
    pub fn is_called(&self, name: &str) -> bool {
        let name = name.trim();
        self.name.trim().eq_ignore_ascii_case(name) || self.label().eq_ignore_ascii_case(name)
    }

    /// What is shown of it: its title, or its name where it has none.
    pub fn label(&self) -> &str {
        match self.title.as_deref().map(str::trim) {
            Some(title) if !title.is_empty() => title,
            _ => self.name.trim(),
        }
    }

    /// The number a workspace shows as, where it shows as one: `3`, and
    /// the older `Workspace 3` and `Workspace` (the first).
    pub fn number(&self) -> Option<usize> {
        number_of(self.label()).or_else(|| number_of(&self.name))
    }

    /// Where among workspaces the one meant by `name` is: the one of that
    /// name, or else the one shown by it.
    pub fn position_called(workspaces: &[Workspace], name: &str) -> Option<usize> {
        let name = name.trim();
        workspaces
            .iter()
            .position(|workspace| workspace.name.trim().eq_ignore_ascii_case(name))
            .or_else(|| {
                workspaces
                    .iter()
                    .position(|workspace| workspace.label().eq_ignore_ascii_case(name))
            })
    }

    pub fn new(id: usize, name: String) -> Self {
        Self {
            name,
            title: None,
            configured: false,
            // (One made for the moment says so itself: see
            // `Executor::create_workspace_for_selector`.)
            persistent: true,
            containers: Ring::new(),
            floating_windows: Ring::new(),
            layout: LayoutType::BSP,
            padding: Padding::uniform(10),
            container_padding: Padding::uniform(5),
            tiling_enabled: true,
            float_override: false,
            float_override_windows: HashSet::new(),
            floated_by_layout: HashSet::new(),
            aside: HashMap::new(),
            floating_look: Default::default(),
            dressed_as_floating: HashMap::new(),
            arrivals: Vec::new(),
            work_area_offset: gevrek_model::settings::ReservedArea::default(),
            invisible_borders: gevrek_model::settings::InvisibleBordersProfile::default(),
            scale: 1.0,
            id,
            next_container_id: 0,
            // Default border configuration (disabled by default to avoid breaking changes)
            border_enabled: false,
            border_width: 4,
            border_offset: -1,
            border_style: BorderStyle::Rounded,
            active_border_color: (255, 0, 0),
            inactive_border_color: (0, 0, 255),
            active_border_alpha: 255,
            inactive_border_alpha: 200,
            stackbar: StackbarProfile::default(),
            // Layout configuration defaults
            master_ratio: 0.5,
            master_count: 1,
            master_position: MasterPosition::Left,
            new_window_placement: NewWindowPlacement::StackEnd,
            window_container_behaviour: WindowContainerBehaviour::Column,
            layout_rotation: 0,
            layout_flip_horizontal: false,
            layout_flip_vertical: false,
            layout_locked: false,
            monocle_restore_layout: None,
            dwindle_direction: DwindleDirection::Alternate,
            dwindle_preserve_split: false,
            dwindle_split_history: Vec::new(),
            pseudotile_sizes: HashMap::new(),
            smallest_sizes: HashMap::new(),
            split_ratios: Vec::new(),
            bsp_split_ratios: Vec::new(),
            sizes: WindowSizes::default(),
            given_sizes: WindowSizes::default(),
            scroll_pending: 0,
            scroll_first: 0,
            scroll_offset: 0,
            scroll_whole_columns: crate::layouts::scrolling_whole_columns_only(),
            scroll_held: None,
            visibility_stale: false,
            expose_state: None,
        }
    }

    /// A length of the configuration in the pixels of the display this
    /// workspace is on (see `scale`).
    pub fn seen(&self, length: i32) -> i32 {
        // (A workspace read from a saved session has no scale yet.)
        if self.scale <= 0.0 || self.scale == 1.0 {
            return length;
        }
        (length as f32 * self.scale.clamp(0.5, 8.0)).round() as i32
    }

    fn seen_padding(&self, padding: Padding) -> Padding {
        Padding::new(
            self.seen(padding.left),
            self.seen(padding.top),
            self.seen(padding.right),
            self.seen(padding.bottom),
        )
    }

    /// The room left free round the windows, in the display's pixels.
    pub fn outer_gaps(&self) -> Padding {
        self.seen_padding(self.padding)
    }

    /// The room between the windows, in the display's pixels.
    pub fn inner_gaps(&self) -> Padding {
        self.seen_padding(self.container_padding)
    }

    /// The tab strip as it is drawn on this display: as high as the
    /// display's scale makes it.
    pub fn stackbar_seen(&self) -> StackbarProfile {
        let mut stackbar = self.stackbar.clone();
        stackbar.height = self.seen(stackbar.height);
        stackbar
    }

    /// Whether a window is in a column that is out of view (scrolling
    /// layout).
    pub fn out_of_view(&self, handle: WindowHandle) -> bool {
        self.containers
            .iter()
            .any(|container| container.offscreen && container.contains_window(handle))
    }

    /// The room the columns of the scrolling layout share: the work area
    /// within the workspace's offset and padding.
    fn strip_room(&self, work_area: Rect) -> i32 {
        self.work_area_offset
            .apply_to(work_area)
            .with_padding(self.outer_gaps())
            .width
            .max(1)
    }

    /// Give a column the width its window was resized to, in pixels (the
    /// gap round the window is the column's too). False when it is no
    /// column or already has that width.
    pub fn set_column_width(
        &mut self,
        container_idx: usize,
        window_width: i32,
        work_area: Rect,
    ) -> bool {
        let room = self.strip_room(work_area);
        let gaps = self.inner_gaps().left / 2 + self.inner_gaps().right / 2;
        let (least, most) = crate::layouts::SCROLLING_WIDTH_RANGE;
        let share = ((window_width + gaps) as f32 / room as f32).clamp(least, most);
        // Where the places in view are sized, a column that stands at one
        // gives the place the width.
        if let Some(place) = self.place_of(container_idx) {
            let Some(sized) = self.sizes.by_place.get_mut(place) else {
                return false;
            };
            if (*sized - share).abs() < f32::EPSILON {
                return false;
            }
            *sized = share;
            return true;
        }
        let Some(container) = self.containers.get_mut(container_idx) else {
            return false;
        };
        if (container.strip_width - share).abs() < f32::EPSILON {
            return false;
        }
        container.strip_width = share;
        true
    }

    /// The place in view a column stands at, where the places are sized
    /// and it stands at one.
    fn place_of(&self, container_idx: usize) -> Option<usize> {
        if !self.sizes_places() {
            return None;
        }
        let position = self
            .active_container_indices()
            .iter()
            .position(|index| *index == container_idx)?;
        position
            .checked_sub(self.scroll_first)
            .filter(|place| *place < self.sizes.by_place.len())
    }

    /// Make a column wider (or, with a negative `delta_px`, narrower).
    fn resize_column(&mut self, container_idx: usize, delta_px: i32, work_area: Rect) -> bool {
        let room = self.strip_room(work_area);
        let Some(container) = self.containers.get(container_idx) else {
            return false;
        };
        let share = match self.place_of(container_idx) {
            Some(place) => self.sizes.column_width_at(place),
            None => container.strip_width,
        };
        let now = crate::layouts::scrolling_column_width(share, room);
        let gaps = self.inner_gaps().left / 2 + self.inner_gaps().right / 2;
        self.set_column_width(container_idx, now.saturating_add(delta_px).saturating_sub(gaps), work_area)
    }

    /// Update split ratios based on a window being resized
    /// This recalculates the ratios for all containers based on the new position of the resized window.
    /// `new_width` and `work_area_width` are along the way the shares lie;
    /// `along_height`: that way is the height (the gaps at the top and the
    /// bottom are the ones taken off).
    pub fn update_split_ratios_from_resize(
        &mut self,
        container_idx: usize,
        new_width: i32,
        work_area_width: i32,
        along_height: bool,
    ) {
        let count = self.containers.len();
        if count < 2 || container_idx >= count {
            return;
        }

        // Calculate available width after workspace padding
        let gaps = self.outer_gaps();
        let available_width = work_area_width
            - if along_height {
                gaps.top + gaps.bottom
            } else {
                gaps.left + gaps.right
            };
        if available_width <= 0 {
            return;
        }

        // Calculate the new ratio for the resized container
        let new_ratio = (new_width as f32) / (available_width as f32);
        let new_ratio = new_ratio.clamp(0.1, 0.9); // Minimum 10% width

        self.rebalance_split_ratios(container_idx, new_ratio);

        log::trace!("Updated split ratios: {:?}", self.split_ratios);
    }

    /// Clear split ratios (reset to equal sizing)
    pub fn reset_split_ratios(&mut self) {
        self.split_ratios.clear();
    }

    /// Columns and Rows: the shares of `count` windows that have not been
    /// resized by hand: those the configuration gives them by their
    /// place, else as much each as the others.
    fn starting_shares(&self, count: usize) -> Vec<f32> {
        let shares = self.sizes.shares(count);
        if shares.len() == count {
            shares
        } else {
            vec![1.0 / count.max(1) as f32; count]
        }
    }

    /// Take the sizes the configuration gives the windows. When they are
    /// other than those the workspace had, the windows are given them
    /// anew, those resized by hand too: the configuration was changed to
    /// be seen. False when they are the same.
    pub fn set_sizes(&mut self, sizes: &WindowSizes) -> bool {
        // (The same as the configuration gave before: what was changed by
        // hand since is kept.)
        if self.given_sizes == *sizes {
            return false;
        }
        self.given_sizes = sizes.clone();
        self.sizes = sizes.clone();
        self.split_ratios.clear();
        self.bsp_split_ratios.clear();
        let width = self.sizes.new_column_width();
        for container in self.containers.elements_mut().iter_mut() {
            container.strip_width = width;
        }
        true
    }

    /// The scrolling layout sizes the places in view (the configuration
    /// gives places sizes of their own): a column is as wide as the place
    /// it stands at, counted from the first in view.
    pub fn sizes_places(&self) -> bool {
        self.layout == LayoutType::Scrolling && !self.sizes.by_place.is_empty()
    }

    /// The places' widths in pixels, in a view so wide.
    fn place_widths(&self, room: i32) -> Vec<i32> {
        (0..self.sizes.by_place.len())
            .map(|place| {
                crate::layouts::scrolling_column_width(self.sizes.column_width_at(place), room)
            })
            .collect()
    }

    /// The strip's columns that are laid out, their own widths, and the
    /// places' widths, in a view so wide.
    fn strip_widths(&self, room: i32) -> (Vec<usize>, Vec<i32>, Vec<i32>) {
        let active = self.active_container_indices();
        let own = active
            .iter()
            .filter_map(|index| self.containers.get(*index))
            .map(|container| crate::layouts::scrolling_column_width(container.strip_width, room))
            .collect();
        let places = if self.sizes_places() {
            self.place_widths(room)
        } else {
            Vec::new()
        };
        (active, own, places)
    }

    #[cfg(test)]
    pub(crate) fn bsp_ratios(&self) -> &[f32] {
        &self.bsp_split_ratios
    }

    /// Keep the BSP split-ratio chain in step with the container count,
    /// preserving existing ratios across membership changes and filling new
    /// splits with the default 0.5.
    fn sync_bsp_split_ratios(&mut self, container_count: usize) {
        let needed = container_count.saturating_sub(1);
        self.bsp_split_ratios.truncate(needed);
        while self.bsp_split_ratios.len() < needed {
            self.bsp_split_ratios
                .push(self.sizes.split_share().unwrap_or(BSP_DEFAULT_SPLIT_RATIO));
        }
    }

    /// Indices of the non-empty containers that participate in the layout,
    /// in ring order. Mirrors the filtering in `apply_layout`.
    fn active_container_indices(&self) -> Vec<usize> {
        self.containers
            .elements()
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_empty())
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Rebalance `split_ratios` so `container_idx` holds `new_ratio`,
    /// spreading the difference across the other containers and
    /// renormalizing to a sum of 1.
    fn rebalance_split_ratios(&mut self, container_idx: usize, new_ratio: f32) {
        let count = self.containers.len();
        if count < 2 || container_idx >= count {
            return;
        }
        if self.split_ratios.len() != count {
            self.split_ratios = self.starting_shares(count);
        }
        let old_ratio = self.split_ratios[container_idx];
        let ratio_diff = new_ratio - old_ratio;
        let other_count = count - 1;
        if other_count > 0 {
            let diff_per_other = ratio_diff / other_count as f32;
            for (i, ratio) in self.split_ratios.iter_mut().enumerate() {
                if i == container_idx {
                    *ratio = new_ratio;
                } else {
                    *ratio -= diff_per_other;
                    *ratio = ratio.clamp(0.05, 0.95); // Ensure minimum 5%
                }
            }
        }
        // Normalize ratios to sum to 1.0
        let sum: f32 = self.split_ratios.iter().sum();
        if sum > 0.0 {
            for ratio in self.split_ratios.iter_mut() {
                *ratio /= sum;
            }
        }
    }

    /// Resize one edge of a tiled container by `delta_px` pixels; a positive
    /// delta grows the container. Returns `false` when the layout has no
    /// adjustable split along that edge (or the edge
    /// sits on the workspace boundary), so the caller can report a no-op.
    pub fn resize_container_edge(
        &mut self,
        container_idx: usize,
        direction: EdgeDirection,
        delta_px: i32,
        work_area: Rect,
    ) -> bool {
        match self.layout {
            LayoutType::BSP => {
                let padded = work_area.with_padding(self.outer_gaps());
                let active = self.active_container_indices();
                let Some(position) = active.iter().position(|idx| *idx == container_idx) else {
                    return false;
                };
                self.sync_bsp_split_ratios(active.len());
                let Some((split, new_ratio)) = bsp_resized_ratio(
                    padded,
                    active.len(),
                    position,
                    &self.bsp_split_ratios,
                    direction,
                    delta_px,
                ) else {
                    return false;
                };
                self.bsp_split_ratios[split - 1] = new_ratio;
                true
            }
            LayoutType::MasterStack | LayoutType::UltrawideVerticalStack => {
                let axis = match direction {
                    EdgeDirection::Left | EdgeDirection::Right => LayoutAxis::Horizontal,
                    EdgeDirection::Up | EdgeDirection::Down => LayoutAxis::Vertical,
                };
                if !self.master_split_matches(axis)
                    || !self.edge_faces_master(container_idx, direction)
                {
                    return false;
                }
                self.adjust_master_ratio_for(container_idx, delta_px, axis, work_area)
            }
            LayoutType::CenteredMaster => match direction {
                EdgeDirection::Left | EdgeDirection::Right => self.adjust_master_ratio_for(
                    container_idx,
                    delta_px,
                    LayoutAxis::Horizontal,
                    work_area,
                ),
                EdgeDirection::Up | EdgeDirection::Down => false,
            },
            LayoutType::Columns => match direction {
                EdgeDirection::Left | EdgeDirection::Right => self.adjust_column_ratio(
                    container_idx,
                    delta_px,
                    work_area,
                    LayoutAxis::Horizontal,
                ),
                EdgeDirection::Up | EdgeDirection::Down => false,
            },
            LayoutType::Rows => match direction {
                EdgeDirection::Up | EdgeDirection::Down => self.adjust_column_ratio(
                    container_idx,
                    delta_px,
                    work_area,
                    LayoutAxis::Vertical,
                ),
                EdgeDirection::Left | EdgeDirection::Right => false,
            },
            // A column's width is its own: no other column gives or takes.
            LayoutType::Scrolling => match direction {
                EdgeDirection::Left | EdgeDirection::Right => {
                    self.resize_column(container_idx, delta_px, work_area)
                }
                EdgeDirection::Up | EdgeDirection::Down => false,
            },
            _ => false,
        }
    }

    /// Resize a tiled container along an axis by `delta_px` pixels; a positive
    /// delta grows the container. Returns `false`
    /// when nothing along the axis is adjustable.
    pub fn resize_container_axis(
        &mut self,
        container_idx: usize,
        axis: LayoutAxis,
        delta_px: i32,
        work_area: Rect,
    ) -> bool {
        match self.layout {
            LayoutType::BSP => {
                let padded = work_area.with_padding(self.outer_gaps());
                let active = self.active_container_indices();
                let Some(position) = active.iter().position(|idx| *idx == container_idx) else {
                    return false;
                };
                self.sync_bsp_split_ratios(active.len());
                let mut resized = false;
                // Grow along the forward split when possible, else pull the
                // trailing boundary; either grows the container.
                for directions in [
                    [EdgeDirection::Right, EdgeDirection::Left],
                    [EdgeDirection::Down, EdgeDirection::Up],
                ] {
                    let matches_axis = match directions[0] {
                        EdgeDirection::Right => axis.includes_horizontal(),
                        _ => axis.includes_vertical(),
                    };
                    if !matches_axis {
                        continue;
                    }
                    for direction in directions {
                        if let Some((split, new_ratio)) = bsp_resized_ratio(
                            padded,
                            active.len(),
                            position,
                            &self.bsp_split_ratios,
                            direction,
                            delta_px,
                        ) {
                            self.bsp_split_ratios[split - 1] = new_ratio;
                            resized = true;
                            break;
                        }
                    }
                }
                resized
            }
            LayoutType::MasterStack
            | LayoutType::CenteredMaster
            | LayoutType::UltrawideVerticalStack => {
                let mut resized = false;
                if axis.includes_horizontal() && self.master_split_matches(LayoutAxis::Horizontal) {
                    resized |= self.adjust_master_ratio_for(
                        container_idx,
                        delta_px,
                        LayoutAxis::Horizontal,
                        work_area,
                    );
                }
                if axis.includes_vertical() && self.master_split_matches(LayoutAxis::Vertical) {
                    resized |= self.adjust_master_ratio_for(
                        container_idx,
                        delta_px,
                        LayoutAxis::Vertical,
                        work_area,
                    );
                }
                resized
            }
            // Dwindle shares every room by one ratio (the master ratio,
            // kept between 0.3 and 0.7). A window keeps the first part of
            // its own split and lies in the rest of every split before
            // it: growing it raises the ratio where its own split lies
            // along the axis, and lowers it where an earlier one does.
            // (It answered that nothing could be resized: the keys for
            // wider and narrower did nothing in the layout gevrek starts
            // with.)
            LayoutType::Dwindle => {
                let active = self.active_container_indices();
                let Some(position) = active.iter().position(|idx| *idx == container_idx) else {
                    return false;
                };
                let count = active.len();
                if count < 2 {
                    return false;
                }
                self.sync_dwindle_split_history(count);
                let history = &self.dwindle_split_history;
                let along = |depth: usize| match history.get(depth) {
                    Some(DwindleDirection::Vertical) => axis.includes_vertical(),
                    _ => axis.includes_horizontal(),
                };
                let own = position + 1 < count && along(position);
                let depth = if own {
                    Some(position)
                } else {
                    (0..position).rev().find(|depth| along(*depth))
                };
                let Some(depth) = depth else {
                    return false;
                };
                let padded = work_area.with_padding(self.outer_gaps());
                let extent = match history.get(depth) {
                    Some(DwindleDirection::Vertical) => padded.height,
                    _ => padded.width,
                };
                if extent <= 0 {
                    return false;
                }
                let sign = if own { 1.0 } else { -1.0 };
                self.master_ratio = (self.master_ratio.clamp(0.3, 0.7)
                    + sign * delta_px as f32 / extent as f32)
                    .clamp(0.3, 0.7);
                true
            }
            LayoutType::Columns if axis.includes_horizontal() => {
                self.adjust_column_ratio(container_idx, delta_px, work_area, LayoutAxis::Horizontal)
            }
            LayoutType::Rows if axis.includes_vertical() => {
                self.adjust_column_ratio(container_idx, delta_px, work_area, LayoutAxis::Vertical)
            }
            LayoutType::Scrolling if axis.includes_horizontal() => {
                self.resize_column(container_idx, delta_px, work_area)
            }
            _ => false,
        }
    }

    /// Whether the master area is separated from the stack along `axis`.
    fn master_split_matches(&self, axis: LayoutAxis) -> bool {
        let horizontal_split = match self.layout {
            LayoutType::CenteredMaster | LayoutType::UltrawideVerticalStack => true,
            LayoutType::MasterStack => matches!(
                self.master_position,
                MasterPosition::Left | MasterPosition::Right
            ),
            _ => return false,
        };
        match axis {
            LayoutAxis::Horizontal => horizontal_split,
            LayoutAxis::Vertical => !horizontal_split,
            LayoutAxis::HorizontalAndVertical => true,
        }
    }

    /// Whether the container is on the master side of the master split.
    fn is_master_side(&self, container_idx: usize) -> bool {
        container_idx < self.master_count.max(1)
    }

    /// Whether `direction` points from the container toward the master split
    /// (the only adjustable boundary in master-family layouts).
    fn edge_faces_master(&self, container_idx: usize, direction: EdgeDirection) -> bool {
        let master_side = self.is_master_side(container_idx);
        match self.master_position {
            MasterPosition::Left => matches!(
                (master_side, direction),
                (true, EdgeDirection::Right) | (false, EdgeDirection::Left)
            ),
            MasterPosition::Right => matches!(
                (master_side, direction),
                (true, EdgeDirection::Left) | (false, EdgeDirection::Right)
            ),
            MasterPosition::Top => matches!(
                (master_side, direction),
                (true, EdgeDirection::Down) | (false, EdgeDirection::Up)
            ),
            MasterPosition::Bottom => matches!(
                (master_side, direction),
                (true, EdgeDirection::Up) | (false, EdgeDirection::Down)
            ),
        }
    }

    /// Adjust the master ratio by `delta_px` of the padded work area, growing
    /// the side the container sits on. Returns `false` when the container is
    /// not part of the tiled ring.
    fn adjust_master_ratio_for(
        &mut self,
        container_idx: usize,
        delta_px: i32,
        axis: LayoutAxis,
        work_area: Rect,
    ) -> bool {
        if container_idx >= self.containers.len() {
            return false;
        }
        let padded = work_area.with_padding(self.outer_gaps());
        let extent = match axis {
            LayoutAxis::Vertical => padded.height,
            _ => padded.width,
        };
        if extent <= 0 {
            return false;
        }
        let sign = if self.is_master_side(container_idx) {
            1.0
        } else {
            -1.0
        };
        let delta_ratio = sign * delta_px as f32 / extent as f32;
        self.master_ratio = (self.master_ratio + delta_ratio).clamp(0.1, 0.9);
        true
    }

    /// Adjust a Columns/Rows split ratio by `delta_px` of the padded work
    /// area along `axis`, rebalancing the other containers.
    fn adjust_column_ratio(
        &mut self,
        container_idx: usize,
        delta_px: i32,
        work_area: Rect,
        axis: LayoutAxis,
    ) -> bool {
        let count = self.containers.len();
        if count < 2 || container_idx >= count {
            return false;
        }
        let padded = work_area.with_padding(self.outer_gaps());
        let extent = match axis {
            LayoutAxis::Vertical => padded.height,
            _ => padded.width,
        };
        if extent <= 0 {
            return false;
        }
        if self.split_ratios.len() != count {
            self.split_ratios = self.starting_shares(count);
        }
        let new_ratio =
            (self.split_ratios[container_idx] + delta_px as f32 / extent as f32).clamp(0.1, 0.9);
        self.rebalance_split_ratios(container_idx, new_ratio);
        true
    }

    /// Toggle the layout mirror along an axis.
    pub fn toggle_layout_flip(&mut self, axis: LayoutAxis) {
        if axis.includes_horizontal() {
            self.layout_flip_horizontal = !self.layout_flip_horizontal;
        }
        if axis.includes_vertical() {
            self.layout_flip_vertical = !self.layout_flip_vertical;
        }
    }

    /// Switch the workspace to the monocle layout,
    /// remembering the current layout; a second toggle restores it. Returns
    /// `true` when monocle is now active, `false` when the previous layout
    /// was restored, and an error message when the workspace already uses the
    /// monocle layout persistently (nothing to toggle back to).
    pub fn toggle_monocle(&mut self) -> Result<bool, String> {
        match self.monocle_restore_layout.take() {
            Some(previous) if self.layout == LayoutType::Monocle => {
                self.layout = previous;
                Ok(false)
            }
            Some(_) => {
                // The layout changed while monocle was active; drop the stale
                // restore point and start a fresh monocle toggle.
                self.monocle_restore_layout = Some(self.layout);
                self.layout = LayoutType::Monocle;
                Ok(true)
            }
            None if self.layout == LayoutType::Monocle => {
                Err("The workspace already uses the monocle layout".to_string())
            }
            None => {
                self.monocle_restore_layout = Some(self.layout);
                self.layout = LayoutType::Monocle;
                Ok(true)
            }
        }
    }

    /// Toggle the workspace's window container behaviour between Column and
    /// Append. Returns the new
    /// behaviour.
    pub fn toggle_window_container_behaviour(&mut self) -> WindowContainerBehaviour {
        self.window_container_behaviour = self.window_container_behaviour.toggled();
        self.window_container_behaviour
    }

    /// Add a tiled window to the workspace
    /// Creates a new container or adds to focused container based on mode
    pub fn add_window(&mut self, handle: WindowHandle, create_container: bool) {
        // Under the Floating layout nothing is tiled: the window floats,
        // and is put in the middle of the display at the next layout pass.
        if self.layout == LayoutType::Floating {
            self.add_floating_window(handle);
            self.floated_by_layout.insert(handle);
            if !self.arrivals.contains(&handle) {
                self.arrivals.push(handle);
            }
            return;
        }
        // While the float override is active, every new window floats.
        if self.float_override {
            self.add_floating_window(handle);
            self.float_override_windows.insert(handle);
            return;
        }
        // In Append mode a newly
        // added window joins the focused container instead of getting its
        // own. The first window on an empty workspace always creates one.
        let append_to_focused = create_container
            && matches!(
                self.window_container_behaviour,
                WindowContainerBehaviour::Append
            )
            && !self.containers.is_empty();
        if (create_container && !append_to_focused) || self.containers.is_empty() {
            // Create new container for this window
            let mut container = Container::new(self.next_container_id);
            self.next_container_id += 1;
            container.add_window(handle);
            let master_family = matches!(
                self.layout,
                LayoutType::MasterStack
                    | LayoutType::CenteredMaster
                    | LayoutType::UltrawideVerticalStack
            );
            if self.layout == LayoutType::Scrolling {
                // Its column is beside the one with the keyboard, as wide
                // as the configuration says a new column is.
                let index = if self.containers.is_empty() {
                    0
                } else {
                    self.containers.focused_idx().saturating_add(1)
                };
                container.strip_width = self.sizes.new_column_width();
                self.containers.insert_at(index, container);
            } else if master_family {
                match self.new_window_placement {
                    NewWindowPlacement::StackEnd => self.containers.push(container),
                    NewWindowPlacement::Master => self.containers.push_front(container),
                    NewWindowPlacement::AfterFocused => {
                        let index = self.containers.focused_idx().saturating_add(1);
                        self.containers.insert_at(index, container);
                    }
                }
            } else {
                container.strip_width = self.sizes.new_column_width();
                self.containers.push(container);
            }
            // Reset split_ratios since container count changed - stale ratios
            // from the old count would cause incorrect column/row widths
            self.split_ratios.clear();
        } else {
            // Add to focused container
            if let Some(container) = self.containers.focused_mut() {
                container.add_window(handle);
            }
        }
    }

    /// Add a tiled window at a specific container index (preserves order)
    /// This is used when re-adding a window that was temporarily removed
    pub fn add_window_at(&mut self, handle: WindowHandle, container_idx: usize) {
        // Create new container for this window
        let mut container = Container::new(self.next_container_id);
        self.next_container_id += 1;
        container.add_window(handle);
        container.strip_width = self.sizes.new_column_width();

        // Insert at the specified index (preserves order)
        self.containers.insert_at(container_idx, container);
    }

    /// A container was taken out of the ring at `position` by other
    /// hands than [`Workspace::remove_window`] (two joined as a group):
    /// its split goes with it, so the splits after it keep their
    /// shares. (They were cut at the end instead, and every share
    /// after the one that went moved along by one.)
    pub(crate) fn container_left(&mut self, position: usize) {
        remove_split_at(&mut self.bsp_split_ratios, position);
        remove_split_at(&mut self.dwindle_split_history, position);
        let needed = self.containers.len().saturating_sub(1);
        self.bsp_split_ratios.truncate(needed);
        self.dwindle_split_history.truncate(needed);
    }

    /// A container was put into the ring at `position` (a window left
    /// its group): a split of its own comes in there, an even one, and
    /// the splits after it keep their shares.
    pub(crate) fn container_came(&mut self, position: usize) {
        let needed = self.containers.len().saturating_sub(1);
        if self.bsp_split_ratios.len() < needed {
            let at = position.min(self.bsp_split_ratios.len());
            let share = self.sizes.split_share().unwrap_or(BSP_DEFAULT_SPLIT_RATIO);
            self.bsp_split_ratios.insert(at, share);
        }
    }

    /// Add a floating window to the workspace
    pub fn add_floating_window(&mut self, handle: WindowHandle) {
        self.floating_windows.push(handle);
    }

    /// Remove a window from the workspace (checks both tiled and floating)
    pub fn remove_window(&mut self, handle: WindowHandle) -> bool {
        let mut removed = false;
        let mut empty_container_idx = None;

        // Try removing from tiled containers
        for (idx, container) in self.containers.elements_mut().iter_mut().enumerate() {
            if container.remove_window(handle) {
                removed = true;
                if container.is_empty() {
                    empty_container_idx = Some(idx);
                }
                break;
            }
        }

        // Remove empty container
        if let Some(idx) = empty_container_idx {
            // Position among the laid-out containers (the BSP chain is
            // indexed by active containers; this one is empty now).
            let active_position = self
                .active_container_indices()
                .iter()
                .filter(|active| **active < idx)
                .count();
            self.containers.remove_at(idx);
            // Reset split_ratios since container count changed
            self.split_ratios.clear();
            // Drop the split that held the removed container, so the other
            // splits keep their ratios (truncating always dropped the last
            // split, shifting every ratio after the removed container).
            remove_split_at(&mut self.bsp_split_ratios, active_position);
            remove_split_at(&mut self.dwindle_split_history, active_position);
            self.bsp_split_ratios
                .truncate(self.containers.len().saturating_sub(1));
            self.dwindle_split_history
                .truncate(self.containers.len().saturating_sub(1));
        }

        // If not found in containers, try removing from floating windows
        if !removed {
            let mut floating_idx = None;
            for (idx, win) in self.floating_windows.iter().enumerate() {
                if *win == handle {
                    floating_idx = Some(idx);
                    break;
                }
            }
            if let Some(idx) = floating_idx {
                self.floating_windows.remove_at(idx);
                removed = true;
            }
        }

        if removed {
            self.pseudotile_sizes.remove(&handle);
            self.float_override_windows.remove(&handle);
            self.floated_by_layout.remove(&handle);
            self.dressed_as_floating.remove(&handle);
            self.aside.remove(&handle);
            self.arrivals.retain(|arrived| *arrived != handle);
        }

        removed
    }

    /// The border a window of this workspace is drawn with: the
    /// workspace's own, or for a floating window the floating look where
    /// the configuration sets one apart (its colours and strengths fall
    /// back to the workspace's when they are not given).
    pub fn border_look(&self, floating: bool) -> BorderLook {
        let tiled = BorderLook {
            enabled: self.border_enabled,
            width: self.seen(self.border_width),
            offset: self.seen(self.border_offset),
            style: self.border_style,
            active_color: self.active_border_color,
            inactive_color: self.inactive_border_color,
            active_alpha: self.active_border_alpha,
            inactive_alpha: self.inactive_border_alpha,
            floating: false,
        };
        let look = &self.floating_look;
        if !floating || !look.separate {
            return tiled;
        }
        BorderLook {
            // A workspace without borders has none on its floating
            // windows either.
            enabled: tiled.enabled && look.border_enabled,
            width: self.seen(look.border_width),
            offset: self.seen(look.border_offset),
            style: tiled.style,
            active_color: look.active_border_color.unwrap_or(tiled.active_color),
            inactive_color: look.inactive_border_color.unwrap_or(tiled.inactive_color),
            active_alpha: look.active_border_alpha.unwrap_or(tiled.active_alpha),
            inactive_alpha: look.inactive_border_alpha.unwrap_or(tiled.inactive_alpha),
            floating: true,
        }
    }

    /// Whether a window of this workspace floats.
    pub fn floats(&self, handle: WindowHandle) -> bool {
        self.floating_windows.iter().any(|window| *window == handle)
    }

    /// Bring the windows in line with the layout where the Floating
    /// layout is concerned, whichever way the layout was set (a command,
    /// the configuration, the graph). Under it every tiled window is
    /// floated, where it is; under any other the windows it floated are
    /// tiled again. Returns `(floated, retiled)` as `set_float_override`
    /// does; both empty when nothing was to do.
    pub fn sync_floating_layout(&mut self) -> (Vec<WindowHandle>, Vec<WindowHandle>) {
        if self.layout == LayoutType::Floating {
            let floated: Vec<WindowHandle> = self
                .containers
                .iter()
                .flat_map(|container| container.windows.iter().copied())
                .collect();
            if floated.is_empty() {
                return (Vec::new(), Vec::new());
            }
            if self.containers.iter().any(|container| container.offscreen) {
                self.visibility_stale = true;
            }
            self.containers.clear();
            self.split_ratios.clear();
            self.bsp_split_ratios.clear();
            for handle in &floated {
                self.floating_windows.push(*handle);
                self.floated_by_layout.insert(*handle);
            }
            return (floated, Vec::new());
        }
        self.arrivals.clear();
        if self.floated_by_layout.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let mut retiled = Vec::new();
        let mut index = 0;
        while index < self.floating_windows.len() {
            let handle = self.floating_windows.elements()[index];
            if self.floated_by_layout.remove(&handle) {
                self.floating_windows.remove_at(index);
                self.add_window(handle, true);
                retiled.push(handle);
            } else {
                index += 1;
            }
        }
        self.floated_by_layout.clear();
        (Vec::new(), retiled)
    }

    /// The windows that came under the Floating layout since this was
    /// last asked, to be put in the middle of the display.
    pub fn take_arrivals(&mut self) -> Vec<WindowHandle> {
        std::mem::take(&mut self.arrivals)
    }

    /// Enable or disable the workspace float override.
    ///
    /// Enabling moves every tiled window to the floating ring and records it;
    /// disabling re-tiles exactly the recorded windows still present, leaving
    /// windows the user floated by hand alone. Returns the windows whose
    /// membership changed as `(floated, retiled)` so callers can fix up
    /// window styles and re-apply the layout.
    pub fn set_float_override(&mut self, enabled: bool) -> (Vec<WindowHandle>, Vec<WindowHandle>) {
        if self.float_override == enabled {
            return (Vec::new(), Vec::new());
        }
        self.float_override = enabled;

        if enabled {
            let floated: Vec<WindowHandle> = self
                .containers
                .iter()
                .flat_map(|container| container.windows.iter().copied())
                .collect();
            self.containers.clear();
            self.split_ratios.clear();
            self.bsp_split_ratios.clear();
            for handle in &floated {
                self.floating_windows.push(*handle);
                self.float_override_windows.insert(*handle);
            }
            (floated, Vec::new())
        } else {
            // Re-tile in floating-ring order so the restored container order
            // matches the order the windows were floated in.
            let mut retiled = Vec::new();
            let mut index = 0;
            while index < self.floating_windows.len() {
                let handle = self.floating_windows.elements()[index];
                if self.float_override_windows.contains(&handle) {
                    self.floating_windows.remove_at(index);
                    self.float_override_windows.remove(&handle);
                    self.add_window(handle, true);
                    retiled.push(handle);
                } else {
                    index += 1;
                }
            }
            (Vec::new(), retiled)
        }
    }

    /// Toggle pseudotiling for a tiled window, returning its new state.
    pub fn toggle_pseudotile(&mut self, handle: WindowHandle, preferred: Rect) -> bool {
        if self.pseudotile_sizes.remove(&handle).is_some() {
            false
        } else {
            self.pseudotile_sizes
                .insert(handle, (preferred.width.max(1), preferred.height.max(1)));
            true
        }
    }

    pub fn is_pseudotiled(&self, handle: WindowHandle) -> bool {
        self.pseudotile_sizes.contains_key(&handle)
    }

    /// Cycle the workspace Dwindle policy. Preserved split depths stay intact;
    /// newly created depths use the selected policy.
    pub fn cycle_dwindle_direction(&mut self) -> DwindleDirection {
        self.dwindle_direction = match self.dwindle_direction {
            DwindleDirection::Alternate => DwindleDirection::Horizontal,
            DwindleDirection::Horizontal => DwindleDirection::Vertical,
            DwindleDirection::Vertical => DwindleDirection::Alternate,
        };
        self.dwindle_direction
    }

    fn sync_dwindle_split_history(&mut self, container_count: usize) {
        let needed_splits = container_count.saturating_sub(1);
        if !self.dwindle_preserve_split {
            self.dwindle_split_history.clear();
        }
        self.dwindle_split_history.truncate(needed_splits);
        while self.dwindle_split_history.len() < needed_splits {
            let depth = self.dwindle_split_history.len();
            let direction = match self.dwindle_direction {
                DwindleDirection::Alternate if depth.is_multiple_of(2) => {
                    DwindleDirection::Horizontal
                }
                DwindleDirection::Alternate => DwindleDirection::Vertical,
                direction => direction,
            };
            self.dwindle_split_history.push(direction);
        }
    }

    /// Get all windows in this workspace (both tiled and floating)
    pub fn all_windows(&self) -> Vec<WindowHandle> {
        let mut windows = Vec::new();

        // Add tiled windows
        for container in self.containers.iter() {
            windows.extend(container.all_windows());
        }

        // Add floating windows
        windows.extend(self.floating_windows.iter().copied());

        windows
    }

    /// Find which container contains a window
    pub fn find_container(&self, handle: WindowHandle) -> Option<usize> {
        for (idx, container) in self.containers.elements().iter().enumerate() {
            if container.contains_window(handle) {
                return Some(idx);
            }
        }
        None
    }

    /// Apply the current layout to all containers
    pub fn apply_layout(&mut self, work_area: Rect) {
        if !self.tiling_enabled || self.containers.is_empty() {
            // Nothing is laid out: nothing is out of view either.
            self.bring_all_into_view();
            return;
        }

        // Workspace work-area offset:
        // applied last in the composition chain Windows work area → monitor
        // reservations → global offset → workspace offset.
        let work_area = self.work_area_offset.apply_to(work_area);

        // CRITICAL FIX: Don't drain containers - instead identify which ones should participate in layout
        // and apply layout only to those, while preserving ALL containers

        // First, identify indices of active containers (non-empty containers)
        // NOTE: We intentionally do NOT filter by minimized state here.
        // The minimized check was causing issues where windows that appeared minimized
        // during rapid startup enumeration would be skipped, leaving their containers
        // with 0x0 rects. The positioning code in executor.rs handles minimized windows
        // appropriately by checking state at actual positioning time.
        let active_indices: Vec<usize> = self.active_container_indices();

        if active_indices.is_empty() {
            log::trace!("No active containers to layout (all empty)");
            return;
        }

        log::trace!("📐 Applying {:?} layout to workspace '{}' with {} active containers (of {} total). Workspace padding: {:?}, Container padding: {:?}",
            self.layout, self.name, active_indices.len(), self.containers.len(), self.padding, self.container_padding);

        // Extract active containers for layout calculation
        let mut active_containers: Vec<Container> = active_indices
            .iter()
            .filter_map(|&idx| self.containers.get(idx).cloned())
            .collect();

        // Apply layout with workspace configuration (master_ratio, master_count)
        if self.layout == LayoutType::Dwindle {
            self.sync_dwindle_split_history(active_containers.len());
        }
        if self.layout == LayoutType::BSP {
            self.sync_bsp_split_ratios(active_containers.len());
        }

        // Out of view is the scrolling layout's word, said anew at every
        // pass; under any other layout every container is in view.
        for container in &mut active_containers {
            container.offscreen = false;
        }
        if self.layout == LayoutType::Scrolling {
            let room = work_area.with_padding(self.outer_gaps()).width.max(1);
            let widths: Vec<i32> = active_containers
                .iter()
                .map(|container| {
                    crate::layouts::scrolling_column_width(container.strip_width, room)
                })
                .collect();
            let focused = active_indices
                .iter()
                .position(|index| *index == self.containers.focused_idx())
                .unwrap_or(0);
            if self.sizes_places() {
                // The places in view are sized: the first column in view
                // leads, and every column is as wide as the place it
                // comes to as the view moves.
                // (The view stays where it was moved to while the keyboard
                // stays on the column it was on then.)
                let places = self.place_widths(room);
                if self.scroll_held != Some(focused) {
                    self.scroll_held = None;
                }
                let follow = if self.scroll_held.is_some() {
                    self.scroll_first
                } else {
                    focused
                };
                let lead = crate::layouts::scrolling_lead_column(
                    self.scroll_first,
                    follow,
                    &widths,
                    &places,
                    room,
                );
                let (placed, offset) =
                    crate::layouts::scrolling_lead_view(lead, &widths, &places, room);
                self.scroll_first = lead;
                self.scroll_offset = offset;
                // (For this pass only: each column keeps its own width.)
                for (container, width) in active_containers.iter_mut().zip(&placed) {
                    container.strip_width = *width as f32 / room as f32;
                }
            } else {
                self.scroll_first = crate::layouts::scrolling_first_column(
                    self.scroll_first,
                    focused,
                    &widths,
                    room,
                );
                // The view follows the keyboard, unless it was moved by
                // itself and the keyboard is still on the column it was on
                // then (looking along the strip does not snap back).
                if self.scroll_held != Some(focused) {
                    self.scroll_held = None;
                    self.scroll_offset = crate::layouts::scrolling_view_offset(
                        self.scroll_offset,
                        focused,
                        &widths,
                        room,
                    );
                } else {
                    self.scroll_offset =
                        crate::layouts::scrolling_scrolled(self.scroll_offset, 0, &widths, room);
                }
            }
        }

        let config = LayoutConfig {
            scroll_first: self.scroll_first,
            scroll_offset: (self.layout == LayoutType::Scrolling && !self.scroll_whole_columns)
                .then_some(self.scroll_offset),
            master_ratio: self.master_ratio,
            master_count: self.master_count,
            master_position: self.master_position,
            dwindle_direction: self.dwindle_direction,
            dwindle_preserve_split: self.dwindle_preserve_split,
            dwindle_split_history: self.dwindle_split_history.clone(),
            bsp_split_ratios: self.bsp_split_ratios.clone(),
            split: self.sizes.split_share(),
        };
        // (Columns and Rows: as resized by hand, else as configured.)
        let shares = if self.split_ratios.len() == active_containers.len() {
            self.split_ratios.clone()
        } else {
            self.sizes.shares(active_containers.len())
        };
        apply_layout_configured(
            self.layout,
            work_area,
            &mut active_containers,
            self.outer_gaps(),
            &shares,
            config,
        );

        if self.layout_rotation != 0 {
            for container in &mut active_containers {
                container.rect =
                    rotate_layout_rect(container.rect, work_area, self.layout_rotation);
            }
        }

        // Flip: mirror the laid-out rects across the
        // workspace mid-axis, after rotation.
        if self.layout_flip_horizontal || self.layout_flip_vertical {
            for container in &mut active_containers {
                container.rect = flip_layout_rect(
                    container.rect,
                    work_area,
                    self.layout_flip_horizontal,
                    self.layout_flip_vertical,
                );
            }
        }

        // Apply container padding (inner gaps between windows)
        // Use full gap value on each side - the workspace padding already handles outer edges,
        // and the layout algorithm places containers edge-to-edge, so each container needs
        // the full half-gap on each side to produce consistent gaps between adjacent windows.
        let inner = self.inner_gaps();
        if inner.left > 0 || inner.top > 0 || inner.right > 0 || inner.bottom > 0 {
            // Each edge uses its own gap; `right`/`bottom` used to be ignored
            // (left/top were applied to both sides).
            let gap_left = inner.left / 2;
            let gap_right = inner.right / 2;
            let gap_top = inner.top / 2;
            let gap_bottom = inner.bottom / 2;

            for container in active_containers.iter_mut() {
                let rect = container.rect;
                let new_rect = Rect::new(
                    rect.x + gap_left,
                    rect.y + gap_top,
                    (rect.width - gap_left - gap_right).max(1),
                    (rect.height - gap_top - gap_bottom).max(1),
                );
                container.set_rect(new_rect);
            }
            log::trace!("Applied container gaps: {:?}", self.container_padding);
        }

        // A window that will not be as small as its tile gets the room
        // its neighbours can spare. (Not where one window has the whole
        // display, nor on the strip, whose columns have their own widths.)
        if !self.smallest_sizes.is_empty()
            && !matches!(
                self.layout,
                LayoutType::Monocle | LayoutType::Floating | LayoutType::Scrolling
            )
        {
            // (A tile is as large as the largest that one of its windows
            // must be: the windows of a group lie on one another.)
            let smallest: Vec<(i32, i32)> = active_containers
                .iter()
                .map(|container| {
                    container
                        .windows
                        .iter()
                        .filter_map(|window| self.smallest_sizes.get(window))
                        .fold((0, 0), |most, size| {
                            (most.0.max(size.0), most.1.max(size.1))
                        })
                })
                .collect();
            let mut tiles: Vec<Rect> = active_containers
                .iter()
                .map(|container| container.rect)
                .collect();
            if crate::layouts::make_room(&mut tiles, &smallest) {
                for (container, tile) in active_containers.iter_mut().zip(tiles) {
                    container.set_rect(tile);
                }
            }
        }

        // Pseudotiled clients retain their preferred size while the container
        // still occupies its full place in the layout tree.
        for container in &mut active_containers {
            let Some(handle) = container.focused_window() else {
                continue;
            };
            let Some((preferred_width, preferred_height)) =
                self.pseudotile_sizes.get(&handle).copied()
            else {
                continue;
            };
            let tile = container.rect;
            container.set_rect(pseudotile_rect(tile, preferred_width, preferred_height));
        }

        // Write back the updated rects to the original containers
        for (layout_idx, &orig_idx) in active_indices.iter().enumerate() {
            if let Some(container) = self.containers.get_mut(orig_idx) {
                container.rect = active_containers[layout_idx].rect;
                let offscreen = active_containers[layout_idx].offscreen;
                if container.offscreen != offscreen {
                    container.offscreen = offscreen;
                    self.visibility_stale = true;
                }
            }
        }

        // log::debug!("Layout applied successfully");
    }

    /// Move the view along the strip by so many pixels (the scrolling
    /// layout; a wheel, fingers on a touchpad). It stays there until the
    /// keyboard goes to another column. Returns whether it moved.
    pub fn scroll_view(&mut self, by: i32, work_area: Rect) -> bool {
        if self.layout != LayoutType::Scrolling || self.scroll_whole_columns {
            return false;
        }
        let room = work_area.with_padding(self.outer_gaps()).width.max(1);
        if self.sizes_places() {
            return self.scroll_by_places(by, room);
        }
        let active: Vec<usize> = (0..self.containers.len())
            .filter(|index| {
                self.containers
                    .get(*index)
                    .is_some_and(|container| !container.windows.is_empty())
            })
            .collect();
        let widths: Vec<i32> = active
            .iter()
            .filter_map(|index| self.containers.get(*index))
            .map(|container| crate::layouts::scrolling_column_width(container.strip_width, room))
            .collect();
        let now = crate::layouts::scrolling_scrolled(self.scroll_offset, by, &widths, room);
        if now == self.scroll_offset {
            return false;
        }
        self.scroll_offset = now;
        let mut held = active
            .iter()
            .position(|index| *index == self.containers.focused_idx());
        // A column that left the view altogether does not keep the
        // keyboard: the one in the middle of the view takes it.
        let gone = held.is_some_and(|held| {
            let left: i32 = widths.iter().take(held).sum();
            let right = left + widths.get(held).copied().unwrap_or(0);
            right <= now || left >= now + room
        });
        if gone {
            let middle = crate::layouts::scrolling_column_in_the_middle(now, &widths, room);
            if let Some(index) = active.get(middle) {
                self.containers.focus_at(*index);
                held = Some(middle);
            }
        }
        self.scroll_held = held;
        true
    }

    /// The view moved by itself where the places in view are sized: by
    /// whole columns, a column each time what it was moved by comes to a
    /// third of the first column's width. The columns take the widths of
    /// the places they come to; the keyboard stays where it is while its
    /// column is in view, else goes to the first column in view.
    fn scroll_by_places(&mut self, by: i32, room: i32) -> bool {
        let (active, own, places) = self.strip_widths(room);
        if active.is_empty() {
            return false;
        }
        let (widths, _) =
            crate::layouts::scrolling_lead_view(self.scroll_first, &own, &places, room);
        let step = (widths.get(self.scroll_first).copied().unwrap_or(room) / 3).max(1);
        if self.scroll_pending.signum() != by.signum() {
            self.scroll_pending = 0;
        }
        self.scroll_pending = self.scroll_pending.saturating_add(by);
        if self.scroll_pending.abs() < step {
            return false;
        }
        let direction = self.scroll_pending.signum();
        self.scroll_pending = 0;
        let Some(lead) = crate::layouts::scrolling_lead_stepped(
            self.scroll_first,
            direction,
            &own,
            &places,
            room,
        ) else {
            return false;
        };
        self.scroll_first = lead;
        let (widths, offset) = crate::layouts::scrolling_lead_view(lead, &own, &places, room);
        self.scroll_offset = offset;
        let mut held = active
            .iter()
            .position(|index| *index == self.containers.focused_idx());
        let in_view = held.is_some_and(|held| {
            let left: i32 = widths.iter().take(held).sum();
            let right = left + widths.get(held).copied().unwrap_or(0);
            left >= offset && right <= offset + room
        });
        if !in_view {
            if let Some(index) = active.get(lead) {
                self.containers.focus_at(*index);
                held = Some(lead);
            }
        }
        self.scroll_held = held;
        true
    }

    /// Whether this workspace is a strip whose view can be moved by
    /// itself (the scrolling layout, where part of a column can be shown).
    pub fn scrolls_freely(&self) -> bool {
        self.layout == LayoutType::Scrolling && !self.scroll_whole_columns
    }

    /// No column is out of view any more (the layout is not applied:
    /// tiling is off). True when one was.
    pub fn bring_all_into_view(&mut self) -> bool {
        let mut any = false;
        for container in self.containers.elements_mut().iter_mut() {
            any |= std::mem::take(&mut container.offscreen);
        }
        self.visibility_stale |= any;
        any
    }

    /// Set the layout type
    pub fn set_layout(&mut self, layout: LayoutType) {
        self.layout = layout;
    }

    /// Cycle to next layout
    pub fn cycle_layout(&mut self) {
        let all_layouts = LayoutType::all();
        let current_idx = all_layouts
            .iter()
            .position(|&l| l == self.layout)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % all_layouts.len();
        self.layout = all_layouts[next_idx];
    }

    /// Get the focused container
    pub fn focused_container(&self) -> Option<&Container> {
        self.containers.focused()
    }

    /// Get the focused container mutably
    pub fn focused_container_mut(&mut self) -> Option<&mut Container> {
        self.containers.focused_mut()
    }

    /// Get the focused window
    pub fn focused_window(&self) -> Option<WindowHandle> {
        self.containers.focused()?.focused_window()
    }

    /// Focus the container and tab that own a managed window.
    pub fn focus_window(&mut self, handle: WindowHandle) -> bool {
        let Some(container_index) = self
            .containers
            .iter()
            .position(|container| container.contains_window(handle))
        else {
            let floating_index = self
                .floating_windows
                .iter()
                .position(|window| *window == handle);
            return floating_index.is_some_and(|index| self.floating_windows.focus_at(index));
        };
        self.containers.focus_at(container_index)
            && self
                .containers
                .focused_mut()
                .is_some_and(|container| container.focus_window(handle))
    }

    /// Focus next container
    pub fn focus_next_container(&mut self) {
        self.containers.focus_next();
    }

    /// Focus previous container
    pub fn focus_previous_container(&mut self) {
        self.containers.focus_previous();
    }

    /// Swap focused container with next container
    pub fn swap_container_with_next(&mut self) {
        self.containers.swap_with_next();
    }

    /// Swap focused container with previous container
    pub fn swap_container_with_previous(&mut self) {
        self.containers.swap_with_previous();
    }

    /// Focus container at index
    pub fn focus_container(&mut self, idx: usize) -> bool {
        self.containers.focus_at(idx)
    }

    /// Check if workspace is empty
    pub fn is_empty(&self) -> bool {
        self.containers.is_empty()
    }

    /// Number of containers
    pub fn container_count(&self) -> usize {
        self.containers.len()
    }

    /// Number of windows
    pub fn window_count(&self) -> usize {
        self.all_windows().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_add_remove_window() {
        let mut workspace = Workspace::new(0, "Test".to_string());
        let handle1 = WindowHandle(1);
        let handle2 = WindowHandle(2);

        workspace.add_window(handle1, true);
        workspace.add_window(handle2, true);

        assert_eq!(workspace.container_count(), 2);
        assert_eq!(workspace.window_count(), 2);

        workspace.remove_window(handle1);
        assert_eq!(workspace.container_count(), 1);
        assert_eq!(workspace.window_count(), 1);
    }

    #[test]
    fn test_workspace_layout_cycle() {
        let mut workspace = Workspace::new(0, "Test".to_string());

        let initial = workspace.layout;
        workspace.cycle_layout();
        let after_cycle = workspace.layout;

        assert_ne!(initial, after_cycle);
    }

    #[test]
    fn test_workspace_find_container() {
        let mut workspace = Workspace::new(0, "Test".to_string());
        let handle = WindowHandle(1);

        workspace.add_window(handle, true);

        let container_idx = workspace.find_container(handle);
        assert_eq!(container_idx, Some(0));
    }

    #[test]
    fn append_container_behaviour_stacks_new_windows_on_the_focused_container() {
        let mut workspace = Workspace::new(0, "Append".to_string());
        assert_eq!(
            workspace.window_container_behaviour,
            WindowContainerBehaviour::Column
        );

        // Column (default): every window gets its own container.
        workspace.add_window(WindowHandle(1), true);
        workspace.add_window(WindowHandle(2), true);
        assert_eq!(workspace.container_count(), 2);

        // Append: new windows join the focused container (auto-stack).
        assert_eq!(
            workspace.toggle_window_container_behaviour(),
            WindowContainerBehaviour::Append
        );
        workspace.focus_container(1);
        workspace.add_window(WindowHandle(3), true);
        assert_eq!(workspace.container_count(), 2);
        assert!(workspace
            .containers
            .get(1)
            .unwrap()
            .contains_window(WindowHandle(3)));

        // The first window on an empty workspace still creates a container.
        let mut empty = Workspace::new(1, "Empty append".to_string());
        empty.window_container_behaviour = WindowContainerBehaviour::Append;
        empty.add_window(WindowHandle(1), true);
        assert_eq!(empty.container_count(), 1);

        // Order-preserving re-adds (add_window_at) bypass the behaviour.
        workspace.add_window_at(WindowHandle(9), 0);
        assert_eq!(workspace.container_count(), 3);
        assert!(workspace
            .containers
            .get(0)
            .unwrap()
            .contains_window(WindowHandle(9)));

        // Toggling back restores column placement.
        assert_eq!(
            workspace.toggle_window_container_behaviour(),
            WindowContainerBehaviour::Column
        );
        workspace.add_window(WindowHandle(4), true);
        assert_eq!(workspace.container_count(), 4);

        // Explicit group-style adds (create_container = false) are untouched
        // by the mode in either direction.
        workspace.window_container_behaviour = WindowContainerBehaviour::Column;
        workspace.add_window(WindowHandle(5), false);
        assert_eq!(workspace.container_count(), 4);
    }

    #[test]
    fn focus_window_selects_its_container_and_group_tab() {
        let mut workspace = Workspace::new(0, "Test".to_string());
        workspace.add_window(WindowHandle(1), true);
        workspace.add_window(WindowHandle(2), true);
        workspace
            .containers
            .get_mut(0)
            .unwrap()
            .add_window(WindowHandle(3));

        assert!(workspace.focus_window(WindowHandle(3)));
        assert_eq!(workspace.focused_window(), Some(WindowHandle(3)));
        assert!(!workspace.focus_window(WindowHandle(99)));
    }

    #[test]
    fn master_new_window_placement_controls_container_order() {
        let mut master = Workspace::new(0, "Master".to_string());
        master.layout = LayoutType::MasterStack;
        master.new_window_placement = NewWindowPlacement::Master;
        master.add_window(WindowHandle(1), true);
        master.add_window(WindowHandle(2), true);
        assert_eq!(master.find_container(WindowHandle(2)), Some(0));
        assert_eq!(master.find_container(WindowHandle(1)), Some(1));

        let mut after_focused = Workspace::new(1, "After focused".to_string());
        after_focused.layout = LayoutType::MasterStack;
        after_focused.add_window(WindowHandle(1), true);
        after_focused.add_window(WindowHandle(2), true);
        after_focused.new_window_placement = NewWindowPlacement::AfterFocused;
        after_focused.focus_container(0);
        after_focused.add_window(WindowHandle(3), true);
        assert_eq!(after_focused.find_container(WindowHandle(3)), Some(1));
    }

    #[test]
    fn rotation_maps_columns_to_rows_and_round_trips() {
        let area = Rect::new(10, 20, 100, 200);
        let left = Rect::new(10, 20, 50, 200);
        assert_eq!(
            rotate_layout_rect(left, area, 1),
            Rect::new(10, 20, 100, 100)
        );

        let mut rect = Rect::new(23, 47, 31, 59);
        for _ in 0..4 {
            rect = rotate_layout_rect(rect, area, 1);
        }

        let original = Rect::new(23, 47, 31, 59);
        assert!((rect.x - original.x).abs() <= 1);
        assert!((rect.y - original.y).abs() <= 1);
        assert!((rect.width - original.width).abs() <= 1);
        assert!((rect.height - original.height).abs() <= 1);
    }

    #[test]
    fn dwindle_history_preserves_existing_depths_and_extends_with_policy() {
        let mut workspace = Workspace::new(0, "Dwindle".to_string());
        workspace.layout = LayoutType::Dwindle;
        workspace.dwindle_preserve_split = true;
        workspace.dwindle_direction = DwindleDirection::Horizontal;
        workspace.dwindle_split_history = vec![DwindleDirection::Vertical];

        workspace.sync_dwindle_split_history(4);
        assert_eq!(
            workspace.dwindle_split_history,
            vec![
                DwindleDirection::Vertical,
                DwindleDirection::Horizontal,
                DwindleDirection::Horizontal,
            ]
        );

        workspace.sync_dwindle_split_history(2);
        assert_eq!(
            workspace.dwindle_split_history,
            vec![DwindleDirection::Vertical]
        );
    }

    #[test]
    fn dwindle_history_rebuilds_when_preservation_is_disabled() {
        let mut workspace = Workspace::new(0, "Dwindle".to_string());
        workspace.dwindle_direction = DwindleDirection::Alternate;
        workspace.dwindle_split_history =
            vec![DwindleDirection::Vertical, DwindleDirection::Vertical];

        workspace.sync_dwindle_split_history(4);
        assert_eq!(
            workspace.dwindle_split_history,
            vec![
                DwindleDirection::Horizontal,
                DwindleDirection::Vertical,
                DwindleDirection::Horizontal,
            ]
        );
    }

    #[test]
    fn pseudotile_toggle_tracks_preferred_size_and_cleans_up() {
        let mut workspace = Workspace::new(0, "Dwindle".to_string());
        let handle = WindowHandle(42);
        workspace.add_window(handle, true);

        assert!(workspace.toggle_pseudotile(handle, Rect::new(0, 0, 640, 480)));
        assert!(workspace.is_pseudotiled(handle));
        assert_eq!(workspace.pseudotile_sizes[&handle], (640, 480));
        assert!(!workspace.toggle_pseudotile(handle, Rect::new(0, 0, 1, 1)));
        assert!(!workspace.is_pseudotiled(handle));

        workspace.toggle_pseudotile(handle, Rect::new(0, 0, 320, 200));
        workspace.remove_window(handle);
        assert!(!workspace.is_pseudotiled(handle));
    }

    #[test]
    fn pseudotile_geometry_centers_and_clamps_preferred_size() {
        let tile = Rect::new(100, 200, 800, 600);
        assert_eq!(
            pseudotile_rect(tile, 640, 480),
            Rect::new(180, 260, 640, 480)
        );
        assert_eq!(pseudotile_rect(tile, 1200, 900), tile);
    }

    #[test]
    fn float_override_floats_existing_and_new_windows_then_retiles_only_its_own() {
        let mut workspace = Workspace::new(0, "Override".to_string());
        workspace.add_window(WindowHandle(1), true);
        workspace.add_window(WindowHandle(2), true);
        // A window the user floated by hand before the override.
        workspace.add_floating_window(WindowHandle(9));

        let (floated, retiled) = workspace.set_float_override(true);
        assert_eq!(floated, vec![WindowHandle(1), WindowHandle(2)]);
        assert!(retiled.is_empty());
        assert!(workspace.float_override);
        assert_eq!(workspace.container_count(), 0);
        assert_eq!(workspace.floating_windows.len(), 3);

        // New windows added while the override is active float too.
        workspace.add_window(WindowHandle(3), true);
        assert_eq!(workspace.container_count(), 0);
        assert_eq!(workspace.floating_windows.len(), 4);

        // A window closed while floated by the override must not leak state.
        workspace.remove_window(WindowHandle(3));
        assert!(!workspace.float_override_windows.contains(&WindowHandle(3)));

        let (floated, retiled) = workspace.set_float_override(false);
        assert!(floated.is_empty());
        assert_eq!(retiled, vec![WindowHandle(1), WindowHandle(2)]);
        assert!(!workspace.float_override);
        assert!(workspace.find_container(WindowHandle(1)).is_some());
        assert!(workspace.find_container(WindowHandle(2)).is_some());
        // The hand-floated window stays floating.
        assert!(workspace
            .floating_windows
            .iter()
            .any(|window| *window == WindowHandle(9)));
        assert!(workspace.float_override_windows.is_empty());
    }

    #[test]
    fn float_override_is_idempotent() {
        let mut workspace = Workspace::new(0, "Override".to_string());
        workspace.add_window(WindowHandle(1), true);

        assert!(workspace.set_float_override(true).1.is_empty());
        let (floated, retiled) = workspace.set_float_override(true);
        assert!(floated.is_empty() && retiled.is_empty());
        assert!(workspace.float_override);
        assert_eq!(workspace.floating_windows.len(), 1);
    }

    #[test]
    fn flip_layout_rect_mirrors_across_the_work_area_mid_axis() {
        let area = Rect::new(10, 20, 100, 200);
        let rect = Rect::new(10, 20, 30, 50);
        assert_eq!(
            flip_layout_rect(rect, area, true, false),
            Rect::new(80, 20, 30, 50)
        );
        assert_eq!(
            flip_layout_rect(rect, area, false, true),
            Rect::new(10, 170, 30, 50)
        );
        // Mirroring twice restores the original rect.
        let flipped = flip_layout_rect(rect, area, true, true);
        assert_eq!(flip_layout_rect(flipped, area, true, true), rect);
    }

    #[test]
    fn layout_flip_applies_after_layout_and_toggles_off() {
        let mut workspace = Workspace::new(0, "Flip".to_string());
        workspace.padding = Padding::default();
        workspace.container_padding = Padding::default();
        workspace.add_window(WindowHandle(1), true);
        workspace.add_window(WindowHandle(2), true);
        let work_area = Rect::new(0, 0, 1000, 1000);

        workspace.apply_layout(work_area);
        let left_before = workspace.containers.get(0).unwrap().rect;
        let right_before = workspace.containers.get(1).unwrap().rect;

        workspace.toggle_layout_flip(LayoutAxis::Horizontal);
        workspace.apply_layout(work_area);
        assert_eq!(workspace.containers.get(0).unwrap().rect, right_before);
        assert_eq!(workspace.containers.get(1).unwrap().rect, left_before);

        workspace.toggle_layout_flip(LayoutAxis::Horizontal);
        workspace.apply_layout(work_area);
        assert_eq!(workspace.containers.get(0).unwrap().rect, left_before);
        assert_eq!(workspace.containers.get(1).unwrap().rect, right_before);
    }

    #[test]
    fn container_gaps_use_each_edge() {
        let mut workspace = Workspace::new(0, "Gaps".to_string());
        workspace.padding = Padding::default();
        workspace.container_padding = Padding::new(10, 4, 30, 8);
        workspace.add_window(WindowHandle(1), true);

        workspace.apply_layout(Rect::new(0, 0, 1000, 1000));
        // left 10/2, top 4/2, right 30/2, bottom 8/2.
        assert_eq!(
            workspace.containers.get(0).unwrap().rect,
            Rect::new(5, 2, 1000 - 5 - 15, 1000 - 2 - 4)
        );
    }

    #[test]
    fn removing_a_container_drops_its_own_bsp_split() {
        let setup = || {
            let mut workspace = Workspace::new(0, "Bsp".to_string());
            workspace.layout = crate::layouts::LayoutType::BSP;
            for handle in 1..=3 {
                workspace.add_window(WindowHandle(handle), true);
            }
            workspace.apply_layout(Rect::new(0, 0, 1000, 1000));
            // Split 0 separates window 1 from {2, 3}; split 1 separates 2 from 3.
            workspace.bsp_split_ratios = vec![0.3, 0.7];
            workspace
        };

        let mut first = setup();
        first.remove_window(WindowHandle(1));
        assert_eq!(first.bsp_ratios(), &[0.7]);

        let mut middle = setup();
        middle.remove_window(WindowHandle(2));
        assert_eq!(middle.bsp_ratios(), &[0.3]);

        let mut last = setup();
        last.remove_window(WindowHandle(3));
        assert_eq!(last.bsp_ratios(), &[0.3]);
    }

    #[test]
    fn workspace_work_area_offset_shrinks_the_layout_area() {
        let mut workspace = Workspace::new(0, "Offset".to_string());
        workspace.padding = Padding::default();
        workspace.container_padding = Padding::default();
        workspace.add_window(WindowHandle(1), true);
        workspace.work_area_offset = gevrek_model::settings::ReservedArea {
            left: 100,
            top: 50,
            right: 200,
            bottom: 150,
        };

        workspace.apply_layout(Rect::new(0, 0, 1000, 1000));
        assert_eq!(
            workspace.containers.get(0).unwrap().rect,
            Rect::new(100, 50, 700, 800)
        );
    }

    #[test]
    fn toggle_monocle_remembers_and_restores_the_previous_layout() {
        let mut workspace = Workspace::new(0, "Monocle".to_string());
        workspace.layout = LayoutType::BSP;

        assert_eq!(workspace.toggle_monocle(), Ok(true));
        assert_eq!(workspace.layout, LayoutType::Monocle);
        assert_eq!(workspace.toggle_monocle(), Ok(false));
        assert_eq!(workspace.layout, LayoutType::BSP);

        // Changing the layout mid-monocle drops the stale restore point.
        assert_eq!(workspace.toggle_monocle(), Ok(true));
        workspace.layout = LayoutType::Grid;
        assert_eq!(workspace.toggle_monocle(), Ok(true));
        assert_eq!(workspace.layout, LayoutType::Monocle);
        assert_eq!(workspace.toggle_monocle(), Ok(false));
        assert_eq!(workspace.layout, LayoutType::Grid);

        // A workspace already on the monocle layout has nothing to toggle to.
        workspace.layout = LayoutType::Monocle;
        workspace.monocle_restore_layout = None;
        assert!(workspace.toggle_monocle().is_err());
    }

    #[test]
    fn bsp_resize_edge_adjusts_the_stored_split_ratio() {
        let mut workspace = Workspace::new(0, "BSP".to_string());
        workspace.padding = Padding::default();
        workspace.container_padding = Padding::default();
        workspace.layout = LayoutType::BSP;
        for handle in [1, 2, 3, 4].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        let work_area = Rect::new(0, 0, 1000, 1000);

        // Grow container 0's right edge by 100px of the 1000px work area.
        assert!(workspace.resize_container_edge(0, EdgeDirection::Right, 100, work_area));
        assert!((workspace.bsp_split_ratios[0] - 0.6).abs() < 1e-6);

        // Container 3's right edge is the workspace boundary: no-op.
        assert!(!workspace.resize_container_edge(3, EdgeDirection::Right, 100, work_area));

        // The stored ratios drive the layout.
        workspace.apply_layout(work_area);
        assert_eq!(workspace.containers.get(0).unwrap().rect.width, 600);
    }

    #[test]
    fn bsp_resize_axis_falls_back_to_the_trailing_edge() {
        let mut workspace = Workspace::new(0, "BSP".to_string());
        workspace.padding = Padding::default();
        workspace.layout = LayoutType::BSP;
        for handle in [1, 2].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        let work_area = Rect::new(0, 0, 1000, 1000);

        // Container 1 has no forward horizontal split; growing it
        // horizontally pulls the split-1 boundary left instead.
        assert!(workspace.resize_container_axis(1, LayoutAxis::Horizontal, 100, work_area));
        assert!((workspace.bsp_split_ratios[0] - 0.4).abs() < 1e-6);

        // Single window: nothing to resize.
        workspace.remove_window(WindowHandle(2));
        assert!(!workspace.resize_container_axis(0, LayoutAxis::Horizontal, 100, work_area));
    }

    #[test]
    fn master_layout_axis_resize_maps_to_the_master_ratio() {
        let mut workspace = Workspace::new(0, "Master".to_string());
        workspace.padding = Padding::default();
        workspace.layout = LayoutType::MasterStack;
        workspace.master_ratio = 0.5;
        for handle in [1, 2].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        let work_area = Rect::new(0, 0, 1000, 1000);

        // Growing the master (index 0) horizontally raises the master ratio.
        assert!(workspace.resize_container_axis(0, LayoutAxis::Horizontal, 100, work_area));
        assert!((workspace.master_ratio - 0.6).abs() < 1e-6);

        // Growing the stack window shrinks the master share.
        assert!(workspace.resize_container_axis(1, LayoutAxis::Horizontal, 100, work_area));
        assert!((workspace.master_ratio - 0.5).abs() < 1e-6);

        // Master on the left: the vertical axis has no adjustable split.
        assert!(!workspace.resize_container_axis(0, LayoutAxis::Vertical, 100, work_area));

        // Edge resize: the master's right edge faces the split, its left edge
        // is the workspace boundary.
        assert!(workspace.resize_container_edge(0, EdgeDirection::Right, 50, work_area));
        assert!((workspace.master_ratio - 0.55).abs() < 1e-6);
        assert!(!workspace.resize_container_edge(0, EdgeDirection::Left, 50, work_area));
    }

    #[test]
    fn a_container_that_leaves_or_comes_by_a_group_takes_only_its_own_split() {
        let mut workspace = Workspace::new(0, "BSP".to_string());
        workspace.layout = LayoutType::BSP;
        for handle in [1, 2, 3].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        workspace.bsp_split_ratios = vec![0.3, 0.7];
        // The first container joins the second: its split goes, the
        // other keeps its share (it was 0.3 that stayed).
        workspace.containers.remove_at(0);
        workspace.container_left(0);
        assert_eq!(workspace.bsp_split_ratios, vec![0.7]);
        // A window leaves its group after the first container: an even
        // split comes in there.
        workspace.add_window_at(WindowHandle(9), 1);
        workspace.container_came(1);
        assert_eq!(workspace.bsp_split_ratios, vec![0.7, 0.5]);
    }

    #[test]
    fn dwindle_axis_resize_moves_the_ratio_every_room_is_shared_by() {
        let mut workspace = Workspace::new(0, "Dwindle".to_string());
        workspace.padding = Padding::default();
        workspace.layout = LayoutType::Dwindle;
        workspace.master_ratio = 0.5;
        for handle in [1, 2, 3].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        let work_area = Rect::new(0, 0, 1000, 1000);
        // One window alone has nothing to be resized against.
        let mut alone = Workspace::new(1, "Alone".to_string());
        alone.layout = LayoutType::Dwindle;
        alone.add_window(WindowHandle(9), true);
        assert!(!alone.resize_container_axis(0, LayoutAxis::Horizontal, 100, work_area));

        // The first window keeps the left of the first split: wider is
        // more of it.
        assert!(workspace.resize_container_axis(0, LayoutAxis::Horizontal, 100, work_area));
        assert!((workspace.master_ratio - 0.6).abs() < 1e-6);
        // It is as high as the display: nothing to resize that way.
        assert!(!workspace.resize_container_axis(0, LayoutAxis::Vertical, 100, work_area));
        // The second lies in the rest of the first split (wider is less
        // for the first) and keeps the top of the second (higher is more).
        assert!(workspace.resize_container_axis(1, LayoutAxis::Horizontal, 100, work_area));
        assert!((workspace.master_ratio - 0.5).abs() < 1e-6);
        assert!(workspace.resize_container_axis(1, LayoutAxis::Vertical, 100, work_area));
        assert!((workspace.master_ratio - 0.6).abs() < 1e-6);
        // The last keeps nothing of its own: it grows by what the ones
        // before it give.
        assert!(workspace.resize_container_axis(2, LayoutAxis::Vertical, 100, work_area));
        assert!((workspace.master_ratio - 0.5).abs() < 1e-6);
        // Never past what the layout draws.
        for _ in 0..10 {
            assert!(workspace.resize_container_axis(0, LayoutAxis::Horizontal, 100, work_area));
        }
        assert!((workspace.master_ratio - 0.7).abs() < 1e-6);
    }

    #[test]
    fn columns_axis_resize_rebalances_split_ratios() {
        let mut workspace = Workspace::new(0, "Columns".to_string());
        workspace.padding = Padding::default();
        workspace.layout = LayoutType::Columns;
        for handle in [1, 2, 3].map(WindowHandle) {
            workspace.add_window(handle, true);
        }
        let work_area = Rect::new(0, 0, 900, 1000);

        assert!(workspace.resize_container_axis(0, LayoutAxis::Horizontal, 90, work_area));
        assert!((workspace.split_ratios[0] - (1.0 / 3.0 + 0.1)).abs() < 1e-6);
        let sum: f32 = workspace.split_ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);

        // Columns has no vertical split to resize.
        assert!(!workspace.resize_container_axis(0, LayoutAxis::Vertical, 90, work_area));
    }

    #[test]
    fn a_floating_window_has_the_floating_look_where_there_is_one() {
        let mut workspace = Workspace::new(0, "one".to_string());
        workspace.border_width = 4;
        workspace.active_border_color = (1, 2, 3);
        workspace.floating_look.border_width = 2;
        workspace.floating_look.active_border_color = None;
        workspace.floating_look.inactive_border_color = Some((9, 9, 9));
        workspace.add_window(WindowHandle(1), true);
        workspace.add_floating_window(WindowHandle(2));
        assert!(!workspace.floats(WindowHandle(1)) && workspace.floats(WindowHandle(2)));

        let tiled = workspace.border_look(false);
        assert_eq!((tiled.width, tiled.floating), (4, false));
        let floating = workspace.border_look(true);
        assert_eq!((floating.width, floating.floating), (2, true));
        assert_eq!(
            floating.active_color,
            (1, 2, 3),
            "a colour not given is the tiled one"
        );
        assert_eq!(floating.inactive_color, (9, 9, 9));
        assert_eq!(floating.style, tiled.style);

        // Not set apart: floating windows look as the tiled ones do.
        workspace.floating_look.separate = false;
        assert_eq!(workspace.border_look(true), tiled);
        // A workspace without borders has none on floating windows either.
        workspace.floating_look.separate = true;
        workspace.border_enabled = false;
        assert!(!workspace.border_look(true).enabled);
    }
}
