//! The Wayland desktop as the engine sees it: `WlDesktop` answers the
//! engine's window queries from the compositor's own records and carries
//! out its effects on xdg toplevels.
//!
//! A Wayland compositor decides where windows are, so positions live here
//! (the client only learns its size). What needs the seat (keyboard focus)
//! is recorded and applied by the compositor state after the core returns.

use crate::surface::{StateFlag, WindowSurface};
use gevrek_engine::contract::{
    BorderChange, Desktop, Effect, StackOp, StackbarContainerState, StateChange, WindowQueries,
};
use gevrek_engine::events::WindowEvent;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use gevrek_model::settings::{RuleColor, ShadowPolicy};
use gevrek_model::window::WindowInfo;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The border the compositor draws around a window at render time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: i32,
    pub offset: i32,
    pub active_color: (u8, u8, u8),
    pub inactive_color: (u8, u8, u8),
    pub active_alpha: u8,
    pub inactive_alpha: u8,
    pub focused: bool,
    pub visible: bool,
    /// The corners are rounded (the `Rounded` border style).
    pub rounded: bool,
    pub style: gevrek_model::graph::BorderStyle,
}

/// The painted corner tiles of a rounded border, kept until its look
/// changes.
pub struct CornerTiles {
    /// Width, colour and alpha they were painted for.
    pub look: (i32, (u8, u8, u8), u8),
    pub tiles: [smithay::backend::renderer::element::memory::MemoryRenderBuffer; 4],
}

/// One window: a Wayland toplevel or an X11 window.
pub struct WlWindow {
    pub surface: WindowSurface,
    pub title: String,
    pub app_id: String,
    pub initial_title: String,
    pub initial_app_id: String,
    pub pid: Option<u32>,
    /// File name and full path of the client's program, read once from
    /// `/proc/<pid>/exe`.
    pub exe: String,
    pub path: String,
    /// Where the window is: the position is the compositor's, the size is
    /// what the WM asked for until the client commits its own.
    pub rect: Rect,
    /// The size the WM last asked the client for. A commit of another
    /// size while the client has not yet answered that request is an
    /// answer to an older one (see `State::toplevel_committed`).
    pub requested: Option<(i32, i32)>,
    /// The size the client last drew itself at (after any cut to a
    /// tile): what `rect` is brought back to when a resize by the pointer
    /// ends, since the last size asked for may be one the client did not
    /// take (a terminal keeps to whole rows).
    pub committed: Option<(i32, i32)>,
    /// The glow around this window, where it is another than the
    /// configuration's for every window (a floating window's).
    pub glow: Option<gevrek_engine::contract::GlowStrength>,
    /// Whether the client was told it is tiled.
    pub tiled: bool,
    /// The client drew itself larger than the tile it was asked to fill
    /// (it has a smallest size): `rect` is the tile, and what the surface
    /// has beyond it is neither drawn nor given the pointer.
    pub overflows: bool,
    /// The picture its program gave it (xdg-toplevel-icon): a name out
    /// of the icon theme, or the path of a file of its pixels; empty
    /// where it gave none.
    pub icon: String,
    /// Where the window goes back to when it leaves fullscreen or stops
    /// being maximized (and where the layout puts it meanwhile).
    pub restore: Option<Rect>,
    /// The client attached a buffer: the window can be shown.
    pub mapped: bool,
    /// It was shown at a place of its own: where its X11 program put it,
    /// or over the window it belongs to (`State::own_starting_place`), not
    /// where windows open when nothing says where.
    pub placed_itself: bool,
    /// The WM hides it (its workspace is not shown).
    pub hidden_by_wm: bool,
    /// When the WM hid it last: a workspace that is put away slides out,
    /// and its windows are those hidden just before.
    pub hidden_at: Option<std::time::Instant>,
    /// Hidden, and drawn all the same while it is on its way: out (its
    /// workspace gives way to another) or in and perhaps out again (the
    /// workspace beside, while fingers on a touchpad pull it in). Over
    /// when the way is gone.
    pub passing: bool,
    /// Drawn although it may be hidden: the overview of every workspace
    /// is up and shows it in its workspace's picture.
    pub in_overview: bool,
    /// Its exit was drawn and its program asked to close, at this time:
    /// it is not shown again while the program takes its moment to go
    /// (only if it stays, which a program that asks first may).
    pub closing: Option<std::time::Instant>,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub always_on_top: bool,
    pub opacity: u8,
    pub border: Option<Border>,
    /// Where the window's visible frame starts inside its surface (a client
    /// that draws its own shadow sets a geometry smaller than the buffer).
    pub geometry_offset: (i32, i32),
    /// Identities of the four border pieces for damage tracking, and a
    /// counter that changes whenever the border looks different.
    pub border_ids: [smithay::backend::renderer::element::Id; 4],
    pub border_commit: smithay::backend::renderer::utils::CommitCounter,
    /// Identity of the tint over the window, for damage tracking.
    pub tint_id: smithay::backend::renderer::element::Id,
    /// Where the window is shown while an overview is up: its place in
    /// the overview's grid, at its own shape. The window itself stays as
    /// large as it is and where it is (`rect`); only its picture is drawn
    /// there, made smaller.
    pub overview: Option<Rect>,
    /// Where the window is drawn while a transition runs (`rect` is where
    /// it ends), and how opaque while it fades in.
    pub drawn: Option<Rect>,
    pub drawn_opacity: Option<u8>,
    /// The border colour and alpha while it changes between the unfocused
    /// and the focused look.
    pub drawn_border: Option<((u8, u8, u8), u8)>,
    /// Identities of the extra pieces some border styles draw (dashes,
    /// the two lines of a double border), made as they are needed.
    pub extra_ids: std::cell::RefCell<Vec<smithay::backend::renderer::element::Id>>,
    /// The picture of shadow, glow and border around the window when the
    /// drawn effects are on, with what it was painted for (painted again
    /// when that changes).
    pub aura: std::cell::RefCell<
        Option<(
            gevrek_engine::decoration_render::DecorationSpec,
            smithay::backend::renderer::element::memory::MemoryRenderBuffer,
        )>,
    >,
    /// The same picture as it was drawn, kept to turn its border's
    /// gradient without drawing the shadow and the glow again.
    pub aura_drawing: std::cell::RefCell<Option<gevrek_engine::decoration_render::Prepared>>,
    /// The corner tiles of a rounded border, painted when first drawn.
    pub corner_tiles: std::cell::RefCell<Option<CornerTiles>>,
    /// The client is gone; the record lives until the core has been told.
    pub alive: bool,
}

/// A window's picture in a place of the overview: at the window's own
/// shape, as large as fits the place and no larger than the window is,
/// in the middle of the place.
pub fn fitted(window: Rect, place: Rect) -> Rect {
    if window.width <= 0 || window.height <= 0 || place.width <= 0 || place.height <= 0 {
        return place;
    }
    let scale = (f64::from(place.width) / f64::from(window.width))
        .min(f64::from(place.height) / f64::from(window.height))
        .min(1.0);
    let width = ((f64::from(window.width) * scale).round() as i32).max(1);
    let height = ((f64::from(window.height) * scale).round() as i32).max(1);
    Rect::new(
        place.x + (place.width - width) / 2,
        place.y + (place.height - height) / 2,
        width,
        height,
    )
}

/// The order of the windows, bottom to top, after one of them is raised:
/// it goes to the top, and the windows that belong to it (its dialogs,
/// and theirs) above it, in the order they had among themselves.
pub fn raised(
    stacking: &[WindowHandle],
    id: WindowHandle,
    parent_of: impl Fn(WindowHandle) -> Option<WindowHandle>,
) -> Vec<WindowHandle> {
    if !stacking.contains(&id) {
        return stacking.to_vec();
    }
    // (No further up than there are windows: two that name each other
    // end nowhere.)
    let belongs = |window: WindowHandle| {
        let mut above = parent_of(window);
        for _ in 0..stacking.len() {
            match above {
                Some(found) if found == id => return true,
                Some(found) => above = parent_of(found),
                None => return false,
            }
        }
        false
    };
    let with_it: Vec<WindowHandle> = stacking
        .iter()
        .copied()
        .filter(|window| *window != id && belongs(*window))
        .collect();
    let mut order: Vec<WindowHandle> = stacking
        .iter()
        .copied()
        .filter(|window| *window != id && !with_it.contains(window))
        .collect();
    order.push(id);
    order.extend(with_it);
    order
}

/// A colour: red, green, blue.
type Rgb = (u8, u8, u8);

/// The tab strip of one container that holds several windows.
pub struct Stackbar {
    /// The container's focused window: the bar is shown with it.
    pub key: WindowHandle,
    /// Where the strip is, in global coordinates.
    pub rect: Rect,
    /// The windows of the tabs, in tab order.
    pub tabs: Vec<WindowHandle>,
    pub titles: Vec<String>,
    pub focused_tab: usize,
    /// The painted strip.
    pub buffer: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    colors: (Rgb, Rgb, Rgb),
    /// How many pixels of the picture one logical pixel is.
    scale: i32,
}

/// One output with its place in the global coordinate space.
#[derive(Debug, Clone)]
pub struct WlOutput {
    pub id: MonitorHandle,
    pub name: String,
    /// Who the display is wherever it is plugged in
    /// (`MonitorInfo::stable_id`): of its EDID on hardware, of its name
    /// where there is no display behind it (nested, headless).
    pub stable_id: String,
    pub rect: Rect,
    /// The part of it no bar reserves: where windows go.
    pub work: Rect,
    /// What a maximized window fills: `work` less what the configuration
    /// and gevrek's own bar keep free (kept current by `State::settle`).
    pub usable: Rect,
    /// How many device pixels one logical pixel is (`rect` is logical).
    pub scale: f64,
    /// The display's mode in its own pixels, as the display lies: `rect`
    /// is this turned as `transform` says and divided by the scale.
    pub mode: (i32, i32),
    /// How the picture is turned for a display that stands on its side or
    /// on its head (`desktop.outputs`, `transform`).
    pub transform: gevrek_model::outputs::OutputTransform,
}

/// The size of a picture that is upright for someone looking at a display
/// turned this way: a display on its side is as wide as it was high.
pub fn turned_size(
    transform: gevrek_model::outputs::OutputTransform,
    (width, height): (i32, i32),
) -> (i32, i32) {
    use gevrek_model::outputs::OutputTransform as T;
    match transform {
        T::Normal | T::UpsideDown => (width, height),
        T::Right | T::Left => (height, width),
    }
}

/// How the compositor turns what it draws for a display turned this way.
/// A display turned a quarter to the right is looked at from its left
/// side: the picture is turned a quarter to the left on it (90 as Wayland
/// counts, against the clock).
pub fn turn_of(transform: gevrek_model::outputs::OutputTransform) -> smithay::utils::Transform {
    use gevrek_model::outputs::OutputTransform as T;
    use smithay::utils::Transform;
    match transform {
        T::Normal => Transform::Normal,
        T::Right => Transform::_90,
        T::UpsideDown => Transform::_180,
        T::Left => Transform::_270,
    }
}

/// A place on the display as its device says it (0 to 1 from its own
/// top-left corner, as the display lies), in the upright picture.
pub fn turned_place(
    transform: gevrek_model::outputs::OutputTransform,
    (x, y): (f64, f64),
) -> (f64, f64) {
    use gevrek_model::outputs::OutputTransform as T;
    match transform {
        T::Normal => (x, y),
        T::Right => (1.0 - y, x),
        T::UpsideDown => (1.0 - x, 1.0 - y),
        T::Left => (y, 1.0 - x),
    }
}

pub struct WlDesktop {
    pub windows: BTreeMap<WindowHandle, WlWindow>,
    /// The windows that stand on a strip (the scrolling layout), each
    /// with the display its strip is on: such a window may reach beyond
    /// that display's edge, and is drawn and reached on that display
    /// alone (the part beyond is cut off, not shown on the display beside).
    pub homes: std::collections::HashMap<WindowHandle, MonitorHandle>,
    /// The overview that is up is the one of every workspace
    /// (`overview.rs`): a place a window is given meanwhile is its place,
    /// not a place in an overview.
    pub overview_all: bool,
    /// The next change of workspace is made at once, with no sliding or
    /// fading (the overview of every workspace shows it its own way).
    pub plain_workspace_change: bool,
    /// Fingers on a touchpad are pulling the workspace beside into view
    /// (`gestures.rs`): what is pulled, and how far.
    pub follow_workspace: Option<FollowWorkspace>,
    /// Windows are put where they belong at once, with no transition:
    /// set while the view of a strip follows fingers on a touchpad.
    pub follow: bool,
    /// Bottom to top.
    pub stacking: Vec<WindowHandle>,
    pub outputs: Vec<WlOutput>,
    pub focused: Option<WindowHandle>,
    /// The windows that have had the focus at least once (a focus the WM
    /// took back at once, by a rule, does not count).
    pub had_focus: std::collections::HashSet<WindowHandle>,
    /// The focus changed since the compositor last gave it to the seat.
    pub focus_dirty: bool,
    /// What the desktop did that the core must hear about (a focus effect
    /// is answered with `Focused`, as the OS does on Windows).
    pub pending_events: Vec<WindowEvent>,
    /// Where the pointer is, in global logical coordinates.
    pub pointer: (i32, i32),
    /// The pointer was moved by a command, not by a device.
    pub pointer_warped: bool,
    pub primary_button_down: bool,
    /// Borders and bars are not drawn (an overview is shown).
    pub decorations_visible: bool,
    /// The tab strips to draw.
    pub stackbars: Vec<Stackbar>,
    /// Something changed that needs a new frame.
    pub needs_redraw: bool,
    /// The transitions in flight. They only change where and how windows
    /// are drawn: a window's rectangle is its end state at once, and its
    /// client is asked for the final size once.
    pub animations: gevrek_engine::animation_scheduler::RuntimeAnimationScheduler,
    rules: Arc<Mutex<RuleEngine>>,
    next_window: isize,
}

/// How long a window that was asked to close stays out of the picture
/// before it is taken to have stayed.
const CLOSING_GRACE: std::time::Duration = std::time::Duration::from_millis(1000);

/// The workspace beside, pulled into view by fingers on a touchpad: the
/// windows shown go out by as much as the fingers went, those of the
/// other workspace come in beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowWorkspace {
    /// The windows that are shown, with their places.
    pub going: Vec<(WindowHandle, Rect)>,
    /// The windows of the workspace beside, with their places there.
    pub coming: Vec<(WindowHandle, Rect)>,
    /// How far the workspace beside lies from its place when nothing is
    /// pulled: a display's width to the right (positive) or left, or its
    /// height below or above.
    pub apart: (i32, i32),
    /// How much of that way is gone, 0 to 1.
    pub gone: f32,
}

/// Windows hidden no longer ago than this were hidden by the change of
/// workspace that is being shown now (the same pass of the window manager).
const JUST_HIDDEN: std::time::Duration = std::time::Duration::from_millis(120);

impl WlDesktop {
    pub fn new(rules: Arc<Mutex<RuleEngine>>) -> Self {
        Self {
            windows: BTreeMap::new(),
            stacking: Vec::new(),
            outputs: Vec::new(),
            focused: None,
            had_focus: Default::default(),
            focus_dirty: false,
            pending_events: Vec::new(),
            pointer: (0, 0),
            pointer_warped: false,
            primary_button_down: false,
            decorations_visible: true,
            stackbars: Vec::new(),
            needs_redraw: true,
            animations: Default::default(),
            rules,
            next_window: 0,
            homes: Default::default(),
            follow: false,
            follow_workspace: None,
            overview_all: false,
            plain_workspace_change: false,
        }
    }

    /// Start tracking a new window. It is not shown, and the core is not
    /// told, until the client attaches a buffer.
    pub fn track(&mut self, surface: WindowSurface, pid: Option<u32>) -> WindowHandle {
        self.next_window += 1;
        let id = WindowHandle(self.next_window);
        let path = pid
            .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let exe = path.rsplit('/').next().unwrap_or_default().to_string();
        self.windows.insert(
            id,
            WlWindow {
                surface,
                title: String::new(),
                app_id: String::new(),
                initial_title: String::new(),
                initial_app_id: String::new(),
                pid,
                exe,
                path,
                rect: Rect::new(0, 0, 0, 0),
                requested: None,
                committed: None,
                glow: None,
                overflows: false,
                icon: String::new(),
                tiled: false,
                restore: None,
                mapped: false,
                placed_itself: false,
                hidden_by_wm: false,
                hidden_at: None,
                passing: false,
                in_overview: false,
                closing: None,
                minimized: false,
                maximized: false,
                fullscreen: false,
                always_on_top: false,
                opacity: 255,
                border: None,
                geometry_offset: (0, 0),
                border_ids: std::array::from_fn(|_| smithay::backend::renderer::element::Id::new()),
                border_commit: Default::default(),
                tint_id: smithay::backend::renderer::element::Id::new(),
                overview: None,
                drawn: None,
                drawn_opacity: None,
                drawn_border: None,
                aura: Default::default(),
                aura_drawing: Default::default(),
                corner_tiles: Default::default(),
                extra_ids: Default::default(),
                alive: true,
            },
        );
        self.stacking.push(id);
        id
    }

    pub fn id_of(&self, surface: &WindowSurface) -> Option<WindowHandle> {
        self.windows
            .iter()
            .find(|(_, window)| &window.surface == surface)
            .map(|(id, _)| *id)
    }

    pub fn id_of_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<WindowHandle> {
        self.windows
            .iter()
            .find(|(_, window)| window.surface.wl_surface().as_ref() == Some(surface))
            .map(|(id, _)| *id)
    }

    fn get(&self, id: WindowHandle) -> Option<&WlWindow> {
        self.windows.get(&id).filter(|window| window.alive)
    }

    /// A window that is on screen now, to be used: it can have the
    /// keyboard and the pointer.
    pub fn shown(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| {
            window.mapped
                && !window.hidden_by_wm
                && !window.minimized
                && !window
                    .closing
                    .is_some_and(|since| since.elapsed() < CLOSING_GRACE)
        })
    }

    /// A window that is hidden already and still drawn on its way out: a
    /// column that goes out of view on its strip, a window of a workspace
    /// that slides away. Nothing reaches it any more.
    pub fn leaving(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| {
            window.hidden_by_wm
                && ((window.drawn.is_some() && self.homes.contains_key(&id))
                    || (window.passing
                        && (window.drawn.is_some() || window.drawn_opacity.is_some())))
                && window.mapped
                && !window.minimized
                && window.closing.is_none()
        })
    }

    /// Windows that were asked to close and are still there after their
    /// moment are shown again (the program kept them: it may have asked
    /// the person first). Returns whether one was. Called a few times a
    /// second.
    pub fn tick_closing(&mut self) -> bool {
        let mut back = false;
        for window in self.windows.values_mut() {
            if window
                .closing
                .is_some_and(|since| since.elapsed() >= CLOSING_GRACE)
            {
                window.closing = None;
                back = true;
            }
        }
        if back {
            self.needs_redraw = true;
        }
        back
    }

    /// Nothing is drawn any more that is hidden and was on its way.
    fn end_passing(&mut self) {
        for window in self.windows.values_mut() {
            if window.passing {
                window.passing = false;
                window.drawn = None;
                window.drawn_opacity = None;
            }
        }
    }

    /// Fingers pull the workspace beside into view, or pull it further:
    /// both workspaces are drawn where the pull has them now.
    pub fn follow_workspace(&mut self, follow: FollowWorkspace) {
        let gone = follow.gone.clamp(0.0, 1.0);
        let by = (
            (follow.apart.0 as f32 * gone).round() as i32,
            (follow.apart.1 as f32 * gone).round() as i32,
        );
        for (id, rect) in &follow.going {
            self.animations.cancel_window(*id);
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = Some(gevrek_engine::animations::offset_rect(*rect, -by.0, -by.1));
            }
        }
        for (id, rect) in &follow.coming {
            self.animations.cancel_window(*id);
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = Some(gevrek_engine::animations::offset_rect(
                    *rect,
                    follow.apart.0 - by.0,
                    follow.apart.1 - by.1,
                ));
                window.passing = window.hidden_by_wm;
            }
        }
        self.follow_workspace = Some(follow);
        self.needs_redraw = true;
    }

    /// The fingers lifted without going far enough (or the swipe was
    /// cancelled): everything goes back, the workspace beside out of view.
    pub fn follow_workspace_back(&mut self) {
        use gevrek_engine::animation_scheduler::TransitionScope;
        let Some(follow) = self.follow_workspace.take() else {
            return;
        };
        let now = std::time::Instant::now();
        for (id, rect) in &follow.going {
            let start = self.windows.get(id).and_then(|window| window.drawn);
            let animated = start.is_some_and(|start| {
                self.animations
                    .schedule_rect(*id, start, *rect, TransitionScope::Workspace, now)
            });
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = if animated { start } else { None };
            }
        }
        for (id, rect) in &follow.coming {
            let end = gevrek_engine::animations::offset_rect(*rect, follow.apart.0, follow.apart.1);
            let start = self.windows.get(id).and_then(|window| window.drawn);
            let animated = start.is_some_and(|start| {
                self.animations
                    .schedule_rect(*id, start, end, TransitionScope::Workspace, now)
            });
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = if animated { start } else { None };
                window.passing = animated && window.hidden_by_wm;
            }
        }
        self.needs_redraw = true;
    }

    /// The windows on screen, bottom to top (always-on-top ones last).
    pub fn visible_in_order(&self) -> Vec<WindowHandle> {
        self.in_order(false)
    }

    /// The windows that are drawn, bottom to top: those on screen and
    /// those still on their way out.
    pub fn drawn_in_order(&self) -> Vec<WindowHandle> {
        self.in_order(true)
    }

    fn in_order(&self, with_leaving: bool) -> Vec<WindowHandle> {
        let shown = |id: &&WindowHandle| {
            self.shown(**id)
                || (with_leaving
                    && (self.leaving(**id)
                        || self.get(**id).is_some_and(|window| {
                            window.in_overview && window.mapped && !window.minimized
                        })))
        };
        // Tiled windows, then floating ones (a press on a tiled window
        // does not bring it over a floating one that lies above it), then
        // those kept on top, then fullscreen ones (and with a fullscreen
        // window what belongs to it: its dialog is over it, not under it
        // with the other floating windows, where nobody would see what
        // has the keyboard).
        let rank = |id: &WindowHandle| {
            let window = &self.windows[id];
            if window.fullscreen || self.over_fullscreen(*id) {
                3
            } else if window.always_on_top {
                2
            } else if !window.tiled {
                1
            } else {
                0
            }
        };
        let mut order: Vec<WindowHandle> = Vec::new();
        for level in 0..=3 {
            order.extend(
                self.stacking
                    .iter()
                    .filter(shown)
                    .filter(|id| rank(id) == level)
                    .copied(),
            );
        }
        order
    }

    /// The topmost shown window at a point.
    pub fn window_at(&self, x: i32, y: i32) -> Option<WindowHandle> {
        let here = self.output_at(x, y);
        self.visible_in_order().into_iter().rev().find(|id| {
            // (A window on a strip is reached on its strip's display
            // only: what of it lies beyond that display's edge is cut off.)
            if self.homes.get(id).is_some_and(|home| Some(*home) != here) {
                return false;
            }
            // (In an overview: where its picture is.)
            let window = &self.windows[id];
            let rect = window.overview.unwrap_or(window.rect);
            x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
        })
    }

    /// A window comes to the top, and with it the windows that belong to
    /// it: a dialog is not left under the window it asks about.
    fn raise(&mut self, id: WindowHandle) {
        self.stacking = raised(&self.stacking, id, |other| self.parent_of(other));
    }

    /// The window another one belongs to (a dialog's, a tool window's):
    /// the parent of a Wayland toplevel, the window an X11 window says it
    /// is there for.
    pub fn parent_of(&self, id: WindowHandle) -> Option<WindowHandle> {
        let above = match &self.windows.get(&id)?.surface {
            WindowSurface::Xdg(toplevel) => self.id_of_surface(&toplevel.parent()?),
            WindowSurface::X11(window) => {
                let above = window.is_transient_for()?;
                self.windows.iter().find_map(|(other, record)| {
                    matches!(&record.surface, WindowSurface::X11(known)
                        if known.window_id() == above)
                    .then_some(*other)
                })
            }
        };
        above.filter(|above| *above != id)
    }

    /// Whether a window belongs to one that is fullscreen and shown (its
    /// dialog, or a dialog of that): it is drawn and reached over that
    /// window.
    pub fn over_fullscreen(&self, id: WindowHandle) -> bool {
        let mut above = self.parent_of(id);
        for _ in 0..8 {
            let Some(found) = above else {
                return false;
            };
            if self.windows[&found].fullscreen && self.shown(found) {
                return true;
            }
            above = self.parent_of(found);
        }
        false
    }

    fn lower(&mut self, id: WindowHandle) {
        if let Some(index) = self.stacking.iter().position(|other| *other == id) {
            self.stacking.remove(index);
            self.stacking.insert(0, id);
        }
    }

    /// Put a window at `rect`: the position at once, the size by asking the
    /// client (it answers with a commit of that size, or of the nearest size
    /// it accepts).
    fn place(&mut self, id: WindowHandle, rect: Rect) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        // A fullscreen or maximized window stays as it is; the place it is
        // given is where it goes when it leaves that state.
        if (window.fullscreen || window.maximized) && window.restore.is_some() {
            window.restore = Some(rect);
            return;
        }
        // While an overview is up, a place a window is given is a place
        // in the overview: its picture goes there, at its own shape and
        // no larger than it is; the window is not asked for another size.
        if !self.decorations_visible
            && !self.overview_all
            && window.rect.width > 0
            && window.rect.height > 0
        {
            window.overview = Some(fitted(window.rect, rect));
            self.needs_redraw = true;
            return;
        }
        window.rect = rect;
        if rect.width > 0 && rect.height > 0 {
            window.requested = Some((rect.width, rect.height));
            window.surface.request_rect(rect);
        }
        self.needs_redraw = true;
    }

    /// Tell a window's client whether it is tiled, when that changed.
    pub fn set_tiled(&mut self, id: WindowHandle, tiled: bool) {
        if let Some(window) = self.windows.get_mut(&id) {
            if window.tiled != tiled {
                window.tiled = tiled;
                window.surface.set_tiled(tiled);
                window.surface.flush();
            }
        }
    }

    /// Make a window fullscreen or maximized, or take it back. A Wayland
    /// client does not resize itself for these states: the compositor gives
    /// it the output (fullscreen) or the usable area of it (maximized), and
    /// puts it back where it was afterwards.
    fn fill(&mut self, id: WindowHandle, flag: StateFlag, on: bool) {
        let Some(window) = self.windows.get(&id) else {
            return;
        };
        let (fullscreen, maximized) = match flag {
            StateFlag::Fullscreen => (on, window.maximized),
            StateFlag::Maximized => (window.fullscreen, on),
            StateFlag::Activated => return,
        };
        let was = window.fullscreen || window.maximized;
        // The output the middle of the window is on.
        let anchor = window.restore.unwrap_or(window.rect);
        let centre = (anchor.x + anchor.width / 2, anchor.y + anchor.height / 2);
        let output = self
            .output_at(centre.0, centre.1)
            .and_then(|id| self.outputs.iter().find(|output| output.id == id))
            .map(|output| (output.rect, output.usable));
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        window.fullscreen = fullscreen;
        window.maximized = maximized;
        let target = match (fullscreen, maximized, output) {
            (true, _, Some((whole, _))) => Some(whole),
            (false, true, Some((_, usable))) => Some(usable),
            _ => None,
        };
        window.surface.stage_state(flag, on);
        match target {
            Some(rect) => {
                if !was {
                    window.restore = Some(window.rect);
                }
                window.rect = rect;
                window.drawn = None;
                window.requested = Some((rect.width, rect.height));
                window.surface.request_rect(rect);
            }
            None => match window.restore.take() {
                Some(rect) if rect.width > 0 && rect.height > 0 => {
                    window.rect = rect;
                    window.drawn = None;
                    window.requested = Some((rect.width, rect.height));
                    window.surface.request_rect(rect);
                }
                _ => window.surface.flush(),
            },
        }
        self.needs_redraw = true;
    }

    fn output_rect(&self, id: MonitorHandle) -> Option<Rect> {
        self.outputs
            .iter()
            .find(|output| output.id == id)
            .map(|output| output.rect)
    }

    pub(crate) fn output_at(&self, x: i32, y: i32) -> Option<MonitorHandle> {
        self.outputs
            .iter()
            .find(|output| {
                let rect = output.rect;
                x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
            })
            .or(self.outputs.first())
            .map(|output| output.id)
    }

    fn facts(&self, id: WindowHandle) -> Option<gevrek_engine::rules::WindowInfo> {
        let window = self.get(id)?;
        Some(gevrek_engine::rules::WindowInfo {
            handle: id,
            title: window.title.clone(),
            // On Wayland the application id is what the class is on Windows.
            class: window.app_id.clone(),
            exe: window.exe.clone(),
            path: window.path.clone(),
            app_id: window.app_id.clone(),
            // A window that belongs to another (a dialog) is a window
            // of its own that has an owner, as an owned window is on
            // Windows; "has a parent" there means a window inside another,
            // which no toplevel is. (Told as having a parent, every dialog
            // was one of the "child windows" the system filters leave
            // alone: on no workspace, in sight on every one of them.)
            has_parent: false,
            is_visible: window.mapped,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: window.surface.has_parent(),
            is_cloaked: false,
            is_layered: false,
            tags: Default::default(),
            workspace: None,
            workspace_index: None,
        })
    }
}

impl WindowQueries for WlDesktop {
    fn active_window(&self) -> Option<WindowHandle> {
        self.focused
    }

    fn exists(&self, id: WindowHandle) -> bool {
        self.get(id).is_some()
    }

    fn has_title(&self, id: WindowHandle, title: &str) -> Result<bool, String> {
        Ok(self
            .get(id)
            .is_some_and(|window| window.title.contains(title)))
    }

    fn position(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id).map(|window| (window.rect.x, window.rect.y))
    }

    fn size(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id)
            .map(|window| (window.rect.width, window.rect.height))
    }

    fn title(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.title.clone())
            .unwrap_or_default()
    }

    fn class(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.app_id.clone())
            .unwrap_or_default()
    }

    fn exe(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.exe.clone())
            .unwrap_or_default()
    }

    fn icon(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.icon.clone())
            .unwrap_or_default()
    }

    fn path(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.path.clone())
            .unwrap_or_default()
    }

    fn app_id(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.app_id.clone())
            .unwrap_or_default()
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.windows
            .iter()
            .filter(|(_, window)| window.alive && window.mapped)
            .map(|(id, window)| WindowInfo {
                handle: *id,
                title: window.title.clone(),
                x: window.rect.x,
                y: window.rect.y,
                width: window.rect.width,
                height: window.rect.height,
                is_visible: !window.hidden_by_wm && !window.minimized,
                is_minimized: window.minimized,
                is_maximized: window.maximized,
            })
            .collect()
    }

    fn rect(&self, id: WindowHandle) -> Result<Rect, String> {
        // (In an overview the engine is told where the window is shown:
        // that is where it put it.)
        self.get(id)
            .map(|window| window.overview.unwrap_or(window.rect))
            .ok_or_else(|| format!("no window {id:?}"))
    }

    fn is_minimized(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.minimized)
    }

    fn is_maximized(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.maximized)
    }

    fn is_fullscreen(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.fullscreen)
    }

    fn is_visible(&self, id: WindowHandle) -> bool {
        self.shown(id)
    }

    fn is_always_on_top(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.always_on_top)
    }

    fn is_tracked(&self, id: WindowHandle) -> bool {
        self.exists(id)
    }

    fn smallest_size(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id)
            .and_then(|window| window.surface.smallest_size())
    }

    fn pid(&self, id: WindowHandle) -> Option<u32> {
        self.get(id).and_then(|window| window.pid)
    }

    fn initial_title(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.initial_title.clone())
            .unwrap_or_default()
    }

    fn initial_class(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.initial_app_id.clone())
            .unwrap_or_default()
    }

    fn hidden_by_app(&self, id: WindowHandle) -> bool {
        // A client hides a window by detaching its buffer.
        self.get(id).is_some_and(|window| !window.mapped)
    }

    fn is_decoration(&self, _id: WindowHandle) -> bool {
        // Borders are drawn at render time; they are not windows.
        false
    }

    fn has_border(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.border.is_some())
    }

    fn opacity(&self, id: WindowHandle) -> u8 {
        self.get(id).map_or(255, |window| window.opacity)
    }

    fn border_color(&self, _id: WindowHandle) -> RuleColor {
        RuleColor::Default
    }

    fn caption_color(&self, _id: WindowHandle) -> RuleColor {
        RuleColor::Default
    }

    fn backdrop(&self, _id: WindowHandle) -> SystemBackdrop {
        SystemBackdrop::Auto
    }

    fn corners(&self, _id: WindowHandle) -> WindowCorner {
        WindowCorner::Default
    }

    fn shadow_policy(&self, _id: WindowHandle) -> ShadowPolicy {
        ShadowPolicy::Default
    }

    fn rule_facts(&self, id: WindowHandle) -> Option<gevrek_engine::rules::WindowInfo> {
        self.facts(id)
    }

    fn parent(&self, id: WindowHandle) -> Option<WindowHandle> {
        self.parent_of(id)
    }

    fn placed_itself(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.placed_itself)
    }

    fn ignored_by_rules(&self, id: WindowHandle) -> bool {
        let Some(facts) = self.facts(id) else {
            return true;
        };
        self.rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .should_ignore(&facts)
    }

    fn floats_by_rules(&self, id: WindowHandle) -> bool {
        let Some(facts) = self.facts(id) else {
            return false;
        };
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A rule of the person's own that says float or tile is the last
        // word; without one a dialog floats, whatever program shows it.
        if let Some(floating) = rules.resolve_static(&facts).floating {
            return floating;
        }
        self.get(id)
            .is_some_and(|window| window.surface.is_dialog())
            || rules.should_float(&facts)
    }
}

impl Desktop for WlDesktop {
    fn apply(&mut self, effect: &Effect) -> anyhow::Result<()> {
        match effect {
            Effect::Configure { id, rect, states } => {
                if let Some(window) = self.windows.get(id) {
                    window.surface.set_tiled(states.tiled);
                }
                self.place(*id, *rect)
            }
            Effect::SetVisible { id, visible } => {
                if let Some(window) = self.windows.get_mut(id) {
                    if !*visible && !window.hidden_by_wm {
                        window.hidden_at = Some(std::time::Instant::now());
                    }
                    window.hidden_by_wm = !*visible;
                    window.passing = false;
                    self.needs_redraw = true;
                }
            }
            Effect::Focus(target) => {
                let target = target.filter(|id| self.shown(*id));
                if self.focused != target {
                    self.focused = target;
                    self.focus_dirty = true;
                    self.needs_redraw = true;
                    if let Some(id) = target {
                        self.raise(id);
                        self.pending_events.push(WindowEvent::Focused(id));
                    }
                }
            }
            Effect::Stack { id, op } => {
                match op {
                    StackOp::Raise => self.raise(*id),
                    StackOp::Lower => self.lower(*id),
                    StackOp::AlwaysOnTop(value) => {
                        if let Some(window) = self.windows.get_mut(id) {
                            window.always_on_top = *value;
                        }
                    }
                }
                self.needs_redraw = true;
            }
            Effect::Close(id) => {
                // The window leaves the way the configuration says, and is
                // asked to close when that transition is over.
                if !self.start_exit(*id) {
                    if let Some(window) = self.get(*id) {
                        window.surface.close();
                    }
                }
            }
            Effect::Opacity { id, alpha } => {
                if let Some(window) = self.windows.get_mut(id) {
                    window.opacity = *alpha;
                    self.needs_redraw = true;
                }
            }
            Effect::State { id, change } => {
                match *change {
                    StateChange::Maximized(value) => self.fill(*id, StateFlag::Maximized, value),
                    StateChange::Fullscreen(value) => self.fill(*id, StateFlag::Fullscreen, value),
                    StateChange::Minimized(value) => {
                        // Wayland has no minimized state for the client: the
                        // window is simply not shown.
                        if let Some(window) = self.windows.get_mut(id) {
                            if window.minimized != value {
                                window.minimized = value;
                                self.pending_events.push(if value {
                                    WindowEvent::Minimized(*id)
                                } else {
                                    WindowEvent::Restored(*id)
                                });
                            }
                        }
                    }
                }
                self.needs_redraw = true;
            }
            Effect::Border { id, change } => {
                if let Some(window) = self.windows.get_mut(id) {
                    match change {
                        BorderChange::Visible(value) => {
                            if let Some(border) = window.border.as_mut() {
                                border.visible = *value;
                            }
                        }
                        BorderChange::Remove => window.border = None,
                        // Borders are drawn around the window every frame.
                        BorderChange::Follow => {}
                        BorderChange::Focused(value) => {
                            if let Some(border) = window.border.as_mut() {
                                if border.focused != *value {
                                    // From the look it has now to the other one.
                                    let look = |focused: bool| {
                                        if focused {
                                            (border.active_color, border.active_alpha)
                                        } else {
                                            (border.inactive_color, border.inactive_alpha)
                                        }
                                    };
                                    let from = window.drawn_border.unwrap_or(look(border.focused));
                                    let to = look(*value);
                                    let animated = self.animations.schedule_border(
                                        *id,
                                        from.0,
                                        to.0,
                                        from.1,
                                        to.1,
                                        std::time::Instant::now(),
                                    );
                                    window.drawn_border = animated.then_some(from);
                                }
                                border.focused = *value;
                            }
                        }
                    }
                    window.border_commit.increment();
                    self.needs_redraw = true;
                }
            }
            Effect::DecorationsVisible(value) => {
                self.decorations_visible = *value;
                // The overview is over: every window is shown where it
                // is again.
                if *value {
                    for window in self.windows.values_mut() {
                        if window.overview.take().is_some() {
                            window.drawn = None;
                        }
                    }
                }
                self.needs_redraw = true;
            }
            Effect::WarpPointer(point) => {
                self.pointer = (point.x, point.y);
                // The window under the new place is told where the
                // pointer is (`State::settle`).
                self.pointer_warped = true;
                self.needs_redraw = true;
            }
            Effect::HideStackbar(key) => {
                self.stackbars.retain(|bar| bar.key != *key);
                self.needs_redraw = true;
            }
            // The DWM effects have no meaning here; the others are not sent
            // by the engine today.
            Effect::Stackbar { .. }
            | Effect::Decorate { .. }
            | Effect::Animate { .. }
            | Effect::Launch(_)
            | Effect::Platform(_) => {}
        }
        Ok(())
    }

    fn output_of(&self, id: WindowHandle) -> Option<MonitorHandle> {
        let rect = self.get(id)?.rect;
        self.output_at(rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    fn output_at_cursor(&self) -> Option<MonitorHandle> {
        self.output_at(self.pointer.0, self.pointer.1)
    }

    fn primary_button_down(&self) -> bool {
        self.primary_button_down
    }

    fn apply_animated_rectangles(
        &mut self,
        operations: &[(WindowHandle, Rect)],
        scope: gevrek_engine::animation_scheduler::TransitionScope,
    ) -> anyhow::Result<()> {
        let now = std::time::Instant::now();
        for (id, rect) in operations {
            // (What fingers on a touchpad are pulling stays where they
            // have it: it is given its place, and goes there when the
            // pull ends.)
            if self.follow_workspace.as_ref().is_some_and(|follow| {
                follow
                    .going
                    .iter()
                    .chain(&follow.coming)
                    .any(|(pulled, _)| pulled == id)
            }) {
                self.place(*id, *rect);
                continue;
            }
            // From where the window is drawn now to where it belongs.
            let start = self.drawn_rect(*id);
            self.place(*id, *rect);
            // (In an overview the window's picture goes to its place
            // there, which is not the whole of the place it was given.)
            let end = self
                .windows
                .get(id)
                .and_then(|window| window.overview)
                .unwrap_or(*rect);
            // (Following fingers: from where it belongs to where it
            // belongs, which ends a transition in flight.)
            let start = if self.follow { Some(end) } else { start };
            let animated = start
                .is_some_and(|start| self.animations.schedule_rect(*id, start, end, scope, now));
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = if animated { start } else { None };
                window.passing = false;
            }
        }
        Ok(())
    }

    /// One workspace gives way to another: the windows hidden a moment
    /// ago go out as the windows of the new one (`operations`) come in,
    /// the way `desktop.animations.workspace_style` says. Where fingers on
    /// a touchpad have pulled the two part of the way already, they go on
    /// from where they are.
    fn begin_workspace_transition(
        &mut self,
        operations: &[(WindowHandle, Rect)],
        offset_x: i32,
        offset_y: i32,
    ) -> anyhow::Result<bool> {
        use gevrek_engine::animation_scheduler::TransitionScope;
        use gevrek_model::settings::WorkspaceTransitionStyle as Style;
        // (What fingers pulled goes on the way they pulled it.)
        let followed = self.follow_workspace.take();
        let (offset_x, offset_y) = followed
            .as_ref()
            .map_or((offset_x, offset_y), |follow| follow.apart);
        if self.plain_workspace_change || !self.animations.moves(TransitionScope::Workspace) {
            self.end_passing();
            return Ok(false);
        }
        let now = std::time::Instant::now();
        let style = self.animations.profile().workspace_style;
        let going: Vec<WindowHandle> = self
            .windows
            .iter()
            .filter(|(id, window)| {
                window.alive
                    && window.mapped
                    && window.hidden_by_wm
                    && !window.minimized
                    && !window.fullscreen
                    && window
                        .hidden_at
                        .is_some_and(|at| now.duration_since(at) < JUST_HIDDEN)
                    && !operations.iter().any(|(coming, _)| coming == *id)
            })
            .map(|(id, _)| *id)
            .collect();
        // Up and down: as far as the display is high, the way the
        // sideways offset points.
        let (offset_x, offset_y) = match style {
            Style::SlideVertical if offset_y == 0 && followed.is_none() => {
                let anchor = operations.first().map(|(_, rect)| *rect).or_else(|| {
                    going
                        .first()
                        .and_then(|id| self.get(*id))
                        .map(|window| window.rect)
                });
                let height = anchor
                    .and_then(|rect| {
                        self.output_at(rect.x + rect.width / 2, rect.y + rect.height / 2)
                    })
                    .and_then(|output| self.output_rectangle(output))
                    .map_or(offset_x.abs(), |screen| screen.height);
                (0, offset_x.signum() * height)
            }
            _ => (offset_x, offset_y),
        };
        // (A pull by fingers is a slide, whatever the style: it goes on
        // as one.)
        let fade = style == Style::Fade && followed.is_none();
        let mut any = false;
        for id in &going {
            let Some(window) = self.windows.get(id) else {
                continue;
            };
            let (rect, start, opacity) = (
                window.rect,
                window.drawn.unwrap_or(window.rect),
                window.opacity,
            );
            let animated = if fade {
                self.animations
                    .schedule_workspace_opacity(*id, opacity, 0, now)
            } else {
                let end = gevrek_engine::animations::offset_rect(rect, -offset_x, -offset_y);
                self.animations
                    .schedule_rect(*id, start, end, TransitionScope::Workspace, now)
            };
            if let Some(window) = self.windows.get_mut(id) {
                if fade {
                    window.drawn = None;
                    window.drawn_opacity = animated.then_some(opacity);
                } else {
                    window.drawn = animated.then_some(start);
                }
                window.passing = animated;
            }
            any |= animated;
        }
        for (id, target) in operations {
            // (Pulled in part of the way already: from where it is.)
            let pulled = self
                .windows
                .get(id)
                .filter(|window| window.passing)
                .and_then(|window| window.drawn);
            // (One that is where it comes to already is not put there
            // again: a floating window that fills the display, or is
            // maximized, comes to the place it has, and put there it
            // would take that for the place to go back to afterwards.)
            if self.get(*id).map(|window| window.rect) != Some(*target) {
                self.place(*id, *target);
            }
            if fade {
                let Some(opacity) = self.get(*id).map(|window| window.opacity) else {
                    continue;
                };
                let animated = self
                    .animations
                    .schedule_workspace_opacity(*id, 0, opacity, now);
                if let Some(window) = self.windows.get_mut(id) {
                    window.drawn = None;
                    window.drawn_opacity = animated.then_some(0);
                }
                any |= animated;
                continue;
            }
            let start = pulled.unwrap_or_else(|| {
                gevrek_engine::animations::offset_rect(*target, offset_x, offset_y)
            });
            let animated =
                self.animations
                    .schedule_rect(*id, start, *target, TransitionScope::Workspace, now);
            if let Some(window) = self.windows.get_mut(id) {
                window.drawn = animated.then_some(start);
            }
            any |= animated;
        }
        // Whatever else was pulled along and is neither coming nor going
        // now (the fingers went one way, the command another) is let go.
        for (id, window) in self.windows.iter_mut() {
            if window.passing
                && !going.contains(id)
                && !operations.iter().any(|(coming, _)| coming == id)
            {
                window.passing = false;
                window.drawn = None;
            }
        }
        self.needs_redraw = true;
        Ok(any || !operations.is_empty())
    }

    /// A new window appears the way the configuration says.
    fn start_configured_window_enter(&mut self, id: WindowHandle) -> anyhow::Result<bool> {
        use gevrek_engine::animation_scheduler::TransitionScope;
        use gevrek_model::settings::WindowTransitionStyle;
        if !self.animations.scope_enabled(TransitionScope::Explicit) {
            return Ok(false);
        }
        let Some(target) = self.get(id).map(|window| window.rect) else {
            return Ok(false);
        };
        let now = std::time::Instant::now();
        let start = match self.animations.profile().window_enter_style {
            WindowTransitionStyle::None => return Ok(false),
            WindowTransitionStyle::Fade => return Ok(self.fade_in(id, 0, now)),
            WindowTransitionStyle::Slide => {
                gevrek_engine::animations::offset_rect(target, -target.width / 3, 0)
            }
            WindowTransitionStyle::Scale => {
                gevrek_engine::animations::centered_scaled_rect(target, 20)
            }
        };
        let animated = self
            .animations
            .schedule_explicit_rect(id, start, target, 0, now);
        if let Some(window) = self.windows.get_mut(&id) {
            window.drawn = animated.then_some(start);
        }
        Ok(animated)
    }

    fn start_window_animation(
        &mut self,
        id: WindowHandle,
        animation_type: gevrek_engine::animations::AnimationType,
        duration_ms: u32,
    ) -> anyhow::Result<bool> {
        use gevrek_engine::animation_scheduler::TransitionScope;
        if !self.animations.scope_enabled(TransitionScope::Explicit) {
            return Ok(false);
        }
        let Some(target) = self.get(id).map(|window| window.rect) else {
            return Ok(false);
        };
        let now = std::time::Instant::now();
        let Some(start) = gevrek_engine::animations::start_rect(animation_type, target) else {
            // Fade changes the opacity, not the place.
            return Ok(self.fade_in(id, duration_ms, now));
        };
        let animated = self
            .animations
            .schedule_explicit_rect(id, start, target, duration_ms, now);
        if let Some(window) = self.windows.get_mut(&id) {
            window.drawn = animated.then_some(start);
        }
        Ok(animated)
    }

    fn is_animation_event_suppressed(&self, _id: WindowHandle) -> bool {
        false
    }

    fn preferred_window_size(&self, _id: WindowHandle) -> Option<(i32, i32)> {
        None
    }

    fn decorate_window(
        &mut self,
        id: WindowHandle,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> anyhow::Result<()> {
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
        if let Some(window) = self.windows.get_mut(&id) {
            window.border = Some(Border {
                width,
                offset,
                active_color,
                inactive_color,
                active_alpha,
                inactive_alpha,
                focused: is_focused,
                visible: true,
                rounded: style == gevrek_model::graph::BorderStyle::Rounded,
                style,
            });
            window.border_commit.increment();
            self.needs_redraw = true;
        }
        Ok(())
    }

    fn set_window_glow(
        &mut self,
        id: WindowHandle,
        glow: Option<gevrek_engine::contract::GlowStrength>,
    ) {
        if let Some(window) = self.windows.get_mut(&id) {
            if window.glow != glow {
                window.glow = glow;
                window.border_commit.increment();
                self.needs_redraw = true;
            }
        }
    }

    fn decorate_window_at(
        &mut self,
        id: WindowHandle,
        _at: Rect,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> anyhow::Result<()> {
        // The border follows the window's rect at render time, so the
        // explicit target rect is not needed.
        self.decorate_window(id, look, is_focused)
    }

    fn update_border_position_explicit(
        &mut self,
        _id: WindowHandle,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn toggle_title_bar(&mut self, _id: WindowHandle) -> anyhow::Result<bool> {
        // Clients are asked for server-side decorations and gevrek draws no
        // title bar, so there is nothing to toggle.
        Ok(false)
    }

    /// The tab strips that should exist now. A strip is painted again only
    /// when what it shows changed.
    fn sync_stackbars(
        &mut self,
        desired: &[StackbarContainerState],
        config: &gevrek_model::settings::StackbarProfile,
    ) -> anyhow::Result<()> {
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
        use smithay::utils::Transform;

        let mut old = std::mem::take(&mut self.stackbars);
        if !config.enabled {
            self.needs_redraw = true;
            return Ok(());
        }
        // Painted for the sharpest output there is.
        let scale = self
            .outputs
            .iter()
            .map(|output| output.scale)
            .fold(1.0, f64::max)
            .ceil() as i32;
        for state in desired {
            // The tab of the window that is shown is marked in the colour
            // of that window's border, where it has one: the strip is of
            // one piece with the frame around it.
            let mut look = config.clone();
            if let Some(border) = self
                .windows
                .get(&state.key)
                .and_then(|window| window.border)
            {
                if border.visible {
                    look.active_text_color = border.active_color;
                }
            }
            let config = &look;
            let colors = (
                config.background_color,
                config.active_text_color,
                config.inactive_text_color,
            );
            let (x, y, width, _) = state.rect;
            if state.tabs.len() < 2 || width <= 0 || config.height <= 0 {
                continue;
            }
            let rect = Rect::new(x, y, width, config.height);
            let titles: Vec<String> = state
                .tabs
                .iter()
                .map(|tab| {
                    let title = self.title(*tab);
                    if title.trim().is_empty() {
                        "Untitled".to_string()
                    } else {
                        title
                    }
                })
                .collect();
            let focused_tab = state.focused_tab.min(state.tabs.len() - 1);
            let kept = old
                .iter()
                .position(|bar| {
                    bar.key == state.key
                        && (bar.rect.width, bar.rect.height) == (rect.width, rect.height)
                        && bar.titles == titles
                        && bar.focused_tab == focused_tab
                        && bar.colors == colors
                        && bar.scale == scale
                })
                .map(|index| old.swap_remove(index));
            let buffer = match kept {
                Some(bar) => bar.buffer,
                None => MemoryRenderBuffer::from_slice(
                    &crate::stackbar::paint(
                        rect.width,
                        rect.height,
                        &titles,
                        focused_tab,
                        config,
                        scale,
                    ),
                    Fourcc::Argb8888,
                    (rect.width * scale, rect.height * scale),
                    scale,
                    Transform::Normal,
                    None,
                ),
            };
            self.stackbars.push(Stackbar {
                key: state.key,
                rect,
                tabs: state.tabs.clone(),
                titles,
                focused_tab,
                buffer,
                colors,
                scale,
            });
        }
        self.needs_redraw = true;
        Ok(())
    }

    fn launch(&mut self, launch: &gevrek_model::settings::LaunchProfile) -> anyhow::Result<u32> {
        crate::launch::spawn(launch)
    }

    fn is_hidden_by_backend(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.hidden_by_wm)
    }

    fn window_for_native_handle(&self, raw: isize) -> WindowHandle {
        WindowHandle(raw)
    }

    fn untrack_window(&mut self, id: WindowHandle) {
        self.animations.cancel_window(id);
        self.windows.remove(&id);
        self.stacking.retain(|other| *other != id);
        self.had_focus.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
            self.focus_dirty = true;
        }
    }

    fn restore_journaled_windows(&mut self) -> usize {
        // A compositor that stops takes its clients' windows with it; there
        // is nothing hidden to give back.
        0
    }
}

impl WlDesktop {
    /// The tab strip at a point, and which of its tabs.
    pub fn stackbar_tab_at(&self, x: i32, y: i32) -> Option<(WindowHandle, usize)> {
        if !self.decorations_visible {
            return None;
        }
        self.stackbars
            .iter()
            .filter(|bar| self.shown(bar.key))
            .find(|bar| {
                x >= bar.rect.x
                    && x < bar.rect.x + bar.rect.width
                    && y >= bar.rect.y
                    && y < bar.rect.y + bar.rect.height
            })
            .and_then(|bar| {
                crate::stackbar::tab_at(bar.rect.width, bar.tabs.len(), x - bar.rect.x)
                    .map(|tab| (bar.key, tab))
            })
    }

    /// Put a window where the pointer drags it: at once, no transition.
    pub fn place_now(&mut self, id: WindowHandle, rect: Rect) {
        self.animations.cancel_window(id);
        if let Some(window) = self.windows.get_mut(&id) {
            window.drawn = None;
        }
        self.place(id, rect);
    }

    /// Where a window is drawn now: mid-transition, or where it belongs.
    pub fn drawn_rect(&self, id: WindowHandle) -> Option<Rect> {
        self.get(id)
            .map(|window| window.drawn.or(window.overview).unwrap_or(window.rect))
    }

    /// Start the exit transition of a window. Returns false when there is
    /// none (the caller closes the window at once).
    fn start_exit(&mut self, id: WindowHandle) -> bool {
        use gevrek_engine::animation_scheduler::TransitionScope;
        use gevrek_model::settings::WindowTransitionStyle;
        if !self.animations.scope_enabled(TransitionScope::Explicit)
            || self.animations.is_close_pending(id)
        {
            return self.animations.is_close_pending(id);
        }
        let Some((rect, opacity)) = self
            .get(id)
            .filter(|window| window.mapped && !window.hidden_by_wm)
            .map(|window| (window.drawn.unwrap_or(window.rect), window.opacity))
        else {
            return false;
        };
        let now = std::time::Instant::now();
        match self.animations.profile().window_exit_style {
            WindowTransitionStyle::None => false,
            WindowTransitionStyle::Fade => {
                let animated = self.animations.schedule_exit_opacity(id, opacity, now);
                if let Some(window) = self.windows.get_mut(&id) {
                    window.drawn_opacity = animated.then_some(opacity);
                }
                animated
            }
            style => {
                let target = if style == WindowTransitionStyle::Slide {
                    gevrek_engine::animations::offset_rect(rect, -rect.width / 3, 0)
                } else {
                    gevrek_engine::animations::centered_scaled_rect(rect, 20)
                };
                let animated = self.animations.schedule_exit_rect(id, rect, target, now);
                if let Some(window) = self.windows.get_mut(&id) {
                    window.drawn = animated.then_some(rect);
                }
                animated
            }
        }
    }

    fn fade_in(&mut self, id: WindowHandle, duration_ms: u32, now: std::time::Instant) -> bool {
        let Some(target) = self.get(id).map(|window| window.opacity) else {
            return false;
        };
        let animated = self
            .animations
            .schedule_opacity(id, 0, target, duration_ms, now);
        if let Some(window) = self.windows.get_mut(&id) {
            window.drawn_opacity = animated.then_some(0);
        }
        animated
    }

    /// Move every transition on to `now`. Returns whether any is still
    /// running (a new frame is needed).
    pub fn tick_animations(&mut self, now: std::time::Instant) -> bool {
        if self.animations.active_count() == 0 {
            return false;
        }
        let frame = self.animations.tick(now);
        for (id, rect) in frame.rectangles {
            let finished = self.animations.rect_target(id).is_none();
            if let Some(window) = self.windows.get_mut(&id) {
                window.drawn = (!finished).then_some(rect);
                if finished && window.drawn_opacity.is_none() {
                    window.passing = false;
                }
            }
        }
        for (id, alpha) in frame.opacities {
            let running = self.animations.opacity_running(id);
            if let Some(window) = self.windows.get_mut(&id) {
                window.drawn_opacity = (alpha != window.opacity).then_some(alpha);
                // (A window of a workspace that faded away is gone when
                // it has faded.)
                if window.passing && !running {
                    window.passing = false;
                    window.drawn_opacity = None;
                }
            }
        }
        for border in frame.borders {
            if let Some(window) = self.windows.get_mut(&border.handle) {
                let target = window.border.map(|look| {
                    if look.focused {
                        (look.active_color, look.active_alpha)
                    } else {
                        (look.inactive_color, look.inactive_alpha)
                    }
                });
                let now = (border.color, border.alpha);
                window.drawn_border = (Some(now) != target).then_some(now);
                window.border_commit.increment();
            }
        }
        // An exit transition is over: now the client is asked to close. If
        // it stays (it may ask the user first), it is shown again.
        for id in frame.close_windows {
            if let Some(window) = self.windows.get_mut(&id) {
                window.drawn = None;
                window.drawn_opacity = None;
                // (Gone from the picture from here on: shown whole again
                // for the frames its program takes to go, it looked as
                // if the window came back.)
                window.closing = Some(std::time::Instant::now());
                window.surface.close();
            }
        }
        // A transition whose window is gone leaves nothing behind.
        for window in self.windows.values_mut() {
            if !window.alive {
                window.drawn = None;
                window.drawn_opacity = None;
            }
        }
        self.needs_redraw = true;
        self.animations.active_count() > 0
    }

    /// The rectangle of an output, for the compositor's own use.
    pub fn output_rectangle(&self, id: MonitorHandle) -> Option<Rect> {
        self.output_rect(id)
    }
}

#[cfg(test)]
mod turn_tests {
    use super::*;

    #[test]
    fn a_windows_picture_fits_its_place_in_the_overview_at_its_own_shape() {
        // A wide window in a square place: as wide as the place, in its middle.
        assert_eq!(
            fitted(Rect::new(0, 0, 1600, 800), Rect::new(100, 100, 400, 400)),
            Rect::new(100, 200, 400, 200)
        );
        // A tall one: as high as the place.
        assert_eq!(
            fitted(Rect::new(0, 0, 400, 800), Rect::new(0, 0, 400, 400)),
            Rect::new(100, 0, 200, 400)
        );
        // A window smaller than its place is not made larger.
        assert_eq!(
            fitted(Rect::new(0, 0, 200, 100), Rect::new(0, 0, 600, 500)),
            Rect::new(200, 200, 200, 100)
        );
        // Nothing to fit: the place as it is.
        assert_eq!(
            fitted(Rect::new(0, 0, 0, 0), Rect::new(5, 6, 70, 80)),
            Rect::new(5, 6, 70, 80)
        );
    }
    use gevrek_model::outputs::OutputTransform as T;

    #[test]
    fn a_raised_window_takes_its_dialogs_up_with_it() {
        let w = WindowHandle;
        // 3 is a dialog of 1, 5 a dialog of that dialog; 2 and 4 are
        // other windows.
        let parent_of = |window: WindowHandle| match window.0 {
            3 => Some(w(1)),
            5 => Some(w(3)),
            _ => None,
        };
        let order = [w(1), w(3), w(2), w(5), w(4)];
        // The window is pressed: it comes up, its dialogs stay above it.
        assert_eq!(
            raised(&order, w(1), parent_of),
            [w(2), w(4), w(1), w(3), w(5)]
        );
        // A dialog comes up with its own, and not the window under it.
        assert_eq!(
            raised(&order, w(3), parent_of),
            [w(1), w(2), w(4), w(3), w(5)]
        );
        // Another window: only itself.
        assert_eq!(
            raised(&order, w(2), parent_of),
            [w(1), w(3), w(5), w(4), w(2)]
        );
        // One that is not there changes nothing.
        assert_eq!(raised(&order, w(9), parent_of), order);
        // Two windows that name each other do not hang it.
        let each_other = |window: WindowHandle| match window.0 {
            1 => Some(w(2)),
            2 => Some(w(1)),
            _ => None,
        };
        assert_eq!(
            raised(&[w(1), w(2), w(3)], w(1), each_other),
            [w(3), w(1), w(2)]
        );
    }

    #[test]
    fn a_display_on_its_side_is_as_wide_as_it_is_high() {
        assert_eq!(turned_size(T::Normal, (2560, 1440)), (2560, 1440));
        assert_eq!(turned_size(T::UpsideDown, (2560, 1440)), (2560, 1440));
        assert_eq!(turned_size(T::Right, (2560, 1440)), (1440, 2560));
        assert_eq!(turned_size(T::Left, (2560, 1440)), (1440, 2560));
    }

    #[test]
    fn a_place_on_the_display_is_found_in_the_turned_picture() {
        let near = |(x, y): (f64, f64), (wx, wy): (f64, f64)| {
            (x - wx).abs() < 1e-9 && (y - wy).abs() < 1e-9
        };
        // The device's own top-left corner.
        assert!(near(turned_place(T::Normal, (0.0, 0.0)), (0.0, 0.0)));
        // Turned a quarter to the right, the picture's top-left corner is
        // at the display's own bottom-left: its top-left is the picture's
        // top-right.
        assert!(near(turned_place(T::Right, (0.0, 0.0)), (1.0, 0.0)));
        assert!(near(turned_place(T::Right, (0.0, 1.0)), (0.0, 0.0)));
        assert!(near(turned_place(T::UpsideDown, (0.25, 1.0)), (0.75, 0.0)));
        assert!(near(turned_place(T::Left, (1.0, 0.0)), (0.0, 0.0)));
        // Four quarter turns are none.
        for place in [(0.1, 0.3), (0.9, 0.2)] {
            let mut turned = place;
            for _ in 0..4 {
                turned = turned_place(T::Right, turned);
            }
            assert!(near(turned, place));
        }
    }
}
