//! Panels without a frame with egui on them: the macOS half of the host.
//!
//! In the shape of the Windows half ([`crate::win`], whose arithmetic of
//! places it shares, [`crate::place`]): a [`Shell`] is the main thread's
//! loop and one graphics device, with any number of surfaces on it, each a
//! window of its own drawn by its own function, opened and closed while it
//! runs through [`Control`].
//!
//! What a [`Place`] asks for, in macOS's terms:
//!
//! - a panel without a frame, on every Space, that gevrek's window manager
//!   leaves alone (it is no window of a regular program) and that does not
//!   make the shell the active program when it is clicked;
//! - `reserve`: macOS lets no program keep room free of the others; the
//!   window manager keeps the panel's room (as on Windows, where the panel
//!   is put at the rectangle it names: [`Control::open_at`]);
//! - `level`: at the desktop, below the windows (above the desktop's icons),
//!   above them, or over everything but the system's menus;
//! - `keyboard`: `Never` is a panel that never takes the keyboard;
//!   `Exclusive` takes it when it opens (a panel takes it without the shell
//!   becoming the active program); `OnDemand` takes it when it is clicked.
//!
//! Sizes are in points, as AppKit and gevrek's window manager measure on
//! macOS; a surface is drawn at its display's scale. A frame is drawn when
//! something happened on a surface, when egui asks for another, and never
//! otherwise: the loop sleeps in between.
//!
//! AppKit tells a program only of the clicks on its own windows: a press
//! elsewhere (which closes a menu) is found by looking at the mouse's
//! buttons, as often as a menu or a surface that takes the keyboard when
//! pressed is open.
//!
//! Not here yet: drag and drop, the desktop shown soft behind a surface,
//! screen readers, input methods (composed text arrives once it is
//! committed), the clipboard.

pub mod input;

use crate::dnd::{DragEvent, DragStart, DropAnswer};
use crate::look::BlurRegion;
use crate::place::{self as places, Area};
use crate::{Edge, Flow, Keyboard, Level, Place, Timings};
use anyhow::Context as _;
use egui_wgpu::wgpu;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSEventType, NSPanel, NSResponder, NSScreen, NSTrackingArea,
    NSTrackingAreaOptions, NSWindow, NSWindowCollectionBehavior, NSWindowSharingType,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSObject, NSPoint, NSRect, NSSize};
use std::ffi::c_void;
use std::time::{Duration, Instant};

pub use crate::place;

/// What draws a surface. It is given the surface's root `Ui` and a way to
/// ask for other things to happen once the frame is done.
pub type Draw = Box<dyn FnMut(&mut egui::Ui, &mut Control) -> Flow>;

/// The name a surface is known by while the shell runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(u32);

/// What a popup is for: a menu takes the pointer (a press outside closes
/// it); a hint is only read, under (or beside) what it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Popped {
    Menu,
    Sub,
    Hint { beside: bool },
}

/// How many presses are kept for a surface that never asks for them.
const PRESSES_KEPT: usize = 16;

/// How often the mouse's buttons are looked at while a press outside the
/// shell's surfaces is waited for (a menu is open).
const OUTSIDE_LOOK: Duration = Duration::from_millis(30);

/// How often the displays are looked at for one that came or went.
const DISPLAYS_LOOK: Duration = Duration::from_secs(2);

/// What a drawing, or a message's handler, asks of the shell. It is
/// carried out when the handler returns.
#[derive(Default)]
pub struct Control {
    open: Vec<(SurfaceId, Place, Draw)>,
    close: Vec<SurfaceId>,
    redraw: Vec<SurfaceId>,
    resize: Vec<(SurfaceId, u32, u32)>,
    popups: Vec<(SurfaceId, SurfaceId, egui::Rect, egui::Vec2, Popped, Draw)>,
    put: Vec<(SurfaceId, Area)>,
    quit: bool,
    next_id: u32,
    known: Vec<SurfaceId>,
    outside: Vec<SurfaceId>,
    pressed: Vec<(SurfaceId, crate::Press)>,
    levels: Vec<(SurfaceId, Level)>,
    keyboards: Vec<(SurfaceId, Keyboard)>,
    captures: Vec<(SurfaceId, bool)>,
    windows: Vec<(SurfaceId, isize)>,
}

impl Control {
    /// Open a surface; it is there after this handler returns.
    pub fn open(
        &mut self,
        place: Place,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.open.push((id, place, Box::new(draw)));
        id
    }

    /// Open a surface at a rectangle of the desktop, in points, that
    /// someone else decided on: the window manager, which keeps the room
    /// of the panel free itself and says where it is.
    pub fn open_at(
        &mut self,
        place: Place,
        area: Area,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        let id = self.open(place, draw);
        self.put.push((id, area));
        id
    }

    /// Give a surface another rectangle of the desktop (see
    /// [`Control::open_at`]); from then on it stays where it is put.
    pub fn put_at(&mut self, id: SurfaceId, area: Area) {
        self.put.push((id, area));
    }

    fn popped(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        popped: Popped,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups
            .push((id, parent, anchor, size, popped, Box::new(draw)));
        id
    }

    /// Open a surface that belongs to another and may reach beyond it (a
    /// menu under a button of a panel): `size` large, under `anchor` (a
    /// rectangle on the parent, in its own points), where the display has
    /// room. A press outside the shell's surfaces closes it, and so does
    /// Escape.
    pub fn popup(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.popped(parent, anchor, size, Popped::Menu, draw)
    }

    /// Open a menu that belongs to a line of another menu: as
    /// [`Control::popup`], but beside `anchor`, their tops in line.
    pub fn submenu(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.popped(parent, anchor, size, Popped::Sub, draw)
    }

    /// Open a hint: as [`Control::popup`], but only to be read. It takes
    /// neither the pointer nor a click. `beside`: next to the anchor.
    pub fn hint(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        beside: bool,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.popped(parent, anchor, size, Popped::Hint { beside }, draw)
    }

    /// The buttons that went down on a surface since this was last asked
    /// for it, each with the system's own time of the press.
    pub fn presses(&mut self, id: SurfaceId) -> Vec<crate::Press> {
        let mut mine = Vec::new();
        self.pressed.retain(|(on, press)| {
            if *on == id {
                mine.push(*press);
            }
            *on != id
        });
        mine
    }

    pub fn close(&mut self, id: SurfaceId) {
        self.close.push(id);
    }

    /// Draw a surface again (what it shows changed from outside).
    pub fn redraw(&mut self, id: SurfaceId) {
        self.redraw.push(id);
    }

    /// Give a surface another size (points).
    pub fn resize(&mut self, id: SurfaceId, width: u32, height: u32) {
        self.resize.push((id, width, height));
    }

    /// Draw every surface again.
    pub fn redraw_all(&mut self) {
        let all = self.known.clone();
        self.redraw.extend(all);
    }

    /// Whether a mouse button was pressed outside the shell's surfaces
    /// since this was last asked for the surface (one that takes the
    /// keyboard when it is pressed, and is put away by such a press).
    pub fn pressed_outside(&mut self, id: SurfaceId) -> bool {
        let before = self.outside.len();
        self.outside.retain(|pressed| *pressed != id);
        self.outside.len() != before
    }

    /// Let other programs' drags be dropped on a surface. Not yet on
    /// macOS: nothing is dropped.
    pub fn accept_drops(&mut self, _id: SurfaceId, _standing: Option<DropAnswer>) {}

    /// What a surface would do with the drag over it. (Nothing is dragged
    /// over one on macOS yet.)
    pub fn answer_drag(&mut self, _id: SurfaceId, _answer: DropAnswer) {}

    /// What happened to drags over and out of a surface: nothing, on macOS
    /// yet.
    pub fn drag_events(&mut self, _id: SurfaceId) -> Vec<DragEvent> {
        Vec::new()
    }

    /// Drag files or text out of a surface. Not yet on macOS.
    pub fn start_drag(&mut self, from: SurfaceId, _drag: DragStart) {
        log::debug!("surface {from:?} asked to drag something out: not yet on macOS");
    }

    /// The window of a surface, as the window server numbers it.
    pub fn native_window(&self, id: SurfaceId) -> Option<isize> {
        self.windows
            .iter()
            .find(|(surface, _)| *surface == id)
            .map(|(_, number)| *number)
    }

    /// Put a surface at another level among the windows.
    pub fn set_level(&mut self, id: SurfaceId, level: Level) {
        self.levels.push((id, level));
    }

    /// Let a surface take the keyboard another way.
    pub fn set_keyboard(&mut self, id: SurfaceId, keyboard: Keyboard) {
        self.keyboards.push((id, keyboard));
    }

    /// The desktop shown soft behind parts of a surface: not yet on macOS.
    pub fn set_blur_regions(&mut self, _id: SurfaceId, _regions: &[BlurRegion]) {}

    pub fn blur_available(&self) -> bool {
        false
    }

    /// Leave a surface out of pictures and recordings of the screen.
    pub fn exclude_from_capture(&mut self, id: SurfaceId, exclude: bool) {
        self.captures.push((id, exclude));
    }

    pub fn is_open(&self, id: SurfaceId) -> bool {
        self.known.contains(&id)
            || self.open.iter().any(|(opened, ..)| *opened == id)
            || self.popups.iter().any(|(opened, ..)| *opened == id)
    }

    /// End the shell's loop.
    pub fn quit(&mut self) {
        self.quit = true;
    }
}

// --- waking the loop from another thread -----------------------------------

#[repr(C)]
struct RunLoopSourceContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
    schedule: *const c_void,
    cancel: *const c_void,
    perform: extern "C" fn(*mut c_void),
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopGetCurrent() -> *const c_void;
    fn CFRunLoopSourceCreate(
        alloc: *const c_void,
        order: isize,
        context: *mut RunLoopSourceContext,
    ) -> *const c_void;
    fn CFRunLoopAddSource(run_loop: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopSourceSignal(source: *const c_void);
    fn CFRunLoopWakeUp(run_loop: *const c_void);
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, after_source: bool) -> i32;
}

extern "C" fn woken(_info: *mut c_void) {}

/// The main thread's run loop and a source on it that another thread
/// signals: the loop, asleep, wakes for it.
struct Waker {
    run_loop: *const c_void,
    source: *const c_void,
}

// Signalling a run loop source and waking a run loop may be done from any
// thread (Core Foundation's own words); both live as long as the program.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

impl Waker {
    fn new() -> Self {
        let mut context = RunLoopSourceContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: std::ptr::null(),
            release: std::ptr::null(),
            copy_description: std::ptr::null(),
            equal: std::ptr::null(),
            hash: std::ptr::null(),
            schedule: std::ptr::null(),
            cancel: std::ptr::null(),
            perform: woken,
        };
        // SAFETY: a source of version 0 with a callback that does nothing,
        // added to this thread's run loop; kept for the program's life.
        unsafe {
            let run_loop = CFRunLoopGetCurrent();
            let source = CFRunLoopSourceCreate(std::ptr::null(), 0, &mut context);
            CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
            Self { run_loop, source }
        }
    }

    fn wake(&self) {
        // SAFETY: see `unsafe impl Send`.
        unsafe {
            CFRunLoopSourceSignal(self.source);
            CFRunLoopWakeUp(self.run_loop);
        }
    }
}

/// A way to send the running shell a message from another thread.
pub struct Sender<M> {
    channel: std::sync::mpsc::Sender<M>,
    waker: std::sync::Arc<Waker>,
}

impl<M> Clone for Sender<M> {
    fn clone(&self) -> Self {
        Self {
            channel: self.channel.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M> Sender<M> {
    /// `false` when the shell is gone.
    pub fn send(&self, message: M) -> bool {
        if self.channel.send(message).is_err() {
            return false;
        }
        self.waker.wake();
        true
    }
}

// --- the panels -------------------------------------------------------------

define_class!(
    // SAFETY: NSPanel has no subclassing requirements; the class adds no
    // state and no `Drop`.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GevrekShellPanel"]
    struct ShellPanel;

    impl ShellPanel {
        /// A window without a frame takes no keyboard unless it says it
        /// can: a launcher's text field needs it.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }
    }
);

/// The level of AppKit's windows a surface of this level stands at.
fn window_level(level: Level) -> isize {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowLevelForKey(key: i32) -> i32;
    }
    // (Core Graphics' keys of its levels.)
    const DESKTOP: i32 = 2;
    const DESKTOP_ICON: i32 = 18;
    // SAFETY: plain numbers.
    let of = |key| isize::try_from(unsafe { CGWindowLevelForKey(key) }).unwrap_or(0);
    match level {
        Level::Background => of(DESKTOP) + 1,
        // Above the desktop's icons, below every window.
        Level::Below => of(DESKTOP_ICON) + 1,
        // Over the windows (floating panels).
        Level::Above => 3,
        // Over everything but the system's own menus (a popup menu's level).
        Level::Overlay | Level::Marks => 101,
    }
}

fn make_panel(mtm: MainThreadMarker, level: Level, popped: Option<Popped>) -> Retained<NSWindow> {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(10.0, 10.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: the panel class's own initializer, with plain arguments.
    let panel: Retained<ShellPanel> = unsafe {
        msg_send![
            mtm.alloc::<ShellPanel>(),
            initWithContentRect: rect,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    let panel: Retained<NSPanel> = Retained::into_super(panel);
    panel.setFloatingPanel(false);
    let window: Retained<NSWindow> = Retained::into_super(panel);
    // SAFETY: kept by `Surface`, so not to be released when closed.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setHidesOnDeactivate(false);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(matches!(popped, Some(Popped::Menu | Popped::Sub)));
    window.setLevel(window_level(level));
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    window.setAcceptsMouseMovedEvents(true);
    window.setIgnoresMouseEvents(matches!(popped, Some(Popped::Hint { .. })));
    if let Some(view) = window.contentView() {
        view.setWantsLayer(true);
        // Moves of the pointer over the surface, whichever program is
        // active (the shell never is).
        let options = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: a tracking area owned by the view it is added to.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                view.bounds(),
                options,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);
    }
    window
}

// --- displays ----------------------------------------------------------------

/// A display: its name, all of it, and what the menu bar and the Dock
/// leave free of it, in points from the top-left of the main display.
#[derive(Debug, Clone, PartialEq)]
struct Display {
    name: String,
    frame: Area,
    visible: Area,
}

fn area_of(rect: NSRect, main_height: f64) -> Area {
    let top = main_height - (rect.origin.y + rect.size.height);
    Area::new(
        rect.origin.x.round() as i32,
        top.round() as i32,
        rect.size.width.round() as i32,
        rect.size.height.round() as i32,
    )
}

fn rect_of(area: Area, main_height: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(
            f64::from(area.x),
            main_height - f64::from(area.y) - f64::from(area.height),
        ),
        NSSize::new(f64::from(area.width), f64::from(area.height)),
    )
}

fn main_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .iter()
        .next()
        .map_or(0.0, |screen| screen.frame().size.height)
}

fn displays(mtm: MainThreadMarker) -> Vec<Display> {
    let height = main_height(mtm);
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| Display {
            name: screen.localizedName().to_string(),
            frame: area_of(screen.frame(), height),
            visible: area_of(screen.visibleFrame(), height),
        })
        .collect()
}

/// The pointer's place, in points from the top-left of the main display.
fn pointer(mtm: MainThreadMarker) -> (i32, i32) {
    let at = NSEvent::mouseLocation();
    let height = main_height(mtm);
    (at.x.round() as i32, (height - at.y).round() as i32)
}

// --- the host ---------------------------------------------------------------

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
}

struct Surface {
    // (First: the graphics surface goes before the window it draws on.)
    target: wgpu::Surface<'static>,
    renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    alpha: wgpu::CompositeAlphaMode,
    /// The size the graphics surface was made for, in pixels.
    configured: (u32, u32),
    window: Retained<NSWindow>,
    id: SurfaceId,
    place: Place,
    popped: Option<(SurfaceId, egui::Rect, Popped)>,
    /// Where someone else put it (`open_at`, `put_at`).
    put: Option<Area>,
    /// Where it is, in points.
    area: Area,
    scale: f32,
    context: egui::Context,
    draw: Draw,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    redraw: bool,
    again_at: Option<Instant>,
    cursor: egui::CursorIcon,
    pointer_inside: bool,
}

impl Surface {
    fn configure_target(&mut self, device: &wgpu::Device) {
        let width = ((self.area.width.max(1) as f32) * self.scale).round() as u32;
        let height = ((self.area.height.max(1) as f32) * self.scale).round() as u32;
        self.target.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width,
                height,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 1,
                alpha_mode: self.alpha,
                view_formats: Vec::new(),
            },
        );
        self.configured = (width, height);
    }

    fn takes_presses_outside(&self) -> bool {
        matches!(self.popped, Some((_, _, Popped::Menu | Popped::Sub)))
            || self.place.keyboard == Keyboard::OnDemand
    }
}

struct Host {
    mtm: MainThreadMarker,
    instance: wgpu::Instance,
    gpu: Option<Gpu>,
    surfaces: Vec<Surface>,
    control: Control,
    started: Instant,
    timings: Timings,
    total_ms: f64,
    /// The mouse's buttons as they were last looked at (for presses
    /// outside the shell's surfaces).
    buttons_down: usize,
    displays: Vec<Display>,
    displays_looked: Instant,
}

impl Host {
    fn new(mtm: MainThreadMarker) -> Self {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::METAL;
        let descriptor = descriptor.with_env();
        Self {
            mtm,
            instance: wgpu::Instance::new(descriptor),
            gpu: None,
            surfaces: Vec::new(),
            control: Control::default(),
            started: Instant::now(),
            timings: Timings::default(),
            total_ms: 0.0,
            buttons_down: 0,
            displays: displays(mtm),
            displays_looked: Instant::now(),
        }
    }

    fn index_of(&self, id: SurfaceId) -> Option<usize> {
        self.surfaces.iter().position(|surface| surface.id == id)
    }

    /// The display a place names, or the one the pointer is on.
    fn display_for(&self, place: &Place) -> Option<Display> {
        if let Some(name) = &place.output {
            if let Some(display) = self.displays.iter().find(|display| display.name == *name) {
                return Some(display.clone());
            }
        }
        let (x, y) = pointer(self.mtm);
        self.displays
            .iter()
            .find(|display| display.frame.holds(x, y))
            .or_else(|| self.displays.first())
            .cloned()
    }

    /// The display that holds most of a rectangle.
    fn display_holding(&self, area: Area) -> Option<Display> {
        let frames: Vec<Area> = self.displays.iter().map(|display| display.frame).collect();
        places::display_holding(area, &frames)
            .map(|index| self.displays[index].clone())
            .or_else(|| self.displays.first().cloned())
    }

    /// Where a surface goes now.
    fn area_for(&self, surface: &Surface) -> Area {
        if let Some(area) = surface.put {
            return area;
        }
        if let Some((parent, anchor, popped)) = surface.popped {
            let Some(parent) = self.index_of(parent).map(|index| &self.surfaces[index]) else {
                return surface.area;
            };
            let anchor = Area::new(
                parent.area.x + anchor.min.x.round() as i32,
                parent.area.y + anchor.min.y.round() as i32,
                anchor.width().round() as i32,
                anchor.height().round() as i32,
            );
            let size = (surface.place.width as i32, surface.place.height as i32);
            let room = self
                .display_holding(parent.area)
                .map_or(anchor, |display| display.visible);
            return match popped {
                Popped::Menu | Popped::Hint { beside: false } => {
                    places::popup_at(anchor, size, room, 2)
                }
                Popped::Hint { beside: true } => places::beside(anchor, size, room, 4),
                Popped::Sub => places::beside_line(anchor, size, room, -4),
            };
        }
        let Some(display) = self.display_for(&surface.place) else {
            return surface.area;
        };
        // What covers a display covers all of it; the rest stays in what
        // the menu bar and the Dock leave free.
        let room = if surface.place.edge == Edge::Fill {
            display.frame
        } else {
            display.visible
        };
        places::placed(&surface.place, room, 1.0)
    }

    /// Put a surface where it goes, at its display's scale, and show it.
    fn settle(&mut self, index: usize) {
        let area = self.area_for(&self.surfaces[index]);
        let height = main_height(self.mtm);
        let surface = &mut self.surfaces[index];
        surface
            .window
            .setFrame_display(rect_of(area, height), false);
        surface.area = area;
        surface.scale = surface.window.backingScaleFactor() as f32;
        if let Some(gpu) = &self.gpu {
            surface.configure_target(&gpu.device);
        }
        surface.redraw = true;
    }

    fn settle_all(&mut self) {
        for index in 0..self.surfaces.len() {
            self.settle(index);
        }
    }

    fn make_surface(
        &mut self,
        id: SurfaceId,
        place: Place,
        popped: Option<(SurfaceId, egui::Rect, Popped)>,
        draw: Draw,
    ) -> anyhow::Result<Surface> {
        let window = make_panel(self.mtm, place.level, popped.map(|(_, _, popped)| popped));
        let view = window.contentView().context("the panel's view")?;
        let handle = raw_window_handle::RawWindowHandle::AppKit(
            raw_window_handle::AppKitWindowHandle::new(
                std::ptr::NonNull::new(Retained::as_ptr(&view) as *mut c_void)
                    .context("the view")?,
            ),
        );
        // SAFETY: the view lives as long as the window, which lives as long
        // as `Surface`, whose graphics surface is its first field and so is
        // dropped before the window it draws on.
        let target = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_window_handle::RawDisplayHandle::AppKit(
                        raw_window_handle::AppKitDisplayHandle::new(),
                    )),
                    raw_window_handle: handle,
                })
        }
        .context("a graphics surface")?;
        if self.gpu.is_none() {
            let adapter =
                pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&target),
                    ..Default::default()
                }))
                .context("no graphics adapter draws on this window")?;
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }))
            .context("the graphics device")?;
            let info = adapter.get_info();
            log::debug!("drawing with {} ({:?})", info.name, info.backend);
            device.on_uncaptured_error(std::sync::Arc::new(|error| {
                log::warn!("the graphics device: {error}");
            }));
            self.gpu = Some(Gpu {
                device,
                queue,
                adapter,
            });
        }
        let gpu = self.gpu.as_ref().expect("just made");
        let capabilities = target.get_capabilities(&gpu.adapter);
        // A format without its own colour curve: egui gives colours as they
        // are to be shown.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .context("the window has no format")?;
        // See-through where nothing is drawn (rounded corners).
        let alpha = [
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::PreMultiplied,
        ]
        .into_iter()
        .find(|mode| capabilities.alpha_modes.contains(mode))
        .or_else(|| capabilities.alpha_modes.first().copied())
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        Ok(Surface {
            target,
            renderer,
            format,
            alpha,
            configured: (0, 0),
            window,
            id,
            place,
            popped,
            put: None,
            area: Area::default(),
            scale: 1.0,
            context: egui::Context::default(),
            draw,
            events: Vec::new(),
            modifiers: egui::Modifiers::default(),
            redraw: true,
            again_at: None,
            cursor: egui::CursorIcon::Default,
            pointer_inside: false,
        })
    }

    /// Bring a new surface on the screen.
    fn show(&mut self, index: usize) {
        self.settle(index);
        let surface = &self.surfaces[index];
        if surface.place.keyboard == Keyboard::Exclusive
            || matches!(surface.popped, Some((_, _, Popped::Menu | Popped::Sub)))
        {
            surface.window.makeKeyAndOrderFront(None);
        } else {
            surface.window.orderFrontRegardless();
        }
    }

    /// What the drawings and the messages asked for, carried out.
    fn carry_out(&mut self) {
        loop {
            let opens = std::mem::take(&mut self.control.open);
            let popups = std::mem::take(&mut self.control.popups);
            let puts = std::mem::take(&mut self.control.put);
            let closes = std::mem::take(&mut self.control.close);
            let resizes = std::mem::take(&mut self.control.resize);
            let redraws = std::mem::take(&mut self.control.redraw);
            let levels = std::mem::take(&mut self.control.levels);
            let keyboards = std::mem::take(&mut self.control.keyboards);
            let captures = std::mem::take(&mut self.control.captures);
            if opens.is_empty()
                && popups.is_empty()
                && puts.is_empty()
                && closes.is_empty()
                && resizes.is_empty()
                && redraws.is_empty()
                && levels.is_empty()
                && keyboards.is_empty()
                && captures.is_empty()
            {
                break;
            }
            for (id, place, draw) in opens {
                match self.make_surface(id, place, None, draw) {
                    Ok(surface) => self.surfaces.push(surface),
                    Err(error) => log::error!("a surface could not be made: {error:#}"),
                }
            }
            for (id, area) in puts {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].put = Some(area);
                }
            }
            for (id, parent, anchor, size, popped, draw) in popups {
                if self.index_of(parent).is_none() {
                    continue;
                }
                let mut place = Place::panel("gevrek-popup", 0);
                place.width = size.x.ceil().max(1.0) as u32;
                place.height = size.y.ceil().max(1.0) as u32;
                place.level = Level::Overlay;
                place.reserve = false;
                match self.make_surface(id, place, Some((parent, anchor, popped)), draw) {
                    Ok(surface) => self.surfaces.push(surface),
                    Err(error) => log::error!("a popup could not be made: {error:#}"),
                }
            }
            // New surfaces (and those put elsewhere) are shown where they go.
            for index in 0..self.surfaces.len() {
                if self.surfaces[index].configured == (0, 0) {
                    self.show(index);
                }
            }
            for (id, area) in self.surfaces.iter().filter_map(|surface| surface.put.map(|area| (surface.id, area))).collect::<Vec<_>>() {
                if let Some(index) = self.index_of(id) {
                    if self.surfaces[index].area != area {
                        self.settle(index);
                    }
                }
            }
            for (id, width, height) in resizes {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].place.width = width;
                    self.surfaces[index].place.height = height;
                    self.settle(index);
                }
            }
            for (id, level) in levels {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].place.level = level;
                    self.surfaces[index].window.setLevel(window_level(level));
                }
            }
            for (id, keyboard) in keyboards {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].place.keyboard = keyboard;
                    if keyboard == Keyboard::Exclusive {
                        self.surfaces[index].window.makeKeyAndOrderFront(None);
                    }
                }
            }
            for (id, exclude) in captures {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].window.setSharingType(if exclude {
                        NSWindowSharingType::None
                    } else {
                        NSWindowSharingType::ReadOnly
                    });
                }
            }
            // Closed, with whatever popped out of them.
            let mut closing = closes;
            loop {
                let children: Vec<SurfaceId> = self
                    .surfaces
                    .iter()
                    .filter(|surface| {
                        surface
                            .popped
                            .is_some_and(|(parent, ..)| closing.contains(&parent))
                            && !closing.contains(&surface.id)
                    })
                    .map(|surface| surface.id)
                    .collect();
                if children.is_empty() {
                    break;
                }
                closing.extend(children);
            }
            for id in closing {
                if let Some(index) = self.index_of(id) {
                    let surface = self.surfaces.remove(index);
                    surface.window.orderOut(None);
                    surface.window.close();
                }
            }
            for id in redraws {
                if let Some(index) = self.index_of(id) {
                    self.surfaces[index].redraw = true;
                }
            }
            self.control.known = self.surfaces.iter().map(|surface| surface.id).collect();
            self.control.windows = self
                .surfaces
                .iter()
                .map(|surface| (surface.id, surface.window.windowNumber()))
                .collect();
        }
    }

    /// An event AppKit has for one of the surfaces, as egui input. `false`:
    /// it is for none of them.
    fn take_in(&mut self, event: &NSEvent) -> bool {
        let number = event.windowNumber();
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.window.windowNumber() == number)
        else {
            return false;
        };
        let kind = event.r#type();
        let surface = &mut self.surfaces[index];
        let modifiers = input::modifiers_of(event.modifierFlags());
        surface.modifiers = modifiers;
        let height = surface.area.height as f32;
        let at = || {
            let point = event.locationInWindow();
            egui::pos2(point.x as f32, height - point.y as f32)
        };
        match kind {
            NSEventType::MouseMoved
            | NSEventType::LeftMouseDragged
            | NSEventType::RightMouseDragged
            | NSEventType::OtherMouseDragged
            | NSEventType::MouseEntered => {
                surface.pointer_inside = true;
                surface.events.push(egui::Event::PointerMoved(at()));
                input::cursor_of(surface.cursor).set();
            }
            NSEventType::MouseExited => {
                surface.pointer_inside = false;
                surface.events.push(egui::Event::PointerGone);
            }
            NSEventType::LeftMouseDown
            | NSEventType::RightMouseDown
            | NSEventType::OtherMouseDown
            | NSEventType::LeftMouseUp
            | NSEventType::RightMouseUp
            | NSEventType::OtherMouseUp => {
                let Some(button) = input::button_of(event.buttonNumber()) else {
                    return true;
                };
                let pressed = matches!(
                    kind,
                    NSEventType::LeftMouseDown | NSEventType::RightMouseDown | NSEventType::OtherMouseDown
                );
                let pos = at();
                if pressed {
                    // A surface that takes the keyboard when it is pressed
                    // takes it now.
                    if surface.place.keyboard == Keyboard::OnDemand {
                        surface.window.makeKeyWindow();
                    }
                    let press = crate::Press {
                        button,
                        at: pos,
                        time_ms: (event.timestamp() * 1000.0) as u32,
                    };
                    self.control.pressed.push((surface.id, press));
                    let over = self.control.pressed.len().saturating_sub(PRESSES_KEPT);
                    self.control.pressed.drain(..over);
                }
                surface.events.push(egui::Event::PointerMoved(pos));
                surface.events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                });
            }
            NSEventType::ScrollWheel => {
                let (x, y) = (event.scrollingDeltaX() as f32, event.scrollingDeltaY() as f32);
                let unit = if event.hasPreciseScrollingDeltas() {
                    egui::MouseWheelUnit::Point
                } else {
                    egui::MouseWheelUnit::Line
                };
                surface.events.push(egui::Event::MouseWheel {
                    unit,
                    delta: egui::vec2(x, y),
                    source: if event.hasPreciseScrollingDeltas() {
                        egui::MouseWheelSource::Trackpad
                    } else {
                        egui::MouseWheelSource::Wheel
                    },
                    modifiers,
                    phase: egui::TouchPhase::Move,
                });
            }
            NSEventType::KeyDown | NSEventType::KeyUp => {
                let pressed = kind == NSEventType::KeyDown;
                if let Some(key) = input::key_of(event.keyCode()) {
                    // Command and a key: what egui knows as copy, cut and
                    // paste are its own events.
                    if pressed && modifiers.command && key == egui::Key::C {
                        surface.events.push(egui::Event::Copy);
                    } else if pressed && modifiers.command && key == egui::Key::X {
                        surface.events.push(egui::Event::Cut);
                    }
                    surface.events.push(egui::Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed,
                        repeat: pressed && event.isARepeat(),
                        modifiers,
                    });
                }
                if pressed {
                    if let Some(text) = event
                        .characters()
                        .and_then(|characters| input::text_of(&characters.to_string(), modifiers))
                    {
                        surface.events.push(egui::Event::Text(text));
                    }
                }
            }
            NSEventType::FlagsChanged => {}
            _ => return false,
        }
        surface.redraw = true;
        true
    }

    /// The mouse's buttons, looked at: a press that is on none of the
    /// shell's surfaces is a press outside, told to those that wait for
    /// one, and closes the open menus.
    fn look_at_the_buttons(&mut self) {
        let down = NSEvent::pressedMouseButtons();
        let pressed = down & !self.buttons_down != 0;
        self.buttons_down = down;
        if !pressed {
            return;
        }
        let (x, y) = pointer(self.mtm);
        if self.surfaces.iter().any(|surface| surface.area.holds(x, y)) {
            return;
        }
        let waiting: Vec<SurfaceId> = self
            .surfaces
            .iter()
            .filter(|surface| surface.takes_presses_outside())
            .map(|surface| surface.id)
            .collect();
        for id in waiting {
            let index = self.index_of(id).expect("just seen");
            if matches!(self.surfaces[index].popped, Some((_, _, Popped::Menu | Popped::Sub))) {
                self.control.close(id);
            } else {
                self.control.outside.push(id);
                self.surfaces[index].redraw = true;
            }
        }
    }

    fn draw(&mut self, index: usize) {
        let began = Instant::now();
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        surface.redraw = false;
        if surface.area.width <= 0 || surface.area.height <= 0 {
            return;
        }
        let points = egui::vec2(surface.area.width as f32, surface.area.height as f32);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut surface.events),
            focused: surface.window.isKeyWindow(),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(surface.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };
        let mut flow = Flow::Continue;
        let draw = &mut surface.draw;
        let control = &mut self.control;
        let mut output = surface.context.run_ui(input, |root| {
            if draw(root, control) == Flow::Close {
                flow = Flow::Close;
            }
        });
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                surface
                    .renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        output.textures_delta.set.clear();
        let freed = std::mem::take(&mut output.textures_delta.free);
        if flow == Flow::Close {
            let id = surface.id;
            self.control.close(id);
            return;
        }
        let wanted = output.platform_output.cursor_icon;
        if wanted != surface.cursor {
            surface.cursor = wanted;
            if surface.pointer_inside {
                input::cursor_of(wanted).set();
            }
        }
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            if viewport.repaint_delay.is_zero() {
                surface.redraw = true;
            } else if viewport.repaint_delay < Duration::from_secs(3600) {
                surface.again_at = Some(Instant::now() + viewport.repaint_delay);
            }
        }
        let jobs = surface
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [surface.configured.0, surface.configured.1],
            pixels_per_point: output.pixels_per_point,
        };
        let frame = match surface.target.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                log::debug!("no frame to draw into on surface {:?}: {other:?}", surface.id);
                surface.configure_target(&gpu.device);
                surface.again_at = Some(Instant::now() + Duration::from_millis(100));
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let before =
            surface
                .renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            surface.renderer.render(&mut pass, &jobs, &screen);
        }
        gpu.queue
            .submit(before.into_iter().chain(std::iter::once(encoder.finish())));
        for id in &freed {
            surface.renderer.free_texture(id);
        }
        gpu.queue.present(frame);
        let took = began.elapsed().as_secs_f64() * 1000.0;
        if self.timings.frames == 0 {
            self.timings.first_frame_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        }
        self.timings.frames += 1;
        self.total_ms += took;
        self.timings.worst_ms = self.timings.worst_ms.max(took);
    }
}

// --- the shell ----------------------------------------------------------------

/// The main thread's surfaces.
pub struct Shell<M: 'static> {
    host: Host,
    app: Retained<NSApplication>,
    messages: std::sync::mpsc::Receiver<M>,
    sender: Sender<M>,
}

impl<M: 'static> Shell<M> {
    /// On the main thread only (AppKit's windows are made there).
    pub fn new() -> anyhow::Result<Self> {
        let mtm = MainThreadMarker::new().context("the shell runs on the main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        // No Dock icon and no menu bar of its own.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let (channel, messages) = std::sync::mpsc::channel();
        Ok(Self {
            host: Host::new(mtm),
            app,
            messages,
            sender: Sender {
                channel,
                waker: std::sync::Arc::new(Waker::new()),
            },
        })
    }

    /// For another thread to talk to the shell with.
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    /// Open a surface before the shell runs.
    pub fn open(
        &mut self,
        place: Place,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        let id = self.host.control.open(place, draw);
        self.host.carry_out();
        id
    }

    /// Run until a handler asks to quit. `on_message` gets every message
    /// sent through a [`Sender`]. With `until_empty`, the shell also ends
    /// when its last surface is closed.
    pub fn run(
        self,
        until_empty: bool,
        on_message: impl FnMut(M, &mut Control),
    ) -> anyhow::Result<Timings> {
        self.run_with_outputs(until_empty, on_message, |_, _| {})
    }

    /// As [`Shell::run`]; `on_outputs` is told the names of the displays
    /// when the shell starts and each time one comes or goes.
    pub fn run_with_outputs(
        mut self,
        until_empty: bool,
        mut on_message: impl FnMut(M, &mut Control),
        mut on_outputs: impl FnMut(&[String], &mut Control),
    ) -> anyhow::Result<Timings> {
        let names = |host: &Host| {
            let mut names: Vec<String> = host.displays.iter().map(|display| display.name.clone()).collect();
            names.sort();
            names
        };
        let mut known = names(&self.host);
        on_outputs(&known, &mut self.host.control);
        self.host.carry_out();
        loop {
            if self.host.control.quit || (until_empty && self.host.surfaces.is_empty()) {
                break;
            }
            // Sleep until AppKit has an event, another thread a message, a
            // surface is due, or the buttons are to be looked at.
            let now = Instant::now();
            let mut wait = if self.host.surfaces.iter().any(|surface| surface.redraw) {
                Duration::ZERO
            } else {
                self.host
                    .surfaces
                    .iter()
                    .filter_map(|surface| surface.again_at)
                    .min()
                    .map_or(Duration::from_secs(3600), |at| at.saturating_duration_since(now))
            };
            if self.host.surfaces.iter().any(Surface::takes_presses_outside) {
                wait = wait.min(OUTSIDE_LOOK);
            }
            wait = wait.min(DISPLAYS_LOOK);
            // SAFETY: the default mode; this thread's run loop.
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, wait.as_secs_f64(), true) };
            // What AppKit has: the surfaces' own as egui input, the rest
            // to AppKit as its own loop would hand it on.
            let past = NSDate::distantPast();
            // SAFETY: the system's constant.
            let mode = unsafe { objc2_foundation::NSDefaultRunLoopMode };
            while let Some(event) =
                self.app
                    .nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&past), mode, true)
            {
                if !self.host.take_in(&event) {
                    self.app.sendEvent(&event);
                }
            }
            self.host.look_at_the_buttons();
            for message in self.messages.try_iter() {
                on_message(message, &mut self.host.control);
            }
            self.host.carry_out();
            if self.host.displays_looked.elapsed() >= DISPLAYS_LOOK {
                self.host.displays_looked = Instant::now();
                let now = displays(self.host.mtm);
                if now != self.host.displays {
                    self.host.displays = now;
                    self.host.settle_all();
                    let names = names(&self.host);
                    if names != known {
                        on_outputs(&names, &mut self.host.control);
                        known = names;
                    }
                }
            }
            let now = Instant::now();
            for index in 0..self.host.surfaces.len() {
                let surface = &mut self.host.surfaces[index];
                if surface.again_at.is_some_and(|at| now >= at) {
                    surface.again_at = None;
                    surface.redraw = true;
                }
                if surface.redraw {
                    self.host.draw(index);
                }
            }
            self.host.carry_out();
        }
        for surface in self.host.surfaces.drain(..) {
            surface.window.orderOut(None);
            surface.window.close();
        }
        if self.host.timings.frames > 0 {
            self.host.timings.mean_ms = self.host.total_ms / self.host.timings.frames as f64;
        }
        Ok(self.host.timings)
    }
}

/// Show one surface at `place` and draw `ui` on it until it says to close.
pub fn run(
    place: Place,
    mut ui: impl FnMut(&mut egui::Ui) -> Flow + 'static,
) -> anyhow::Result<Timings> {
    let mut shell: Shell<()> = Shell::new()?;
    shell.open(place, move |root, _| ui(root));
    shell.run(true, |(), _| {})
}
