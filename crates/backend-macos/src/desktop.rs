//! The desktop as the engine works with it: the windows of the programs
//! that run, through the Accessibility API.
//!
//! Each window gevrek knows is a record here: its element (what it is
//! moved by), its program, its title, where it stands, its states. Queries
//! are answered from the records, which the runtime keeps up to date from
//! the observers' events; effects are carried out on the elements.
//!
//! What macOS does not let a program do to another's windows is left
//! undone, and the capabilities say so (`backend_info`): their opacity,
//! their corners and shadows, keeping them above the others. A window on a
//! workspace that is not shown is parked in a corner of its display
//! (`park`), and written down in the journal until it is back.
//!
//! A window's id is the window server's (`CGWindowID`), which names one
//! window until it closes.

use crate::apps::App;
use crate::ax::Element;
use crate::displays::Display;
use crate::ffi;
use crate::journal::{Journal, Parked};
use gevrek_engine::contract::{
    BorderChange, Desktop, Effect, StackOp, StackbarContainerState, StateChange, WindowQueries,
};
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::geometry::Point;
use gevrek_model::ids::OutputId;
use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use gevrek_model::settings::{RuleColor, ShadowPolicy};
use gevrek_model::window::WindowInfo;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the moves a window reports after gevrek placed it are taken
/// for gevrek's own. (A program answers a new frame with several moves and
/// resizes, some of them late.)
const OWN_MOVE: Duration = Duration::from_millis(400);

/// The kinds of window gevrek manages. Others (a popover, a tooltip, the
/// unknown) are left alone.
const MANAGED_SUBROLES: &[&str] = &[
    "AXStandardWindow",
    "AXDialog",
    "AXSystemDialog",
    "AXFloatingWindow",
];

/// One window gevrek knows.
#[derive(Debug, Clone)]
pub struct MacWindow {
    pub element: Element,
    pub app: App,
    pub title: String,
    pub initial_title: String,
    /// `AXStandardWindow`, `AXDialog`, ...
    pub subrole: String,
    /// Where it stands, as last read or placed.
    pub rect: Rect,
    pub minimized: bool,
    /// gevrek made it fill its display (not macOS's full screen, which
    /// gives a window a Space of its own, out of gevrek's reach).
    pub fullscreen: bool,
    pub maximized: bool,
    /// Its size can be changed.
    pub resizable: bool,
    /// gevrek parked it for a workspace that is not shown.
    pub parked: bool,
    /// Its program is hidden (Command-H).
    pub app_hidden: bool,
    /// It is in macOS's own full screen: on a Space of its own, out of
    /// gevrek's reach until it comes back.
    pub own_space: bool,
    /// The smallest size its program let it have, as far as gevrek has
    /// seen (a size it was given and would not take): 0 where unknown.
    /// macOS has no way to ask.
    pub smallest: (i32, i32),
}

impl MacWindow {
    /// It is away from the desktop gevrek arranges, though it exists: its
    /// program is hidden, or it is in macOS's own full screen. It leaves
    /// the layout until it is back, and its border goes with it.
    pub fn away(&self) -> bool {
        self.app_hidden || self.own_space
    }

    pub fn managed_kind(&self) -> bool {
        MANAGED_SUBROLES.contains(&self.subrole.as_str())
    }

    /// A window of one size, a dialog, a panel: floats unless a rule says
    /// otherwise.
    pub fn floats_by_kind(&self) -> bool {
        self.subrole != "AXStandardWindow" || !self.resizable
    }
}

/// The macOS desktop.
pub struct MacDesktop {
    pub windows: BTreeMap<WindowHandle, MacWindow>,
    pub displays: Vec<Display>,
    pub focused: Option<WindowHandle>,
    rules: Arc<Mutex<RuleEngine>>,
    journal: Journal,
    /// When gevrek last placed each window (its moves are not the user's).
    placed_at: HashMap<WindowHandle, Instant>,
    /// What only some systems do, said once each in the log.
    told: std::collections::HashSet<&'static str>,
    /// Windows that turned out to have a smallest size since the last look
    /// (`take_learned`): the layout gives them room again.
    learned: Vec<WindowHandle>,
    /// The borders, where windows of gevrek's own can be made (the main
    /// thread, with AppKit started); `None`: no borders are drawn.
    pub borders: Option<crate::borders::Borders>,
    /// The stackbars, made where the borders are.
    pub stackbars: Option<crate::stackbars::Stackbars>,
}

/// The id of the window with this window server id.
pub fn handle_of(window: ffi::CGWindowID) -> WindowHandle {
    WindowHandle(window as isize)
}

fn window_id_of(handle: WindowHandle) -> ffi::CGWindowID {
    handle.0 as ffi::CGWindowID
}

impl MacDesktop {
    pub fn new(rules: Arc<Mutex<RuleEngine>>, journal: Journal) -> Self {
        Self {
            windows: BTreeMap::new(),
            displays: Vec::new(),
            focused: None,
            rules,
            journal,
            placed_at: HashMap::new(),
            told: Default::default(),
            learned: Vec::new(),
            borders: None,
            stackbars: None,
        }
    }

    /// Put a window where the mouse drags it: moved (its place only, which
    /// is quick), or resized. Its border follows; what it reports of its
    /// place in answer is gevrek's own. Returns where it stands.
    pub fn drag_to(&mut self, id: WindowHandle, rect: Rect, resize: bool) -> Option<Rect> {
        let window = self.windows.get_mut(&id)?;
        self.placed_at.insert(id, Instant::now());
        let landed = if resize {
            window.element.set_frame(rect).ok()?
        } else {
            window.element.set_position(rect.x, rect.y).ok()?;
            Rect::new(rect.x, rect.y, window.rect.width, window.rect.height)
        };
        window.rect = landed;
        if let Some(borders) = &mut self.borders {
            borders.follow(id, landed);
        }
        Some(landed)
    }

    /// The windows gevrek manages and shows now, by window server id.
    pub fn shown_window_ids(&self) -> std::collections::HashSet<u32> {
        self.windows
            .iter()
            .filter(|(_, window)| !window.parked && !window.minimized && !window.away())
            .map(|(id, _)| window_id_of(*id))
            .collect()
    }

    /// The window came to the front: what gevrek draws around and over it
    /// is put above it again. (A program raises its window a moment after
    /// it is asked, after gevrek ordered its border: this is called when
    /// macOS says the focus changed.)
    pub fn raise_overlays(&mut self, id: WindowHandle) {
        if let Some(borders) = &self.borders {
            borders.raise(id);
        }
        if let Some(stackbars) = &self.stackbars {
            stackbars.raise(id);
        }
    }

    /// The stackbar tab a click hit: the window number of the window
    /// clicked, and the click's distance from its left edge.
    pub fn stackbar_tab_at(&self, window_number: isize, x: f64) -> Option<(WindowHandle, usize)> {
        self.stackbars.as_ref()?.tab_at(window_number, x)
    }

    /// The border of a window goes where the window is now (after a move
    /// gevrek did not make: it trails a dragged window a little).
    pub fn follow_border(&mut self, id: WindowHandle) {
        let Some(rect) = self.get(id).filter(|window| !window.parked).map(|window| window.rect) else {
            return;
        };
        if let Some(borders) = &mut self.borders {
            borders.follow(id, rect);
        }
    }

    /// The windows found to be bigger than they were asked to be since the
    /// last call: their outputs are laid out again, around their smallest
    /// size.
    pub fn take_learned(&mut self) -> Vec<WindowHandle> {
        std::mem::take(&mut self.learned)
    }

    pub fn get(&self, id: WindowHandle) -> Option<&MacWindow> {
        self.windows.get(&id)
    }

    /// Start knowing a window: read it once. `None` for an element that is
    /// no window gevrek manages (a popover, an element that is gone) or one
    /// known already.
    pub fn track(&mut self, element: Element, app: &App) -> Option<WindowHandle> {
        let id = handle_of(element.window_id()?);
        if self.windows.contains_key(&id) {
            return None;
        }
        let subrole = element.subrole();
        if element.role() != "AXWindow" || !MANAGED_SUBROLES.contains(&subrole.as_str()) {
            return None;
        }
        let rect = element.frame().ok()?;
        let title = element.title();
        let parked = self.journal.get(window_id_of(id)).is_some();
        let window = MacWindow {
            resizable: element.is_resizable(),
            minimized: element.is_minimized(),
            fullscreen: false,
            maximized: false,
            initial_title: title.clone(),
            title,
            subrole,
            rect,
            parked,
            app_hidden: false,
            own_space: false,
            smallest: (0, 0),
            app: app.clone(),
            element,
        };
        self.windows.insert(id, window);
        Some(id)
    }

    /// Read again what may have changed of a window: where it is, its
    /// title, whether it is minimized.
    pub fn refresh(&mut self, id: WindowHandle) {
        if let Some(window) = self.windows.get_mut(&id) {
            if let Ok(rect) = window.element.frame() {
                window.rect = rect;
            }
            window.title = window.element.title();
            window.minimized = window.element.is_minimized();
        }
    }

    /// Whether the window went into macOS's own full screen, or came out
    /// of it, since it was last looked at: its new state, if it changed.
    pub fn own_space_changed(&mut self, id: WindowHandle) -> Option<bool> {
        let window = self.windows.get_mut(&id)?;
        let now = window.element.is_fullscreen();
        (now != window.own_space).then(|| {
            window.own_space = now;
            now
        })
    }

    /// The windows of one program.
    pub fn windows_of(&self, pid: i32) -> Vec<WindowHandle> {
        self.windows
            .iter()
            .filter(|(_, window)| window.app.pid == pid)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether a move the window reports now is gevrek's own placing
    /// answered.
    pub fn moved_by_gevrek(&self, id: WindowHandle) -> bool {
        self.placed_at
            .get(&id)
            .is_some_and(|at| at.elapsed() < OWN_MOVE)
    }

    fn display_of(&self, rect: Rect) -> Option<&Display> {
        let (x, y) = (rect.x + rect.width / 2, rect.y + rect.height / 2);
        self.displays
            .iter()
            .find(|display| {
                let b = display.bounds;
                x >= b.x && x < b.x + b.width && y >= b.y && y < b.y + b.height
            })
            .or_else(|| self.displays.iter().find(|display| display.is_main))
    }

    /// Where the window stands for the engine: where it was before it was
    /// parked, while it is.
    fn standing(&self, id: WindowHandle) -> Option<Rect> {
        let window = self.get(id)?;
        if window.parked {
            if let Some(parked) = self.journal.get(window_id_of(id)) {
                return Some(parked.rect());
            }
        }
        Some(window.rect)
    }

    fn place(&mut self, id: WindowHandle, rect: Rect) -> anyhow::Result<()> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(());
        };
        if window.parked {
            // Placed while it waits: it comes back there.
            if let Some(mut parked) = self.journal.get(window_id_of(id)) {
                (parked.x, parked.y, parked.width, parked.height) =
                    (rect.x, rect.y, rect.width, rect.height);
                self.journal.record(window_id_of(id), parked);
            }
            return Ok(());
        }
        self.placed_at.insert(id, Instant::now());
        let landed = window.element.set_frame(rect)?;
        window.rect = landed;
        if let Some(borders) = &mut self.borders {
            borders.follow(id, landed);
        }
        // A program keeps its window at least as big as it says, and
        // macOS has no way to ask how big: what it would not take is
        // learned here.
        let wider = landed.width > rect.width + 1 && landed.width > window.smallest.0;
        let taller = landed.height > rect.height + 1 && landed.height > window.smallest.1;
        if wider || taller {
            if wider {
                window.smallest.0 = landed.width;
            }
            if taller {
                window.smallest.1 = landed.height;
            }
            log::debug!("{id:?} is no smaller than {:?}", window.smallest);
            self.learned.push(id);
        }
        Ok(())
    }

    fn park(&mut self, id: WindowHandle) -> anyhow::Result<()> {
        let Some(window) = self.windows.get(&id) else {
            return Ok(());
        };
        if window.parked {
            return Ok(());
        }
        let from = window.rect;
        let display = self
            .display_of(from)
            .map(|display| (display.id, display.bounds));
        let Some((display_id, bounds)) = display else {
            anyhow::bail!("no display to park a window on");
        };
        let others: Vec<Rect> = self
            .displays
            .iter()
            .filter(|other| other.id != display_id)
            .map(|other| other.bounds)
            .collect();
        let (_, at) = crate::park::spot((from.width, from.height), bounds, &others);
        // Written down first: a crash after the move leaves it findable.
        self.journal.record(
            window_id_of(id),
            Parked {
                pid: window.app.pid,
                x: from.x,
                y: from.y,
                width: from.width,
                height: from.height,
            },
        );
        self.placed_at.insert(id, Instant::now());
        window.element.set_position(at.x, at.y)?;
        if let Some(borders) = &mut self.borders {
            borders.set_visible(id, false);
        }
        if let Some(window) = self.windows.get_mut(&id) {
            window.parked = true;
            if self.focused == Some(id) {
                self.focused = None;
            }
        }
        Ok(())
    }

    fn unpark(&mut self, id: WindowHandle) -> anyhow::Result<()> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(());
        };
        if !window.parked {
            return Ok(());
        }
        // Its place is forgotten once it is back there, and not before:
        // a program that does not answer in time keeps its window parked,
        // with the place to bring it back to. (Forgotten first, the
        // window stayed a sliver in the corner with nowhere to go.)
        let back = self
            .journal
            .get(window_id_of(id))
            .map_or(window.rect, |parked| parked.rect());
        self.placed_at.insert(id, Instant::now());
        window.rect = window.element.set_frame(back)?;
        window.parked = false;
        self.journal.forget(window_id_of(id));
        let rect = window.rect;
        if let Some(borders) = &mut self.borders {
            borders.follow(id, rect);
            borders.set_visible(id, true);
        }
        Ok(())
    }

    /// Say once that something cannot be done here.
    fn not_here(&mut self, what: &'static str) {
        if self.told.insert(what) {
            log::info!("{what} is not possible for other programs' windows on macOS");
        }
    }
}

impl WindowQueries for MacDesktop {
    fn active_window(&self) -> Option<WindowHandle> {
        self.focused
    }

    fn exists(&self, id: WindowHandle) -> bool {
        self.windows.contains_key(&id)
    }

    fn has_title(&self, id: WindowHandle, title: &str) -> Result<bool, String> {
        Ok(self.get(id).is_some_and(|window| window.title.contains(title)))
    }

    fn position(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.standing(id).map(|rect| (rect.x, rect.y))
    }

    fn size(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.standing(id).map(|rect| (rect.width, rect.height))
    }

    fn title(&self, id: WindowHandle) -> String {
        self.get(id).map(|window| window.title.clone()).unwrap_or_default()
    }

    fn class(&self, id: WindowHandle) -> String {
        self.get(id).map(|window| window.app.app_id()).unwrap_or_default()
    }

    fn exe(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.app.executable.clone())
            .unwrap_or_default()
    }

    fn path(&self, id: WindowHandle) -> String {
        self.get(id).map(|window| window.app.path.clone()).unwrap_or_default()
    }

    fn app_id(&self, id: WindowHandle) -> String {
        self.class(id)
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.windows
            .iter()
            .map(|(id, window)| {
                let rect = self.standing(*id).unwrap_or(window.rect);
                WindowInfo {
                    handle: *id,
                    title: window.title.clone(),
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    is_visible: !window.minimized && !window.away(),
                    is_minimized: window.minimized,
                    is_maximized: window.maximized,
                }
            })
            .collect()
    }

    fn rect(&self, id: WindowHandle) -> Result<Rect, String> {
        self.standing(id).ok_or_else(|| format!("no window {id:?}"))
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
        self.get(id)
            .is_some_and(|window| !window.minimized && !window.parked && !window.away())
    }

    fn is_always_on_top(&self, _id: WindowHandle) -> bool {
        false
    }

    fn smallest_size(&self, id: WindowHandle) -> Option<(i32, i32)> {
        self.get(id)
            .map(|window| window.smallest)
            .filter(|smallest| *smallest != (0, 0))
    }

    // A dialog stands where its program put it (over its window).
    fn placed_itself(&self, id: WindowHandle) -> bool {
        self.get(id)
            .is_some_and(|window| window.subrole == "AXDialog" || window.subrole == "AXSystemDialog")
    }

    fn is_tracked(&self, id: WindowHandle) -> bool {
        self.exists(id)
    }

    fn pid(&self, id: WindowHandle) -> Option<u32> {
        self.get(id).map(|window| window.app.pid as u32)
    }

    fn initial_title(&self, id: WindowHandle) -> String {
        self.get(id)
            .map(|window| window.initial_title.clone())
            .unwrap_or_default()
    }

    fn initial_class(&self, id: WindowHandle) -> String {
        self.class(id)
    }

    fn hidden_by_app(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(MacWindow::away)
    }

    fn is_decoration(&self, _id: WindowHandle) -> bool {
        false
    }

    fn has_border(&self, id: WindowHandle) -> bool {
        self.borders.as_ref().is_some_and(|borders| borders.has(id))
    }

    fn opacity(&self, _id: WindowHandle) -> u8 {
        255
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
        let window = self.get(id)?;
        Some(gevrek_engine::rules::WindowInfo {
            handle: id,
            title: window.title.clone(),
            class: window.app.app_id(),
            exe: window.app.executable.clone(),
            path: window.app.path.clone(),
            app_id: window.app.app_id(),
            has_parent: false,
            is_visible: !window.minimized && !window.away(),
            // (A floating panel is what a tool window is on Windows.)
            is_tool_window: window.subrole == "AXFloatingWindow",
            is_noactivate: false,
            has_owner: window.subrole == "AXDialog",
            is_cloaked: false,
            is_layered: false,
            tags: Default::default(),
            workspace: None,
            workspace_index: None,
        })
    }

    fn ignored_by_rules(&self, id: WindowHandle) -> bool {
        let Some(window) = self.get(id) else {
            return true;
        };
        if !window.managed_kind() || !window.app.regular {
            return true;
        }
        let Some(facts) = self.rule_facts(id) else {
            return true;
        };
        self.rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .should_ignore(&facts)
    }

    fn floats_by_rules(&self, id: WindowHandle) -> bool {
        let (Some(window), Some(facts)) = (self.get(id), self.rule_facts(id)) else {
            return false;
        };
        let rules = self
            .rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A rule of the person's own that says float or tile is the last
        // word; without one a dialog, a panel or a window of one size
        // floats, whatever program shows it.
        if let Some(floating) = rules.resolve_static(&facts).floating {
            return floating;
        }
        window.floats_by_kind() || rules.should_float(&facts)
    }
}

impl Desktop for MacDesktop {
    fn apply(&mut self, effect: &Effect) -> anyhow::Result<()> {
        match effect {
            Effect::Configure { id, rect, .. } => self.place(*id, *rect),
            Effect::SetVisible { id, visible: false } => self.park(*id),
            Effect::SetVisible { id, visible: true } => self.unpark(*id),
            Effect::Focus(Some(id)) => {
                let Some(window) = self.get(*id) else {
                    return Ok(());
                };
                window.element.focus()?;
                self.focused = Some(*id);
                Ok(())
            }
            // (Nobody has the keyboard: the program in front keeps it, as
            // macOS has no desktop to give it to.)
            Effect::Focus(None) => Ok(()),
            Effect::Stack { id, op } => match op {
                StackOp::Raise => match self.get(*id) {
                    Some(window) => Ok(window.element.raise()?),
                    None => Ok(()),
                },
                StackOp::Lower => Ok(()),
                StackOp::AlwaysOnTop(_) => {
                    self.not_here("Keeping a window above the others");
                    Ok(())
                }
            },
            Effect::Close(id) => match self.get(*id) {
                Some(window) => Ok(window.element.close()?),
                None => Ok(()),
            },
            Effect::Opacity { .. } => {
                self.not_here("Making a window translucent");
                Ok(())
            }
            Effect::State { id, change } => {
                let Some(window) = self.windows.get_mut(id) else {
                    return Ok(());
                };
                match change {
                    StateChange::Minimized(value) => {
                        window.element.set_minimized(*value)?;
                        window.minimized = *value;
                    }
                    // Both are a place: the engine gives the rectangle.
                    StateChange::Maximized(value) => window.maximized = *value,
                    StateChange::Fullscreen(value) => window.fullscreen = *value,
                }
                Ok(())
            }
            Effect::Animate { id, to, .. } => self.place(*id, *to),
            Effect::WarpPointer(point) => {
                warp_pointer(*point);
                Ok(())
            }
            Effect::Launch(spec) => launch(&gevrek_model::settings::LaunchProfile {
                program: spec.program.clone(),
                args: spec.args.clone(),
                working_directory: spec.working_dir.clone(),
                ..Default::default()
            })
            .map(|_| ()),
            Effect::Decorate { id, deco } => {
                match deco.border {
                    Some(border) => {
                        let look = gevrek_engine::workspace::BorderLook {
                            enabled: true,
                            width: border.width,
                            offset: border.offset,
                            style: if border.radius > 0 {
                                gevrek_model::graph::BorderStyle::Rounded
                            } else {
                                gevrek_model::graph::BorderStyle::Square
                            },
                            active_color: border.color,
                            inactive_color: border.color,
                            active_alpha: border.alpha,
                            inactive_alpha: border.alpha,
                            floating: false,
                        };
                        let focused = self.focused == Some(*id);
                        self.decorate_window(*id, &look, focused)?;
                    }
                    None => {
                        if let Some(borders) = &mut self.borders {
                            borders.remove(*id);
                        }
                    }
                }
                if deco.opacity.is_some() {
                    self.not_here("Making a window translucent");
                }
                Ok(())
            }
            Effect::Border { id, change } => {
                let rect = self.get(*id).map(|window| window.rect);
                let Some(borders) = &mut self.borders else {
                    return Ok(());
                };
                match change {
                    BorderChange::Visible(visible) => borders.set_visible(*id, *visible),
                    BorderChange::Remove => borders.remove(*id),
                    BorderChange::Follow => {
                        if let Some(rect) = rect {
                            borders.follow(*id, rect);
                        }
                    }
                    BorderChange::Focused(focused) => borders.set_focused(*id, *focused),
                }
                Ok(())
            }
            Effect::DecorationsVisible(shown) => {
                if let Some(borders) = &mut self.borders {
                    borders.set_all_shown(*shown);
                }
                if let Some(stackbars) = &mut self.stackbars {
                    stackbars.set_all_shown(*shown);
                }
                Ok(())
            }
            Effect::HideStackbar(key) => {
                if let Some(stackbars) = &mut self.stackbars {
                    stackbars.remove(*key);
                }
                Ok(())
            }
            // (Every bar is given by `sync_stackbars`; the rest is
            // Windows' own.)
            Effect::Stackbar { .. } | Effect::Platform(_) => Ok(()),
        }
    }

    fn output_of(&self, id: WindowHandle) -> Option<OutputId> {
        let rect = self.standing(id)?;
        self.display_of(rect)
            .map(|display| OutputId(display.id as isize))
    }

    // As on Windows: a program opens its window where it last had one.
    fn new_windows_go_where_the_keyboard_is(&self) -> bool {
        true
    }

    fn output_at_cursor(&self) -> Option<OutputId> {
        let at = pointer();
        self.displays
            .iter()
            .find(|display| {
                let b = display.bounds;
                at.x >= b.x && at.x < b.x + b.width && at.y >= b.y && at.y < b.y + b.height
            })
            .map(|display| OutputId(display.id as isize))
    }

    fn primary_button_down(&self) -> bool {
        // SAFETY: plain values; the combined session state, the left button.
        unsafe { ffi::CGEventSourceButtonState(0, 0) }
    }

    // No animation: a program redraws its window for every size it is
    // given, and that is too slow to give it many (`ax.rs`).
    fn apply_animated_rectangles(
        &mut self,
        operations: &[(WindowHandle, Rect)],
        _scope: gevrek_engine::animation_scheduler::TransitionScope,
    ) -> anyhow::Result<()> {
        for (id, rect) in operations {
            if let Err(error) = self.place(*id, *rect) {
                log::debug!("placing {id:?}: {error}");
            }
        }
        Ok(())
    }

    fn begin_workspace_transition(
        &mut self,
        _operations: &[(WindowHandle, Rect)],
        _offset_x: i32,
        _offset_y: i32,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn start_configured_window_enter(&mut self, _id: WindowHandle) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn start_window_animation(
        &mut self,
        _id: WindowHandle,
        _animation_type: gevrek_engine::animations::AnimationType,
        _duration_ms: u32,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn is_animation_event_suppressed(&self, id: WindowHandle) -> bool {
        self.moved_by_gevrek(id)
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
        let Some(at) = self.get(id).filter(|window| !window.parked).map(|window| window.rect) else {
            return Ok(());
        };
        self.decorate_window_at(id, at, look, is_focused)
    }

    fn decorate_window_at(
        &mut self,
        id: WindowHandle,
        at: Rect,
        look: &gevrek_engine::workspace::BorderLook,
        is_focused: bool,
    ) -> anyhow::Result<()> {
        let visible = self
            .get(id)
            .is_some_and(|window| !window.parked && !window.minimized && !window.away());
        if let Some(borders) = &mut self.borders {
            borders.draw(id, at, look, is_focused);
            // Both ways: a window back from being away (its program shown
            // again, out of macOS's full screen) has its border back.
            borders.set_visible(id, visible);
        }
        Ok(())
    }

    fn update_border_position_explicit(
        &mut self,
        id: WindowHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> anyhow::Result<()> {
        if let Some(borders) = &mut self.borders {
            borders.follow(id, Rect::new(x, y, width, height));
        }
        Ok(())
    }

    fn toggle_title_bar(&mut self, _id: WindowHandle) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn sync_stackbars(
        &mut self,
        desired: &[StackbarContainerState],
        config: &gevrek_model::settings::StackbarProfile,
    ) -> anyhow::Result<()> {
        let wanted = desired
            .iter()
            // A bar over a window that is not to be seen is not either.
            .filter(|state| {
                self.get(state.key)
                    .is_some_and(|window| !window.parked && !window.minimized && !window.away())
            })
            .map(|state| {
                let (x, y, width, _) = state.rect;
                crate::stackbars::Wanted {
                    key: state.key,
                    rect: Rect::new(x, y, width, config.height),
                    titles: state
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
                        .collect(),
                    tabs: state.tabs.clone(),
                    focused_tab: state.focused_tab.min(state.tabs.len().saturating_sub(1)),
                }
            })
            .collect();
        if let Some(stackbars) = &mut self.stackbars {
            stackbars.sync(wanted, config);
        }
        Ok(())
    }

    fn launch(&mut self, launch_profile: &gevrek_model::settings::LaunchProfile) -> anyhow::Result<u32> {
        launch(launch_profile)
    }

    fn is_hidden_by_backend(&self, id: WindowHandle) -> bool {
        self.get(id).is_some_and(|window| window.parked)
    }

    fn window_for_native_handle(&self, raw: isize) -> WindowHandle {
        WindowHandle(raw)
    }

    fn untrack_window(&mut self, id: WindowHandle) {
        if let Some(borders) = &mut self.borders {
            borders.remove(id);
        }
        if let Some(stackbars) = &mut self.stackbars {
            stackbars.remove(id);
        }
        self.windows.remove(&id);
        self.placed_at.remove(&id);
        self.journal.forget(window_id_of(id));
        if self.focused == Some(id) {
            self.focused = None;
        }
    }

    /// Put back the windows a gevrek that ended without doing so left
    /// parked. Only those still there, of the same program, are moved.
    fn restore_journaled_windows(&mut self) -> usize {
        let mut restored = 0;
        for (window, parked) in self.journal.take_all() {
            let Some(found) = crate::window_list::find(window) else {
                continue;
            };
            if found.server.pid != parked.pid {
                continue;
            }
            match found.element.set_frame(parked.rect()) {
                Ok(_) => restored += 1,
                Err(error) => log::warn!("Could not put window {window} back: {error}"),
            }
        }
        restored
    }
}

/// Where the pointer is, in points from the top-left of the main display.
pub fn pointer() -> Point {
    // SAFETY: an event made only to read the pointer's place, released.
    unsafe {
        let event = ffi::CGEventCreate(std::ptr::null());
        if event.is_null() {
            return Point { x: 0, y: 0 };
        }
        let at = ffi::CGEventGetLocation(event);
        ffi::CFRelease(event);
        Point {
            x: at.x.round() as i32,
            y: at.y.round() as i32,
        }
    }
}

fn warp_pointer(point: Point) {
    // SAFETY: a plain point.
    unsafe {
        ffi::CGWarpMouseCursorPosition(ffi::CGPoint {
            x: f64::from(point.x),
            y: f64::from(point.y),
        });
    }
}

/// Start a program: a command, or an application by its name or bundle
/// (`Safari`, `/Applications/Safari.app`), which `open` starts.
pub fn launch(profile: &gevrek_model::settings::LaunchProfile) -> anyhow::Result<u32> {
    let program = profile.program.trim();
    let is_bundle = program.ends_with(".app") || program.ends_with(".app/");
    if !is_bundle {
        match gevrek_host::process_manager::spawn_launch(profile) {
            Ok(child) => return Ok(child.id()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    // An application: `open -a` finds it by its name or its bundle.
    let mut args = vec!["-a".to_string(), program.to_string()];
    if !profile.args.is_empty() {
        args.push("--args".to_string());
        args.extend(profile.args.iter().cloned());
    }
    gevrek_host::process_manager::spawn_launch(&gevrek_model::settings::LaunchProfile {
        program: "/usr/bin/open".to_string(),
        args,
        ..profile.clone()
    })
    .map(|child| child.id())
}
