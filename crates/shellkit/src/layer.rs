//! Layer-shell surfaces with egui on them.
//!
//! A [`Shell`] is one connection to the compositor and one graphics
//! device, with any number of surfaces on it, each drawn by its own
//! function. Surfaces are opened and closed while it runs: by the
//! drawing of another surface, or by a message from outside (another
//! thread that listens to the window manager), through [`Control`].
//!
//! A frame is drawn when something happened on a surface, when egui asks
//! for another (an animation, a blinking cursor), and never otherwise: a
//! still panel costs nothing.
//!
//! [`run`] is the short form for one surface.

use crate::{Edge, Flow, Keyboard, Level, Place, Timings};
use anyhow::Context as _;
use egui_wgpu::wgpu;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{AxisScroll, PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            popup::{Popup, PopupConfigure, PopupHandler},
            XdgPositioner, XdgShell,
        },
        WaylandSurface,
    },
};
use std::ptr::NonNull;
use std::time::{Duration, Instant};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3},
    zwp_text_input_v3::{self, ContentHint, ContentPurpose, ZwpTextInputV3},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

/// What draws a surface. It is given the surface's root `Ui` and a way to
/// ask for other things to happen once the frame is done.
pub type Draw = Box<dyn FnMut(&mut egui::Ui, &mut Control) -> Flow>;

/// The name a surface is known by while the shell runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(u32);

/// What kind of popup: a menu takes the pointer (a click outside closes
/// it); a hint is only read, under (or over) what it is about, or
/// beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Popped {
    Menu,
    Hint { beside: bool },
}

/// What a drawing, or a message's handler, asks of the shell. It is
/// carried out when the handler returns.
#[derive(Default)]
pub struct Control {
    open: Vec<(SurfaceId, Place, Draw)>,
    close: Vec<SurfaceId>,
    redraw: Vec<SurfaceId>,
    resize: Vec<(SurfaceId, u32, u32)>,
    /// (The flag: it takes the pointer, so that a click outside closes it.)
    popups: Vec<(SurfaceId, SurfaceId, egui::Rect, egui::Vec2, Popped, Draw)>,
    reach: Vec<(SurfaceId, Vec<egui::Rect>)>,
    lock: Option<Box<dyn FnMut() -> Draw>>,
    unlock: bool,
    quit: bool,
    next_id: u32,
    known: Vec<SurfaceId>,
    /// Which of the keyboard's layouts is in use (its place, from 0), as
    /// the compositor last told it.
    layout: u32,
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

    /// Open a surface that belongs to another and may reach beyond it (a
    /// menu under a button of a panel): `size` large, next to `anchor`
    /// (a rectangle on the parent, in its own points), where the
    /// compositor finds room. A click outside it closes it.
    pub fn popup(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups
            .push((id, parent, anchor, size, Popped::Menu, Box::new(draw)));
        id
    }

    /// Open a hint: as [`Control::popup`], but only to be read. It takes
    /// neither the pointer nor a click (what is under it gets them), and
    /// stays until it is closed. `beside`: next to the anchor (for a
    /// panel that goes down a side) and not under it.
    pub fn hint(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        beside: bool,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups.push((
            id,
            parent,
            anchor,
            size,
            Popped::Hint { beside },
            Box::new(draw),
        ));
        id
    }

    /// The parts of a surface the pointer can reach (in its own points):
    /// everywhere else, clicks go to what is under the surface. For a
    /// surface that is larger than what it shows.
    pub fn reach(&mut self, id: SurfaceId, parts: Vec<egui::Rect>) {
        self.reach.push((id, parts));
    }

    pub fn close(&mut self, id: SurfaceId) {
        self.close.push(id);
    }

    /// Which of the keyboard's layouts is in use: its place among them,
    /// from 0 (the compositor tells it with the keys' state).
    pub fn keyboard_layout(&self) -> u32 {
        self.layout
    }

    /// Draw a surface again (what it shows changed from outside).
    pub fn redraw(&mut self, id: SurfaceId) {
        self.redraw.push(id);
    }

    /// Give a surface another size (logical pixels).
    pub fn resize(&mut self, id: SurfaceId, width: u32, height: u32) {
        self.resize.push((id, width, height));
    }

    /// Draw every surface again.
    pub fn redraw_all(&mut self) {
        let all = self.known.clone();
        self.redraw.extend(all);
    }

    /// Whether a surface is open now.
    pub fn is_open(&self, id: SurfaceId) -> bool {
        // (Also one asked for earlier in this pass, which is made when
        // the pass is over: asked for a second time it would be made
        // twice, and the first never closed.)
        let asked_for = self.open.iter().any(|(asked, ..)| *asked == id)
            || self.popups.iter().any(|(asked, ..)| *asked == id);
        (self.known.contains(&id) || asked_for) && !self.close.contains(&id)
    }

    /// Lock the session: when the compositor grants it, every display
    /// gets a surface that covers it, each drawn by a function `each`
    /// makes. Nothing else is shown or reachable until [`Control::unlock`].
    /// If the compositor refuses (another lock is there), the shell ends.
    pub fn lock(&mut self, each: impl FnMut() -> Draw + 'static) {
        self.lock = Some(Box::new(each));
    }

    /// End the lock: the session is shown again, and the shell ends.
    pub fn unlock(&mut self) {
        self.unlock = true;
    }

    /// End the shell: every surface is closed and `run` returns.
    pub fn quit(&mut self) {
        self.quit = true;
    }
}

/// A way to send the running shell a message from another thread.
pub struct Sender<M> {
    channel: smithay_client_toolkit::reexports::calloop::channel::Sender<M>,
}

impl<M> Clone for Sender<M> {
    fn clone(&self) -> Self {
        Self {
            channel: self.channel.clone(),
        }
    }
}

impl<M> Sender<M> {
    /// `false` when the shell is gone.
    pub fn send(&self, message: M) -> bool {
        self.channel.send(message).is_ok()
    }
}

/// One connection to the compositor with surfaces on it.
pub struct Shell<M: 'static> {
    event_loop: EventLoop<'static, Host>,
    host: Host,
    handle: QueueHandle<Host>,
    messages: std::rc::Rc<std::cell::RefCell<Vec<M>>>,
    sender: Sender<M>,
}

impl<M: 'static> Shell<M> {
    pub fn new() -> anyhow::Result<Self> {
        let started = Instant::now();
        let connection = Connection::connect_to_env().context("no Wayland compositor")?;
        let (globals, queue) =
            registry_queue_init(&connection).context("the compositor's globals")?;
        let handle: QueueHandle<Host> = queue.handle();
        let compositor = CompositorState::bind(&globals, &handle).context("wl_compositor")?;
        // A lock screen needs no layer shell, a panel no session lock:
        // what is missing is said when it is asked for.
        let layer_shell = LayerShell::bind(&globals, &handle).ok();
        let event_loop: EventLoop<Host> = EventLoop::try_new().context("an event loop")?;
        WaylandSource::new(connection.clone(), queue)
            .insert(event_loop.handle())
            .map_err(|error| anyhow::anyhow!("the Wayland source: {error}"))?;
        // Messages from other threads wake the loop and wait in a list.
        let (channel, receiver) =
            smithay_client_toolkit::reexports::calloop::channel::channel::<M>();
        let messages = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let waiting = std::rc::Rc::clone(&messages);
        event_loop
            .handle()
            .insert_source(receiver, move |event, _, _| {
                if let smithay_client_toolkit::reexports::calloop::channel::Event::Msg(message) =
                    event
                {
                    waiting.borrow_mut().push(message);
                }
            })
            .map_err(|error| anyhow::anyhow!("the message source: {error}"))?;
        // What wakes the loop when a screen reader asks something of a
        // surface (from the thread that speaks with it).
        let (access_wake, woken) = smithay_client_toolkit::reexports::calloop::ping::make_ping()
            .map_err(|error| anyhow::anyhow!("the wake for accessibility: {error}"))?;
        event_loop
            .handle()
            .insert_source(woken, |_, _, _| {})
            .map_err(|error| anyhow::anyhow!("the wake for accessibility: {error}"))?;
        let host = Host {
            access_wake,
            surfaces: Vec::new(),
            upright: Vec::new(),
            gpu: None,
            instance: wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            compositor,
            layer_shell,
            xdg_shell: XdgShell::bind(&globals, &handle).ok(),
            fractional: globals.bind(&handle, 1..=1, ()).ok(),
            viewporter: globals.bind(&handle, 1..=1, ()).ok(),
            handle: handle.clone(),
            text_manager: globals.bind(&handle, 1..=1, ()).ok(),
            text_input: None,
            text: TextInput::default(),
            seat: None,
            last_press: None,
            lock_state: SessionLockState::new(&globals, &handle),
            lock: None,
            lock_draw: None,
            registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &handle),
            outputs: OutputState::new(&globals, &handle),
            keyboard: None,
            pointer: None,
            touch: None,
            fingers: Vec::new(),
            modifiers: egui::Modifiers::default(),
            keyboard_on: None,
            pointer_on: None,
            started,
            timings: Timings::default(),
            total_ms: 0.0,
            control: Control::default(),
            connection,
        };
        Ok(Self {
            event_loop,
            host,
            handle,
            messages,
            sender: Sender { channel },
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
        self.host.carry_out(&self.handle);
        id
    }

    /// Lock the session before the shell runs (see [`Control::lock`]).
    pub fn lock(&mut self, each: impl FnMut() -> Draw + 'static) {
        self.host.control.lock(each);
        self.host.carry_out(&self.handle);
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
        // The displays are known after the first answers of the compositor.
        let _ = self
            .event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut self.host);
        let mut known: Option<Vec<String>> = None;
        loop {
            let mut names: Vec<String> = self
                .host
                .outputs
                .outputs()
                .filter_map(|output| self.host.outputs.info(&output).and_then(|info| info.name))
                .collect();
            names.sort();
            if known.as_ref() != Some(&names) {
                on_outputs(&names, &mut self.host.control);
                known = Some(names);
                self.host.carry_out(&self.handle);
            }
            let now = Instant::now();
            let wait = self
                .host
                .surfaces
                .iter()
                .filter_map(|surface| surface.again_at)
                .min()
                .map(|at| at.saturating_duration_since(now));
            self.event_loop
                .dispatch(wait, &mut self.host)
                .context("the event loop")?;
            let waiting: Vec<M> = std::mem::take(&mut *self.messages.borrow_mut());
            for message in waiting {
                on_message(message, &mut self.host.control);
            }
            self.host.carry_out(&self.handle);
            self.host.access_news();
            let now = Instant::now();
            for index in 0..self.host.surfaces.len() {
                let surface = &mut self.host.surfaces[index];
                if surface.again_at.is_some_and(|at| now >= at) {
                    surface.again_at = None;
                    surface.redraw = true;
                }
                if surface.redraw && surface.configured && !surface.waiting_for_frame {
                    self.host.draw(index, &self.handle);
                }
            }
            self.host.carry_out(&self.handle);
            if self.host.control.quit || (until_empty && self.host.surfaces.is_empty()) {
                break;
            }
        }
        // Surfaces first: they draw on the connection.
        self.host.surfaces.clear();
        if self.host.timings.frames > 0 {
            self.host.timings.mean_ms = self.host.total_ms / self.host.timings.frames as f64;
        }
        Ok(self.host.timings)
    }
}

/// Show one surface at `place` and draw `ui` on it until it says to close
/// (or the compositor closes it).
pub fn run(
    place: Place,
    mut ui: impl FnMut(&mut egui::Ui) -> Flow + 'static,
) -> anyhow::Result<Timings> {
    let mut shell: Shell<()> = Shell::new()?;
    shell.open(place, move |root, _| ui(root));
    shell.run(true, |(), _| {})
}

/// The graphics device every surface draws with.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
}

/// What a surface is to the compositor.
enum Role {
    Layer(LayerSurface),
    Lock(SessionLockSurface),
    /// A popup, and the surface it belongs to.
    Popup(Popup, SurfaceId),
}

impl Role {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Role::Layer(layer) => layer.wl_surface(),
            Role::Lock(lock) => lock.wl_surface(),
            Role::Popup(popup, _) => popup.wl_surface(),
        }
    }
}

/// One surface and what is drawn on it.
struct Surface {
    id: SurfaceId,
    // Dropped first: it draws on the layer surface below.
    target: wgpu::Surface<'static>,
    renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    alpha: wgpu::CompositeAlphaMode,
    role: Role,
    context: egui::Context,
    draw: Draw,
    /// What it shows, as screen readers and other aids are told it.
    access: Access,
    /// In logical pixels.
    size: (u32, u32),
    /// Pixels of the display to a logical pixel: a whole number as the
    /// surface is told it, or any (1.25, 1.5) where the compositor says
    /// the display's own (`fraction`).
    scale: f32,
    /// The display's scale as a fraction, and the surface's picture
    /// stretched to its logical size (fractional-scale and viewporter):
    /// with them the surface is drawn at exactly the display's pixels.
    fraction: Option<(WpFractionalScaleV1, WpViewport)>,
    configured: bool,
    /// What happened since the last frame, for egui.
    events: Vec<egui::Event>,
    /// The surface has the keyboard.
    focused: bool,
    redraw: bool,
    /// egui asked to be drawn again at this time.
    again_at: Option<Instant>,
    /// A frame was handed over and the compositor has not shown it yet.
    waiting_for_frame: bool,
}

struct Host {
    /// Wakes the loop for a screen reader's sake (see [`Access`]).
    access_wake: smithay_client_toolkit::reexports::calloop::ping::Ping,
    // Dropped before the connection they live on.
    surfaces: Vec<Surface>,
    /// The surfaces that go down a side of the display: what pops up
    /// from them stands beside them, not under.
    upright: Vec<SurfaceId>,
    gpu: Option<Gpu>,
    instance: wgpu::Instance,
    compositor: CompositorState,
    layer_shell: Option<LayerShell>,
    /// For popups.
    xdg_shell: Option<XdgShell>,
    /// The display's scale as a fraction, and pictures stretched to a
    /// surface's logical size, where the compositor has both.
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    handle: QueueHandle<Host>,
    /// Input methods (text-input v3), where the compositor has them: a
    /// text field of one of the surfaces is told to the input method,
    /// and what it makes of the keys is put into the field.
    text_manager: Option<ZwpTextInputManagerV3>,
    text_input: Option<ZwpTextInputV3>,
    text: TextInput,
    seat: Option<wl_seat::WlSeat>,
    /// The last press of a button: what lets a popup take the pointer.
    last_press: Option<u32>,
    lock_state: SessionLockState,
    /// The lock, while the session is locked by this program.
    lock: Option<SessionLock>,
    /// Makes the drawing of each display's lock surface.
    lock_draw: Option<Box<dyn FnMut() -> Draw>>,
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// A touch screen's fingers, where the seat has one: each finger
    /// that is down, with the surface it came down on and where it is.
    /// The first of them is the pointer for egui (it presses, drags and
    /// lets go), every one is told as a touch (for two-finger moves).
    touch: Option<wayland_client::protocol::wl_touch::WlTouch>,
    fingers: Vec<(i32, SurfaceId, egui::Pos2)>,
    modifiers: egui::Modifiers,
    /// The surface that has the keyboard, and the one under the pointer.
    keyboard_on: Option<SurfaceId>,
    pointer_on: Option<SurfaceId>,
    started: Instant,
    timings: Timings,
    total_ms: f64,
    /// What handlers asked for and is not carried out yet.
    control: Control,
    // Dropped last: everything above lives on it.
    connection: Connection,
}

impl Surface {
    /// Its size in the display's pixels.
    fn pixels(&self) -> (u32, u32) {
        (
            (self.size.0 as f32 * self.scale).round() as u32,
            (self.size.1 as f32 * self.scale).round() as u32,
        )
    }

    fn configure_target(&mut self, device: &wgpu::Device) {
        let (width, height) = self.pixels();
        if width == 0 || height == 0 {
            return;
        }
        // (A picture of the display's own pixels is shown at the
        // surface's logical size.)
        if let Some((_, viewport)) = &self.fraction {
            viewport.set_destination(self.size.0 as i32, self.size.1 as i32);
        }
        self.target.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width,
                height,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: self.alpha,
                view_formats: Vec::new(),
            },
        );
    }
}

impl Host {
    fn index_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|candidate| candidate.role.wl_surface() == surface)
    }

    fn by_id(&mut self, id: Option<SurfaceId>) -> Option<&mut Surface> {
        let id = id?;
        self.surfaces.iter_mut().find(|surface| surface.id == id)
    }

    /// Do what handlers asked for: open, close, draw again.
    fn carry_out(&mut self, handle: &QueueHandle<Self>) {
        for id in std::mem::take(&mut self.control.close) {
            // A surface's popups go with it (and before it).
            self.surfaces.retain(
                |surface| !matches!(&surface.role, Role::Popup(_, parent) if *parent == id),
            );
            self.surfaces.retain(|surface| surface.id != id);
            if self.keyboard_on == Some(id) {
                self.keyboard_on = None;
            }
            if self.pointer_on == Some(id) {
                self.pointer_on = None;
            }
            self.upright.retain(|upright| *upright != id);
        }
        for (id, place, draw) in std::mem::take(&mut self.control.open) {
            match self.make(id, &place, draw, handle) {
                Ok(surface) => self.surfaces.push(surface),
                Err(error) => {
                    log::error!("the surface {} could not be opened: {error:#}", place.name)
                }
            }
        }
        for (id, parent, anchor, size, kind, draw) in std::mem::take(&mut self.control.popups) {
            match self.make_popup(id, parent, anchor, size, kind, draw, handle) {
                Ok(surface) => self.surfaces.push(surface),
                Err(error) => log::error!("a popup could not be opened: {error:#}"),
            }
        }
        for (id, parts) in std::mem::take(&mut self.control.reach) {
            let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor)
            else {
                continue;
            };
            for part in parts {
                region.add(
                    part.min.x.floor() as i32,
                    part.min.y.floor() as i32,
                    part.width().ceil().max(1.0) as i32,
                    part.height().ceil().max(1.0) as i32,
                );
            }
            if let Some(surface) = self.by_id(Some(id)) {
                // (Taken up with the surface's next picture.)
                surface
                    .role
                    .wl_surface()
                    .set_input_region(Some(region.wl_region()));
                surface.redraw = true;
            }
        }
        for (id, width, height) in std::mem::take(&mut self.control.resize) {
            if let Some(surface) = self.by_id(Some(id)) {
                if surface.size != (width, height) {
                    // The compositor answers with a configure, which
                    // makes the picture the new size.
                    if let Role::Layer(layer) = &surface.role {
                        layer.set_size(width, height);
                        layer.commit();
                    }
                }
            }
        }
        for id in std::mem::take(&mut self.control.redraw) {
            if let Some(surface) = self.by_id(Some(id)) {
                surface.redraw = true;
            }
        }
        if let Some(each) = self.control.lock.take() {
            match self.lock_state.lock(handle) {
                Ok(lock) => {
                    self.lock = Some(lock);
                    self.lock_draw = Some(each);
                }
                Err(error) => {
                    log::error!("the session cannot be locked: {error}");
                    self.control.quit = true;
                }
            }
        }
        if std::mem::take(&mut self.control.unlock) {
            if let Some(lock) = self.lock.take() {
                // The surfaces go first, then the lock; the compositor
                // must have heard before this program ends.
                self.surfaces
                    .retain(|surface| !matches!(surface.role, Role::Lock(_)));
                lock.unlock();
                let _ = self.connection.roundtrip();
            }
            self.control.quit = true;
        }
        self.control.known = self.surfaces.iter().map(|surface| surface.id).collect();
    }

    fn make(
        &mut self,
        id: SurfaceId,
        place: &Place,
        draw: Draw,
        handle: &QueueHandle<Self>,
    ) -> anyhow::Result<Surface> {
        let surface = self.compositor.create_surface(handle);
        let level = match place.level {
            Level::Background => Layer::Background,
            Level::Below => Layer::Bottom,
            Level::Above => Layer::Top,
            Level::Overlay | Level::Marks => Layer::Overlay,
        };
        let layer_shell = self
            .layer_shell
            .as_ref()
            .context("the compositor has no layer shell")?;
        // On the display that is named, when it is there.
        let output = place.output.as_ref().and_then(|wanted| {
            self.outputs.outputs().find(|output| {
                self.outputs
                    .info(output)
                    .and_then(|info| info.name)
                    .is_some_and(|name| name == *wanted)
            })
        });
        if place.output.is_some() && output.is_none() {
            anyhow::bail!(
                "there is no display {}",
                place.output.as_deref().unwrap_or("")
            );
        }
        let layer = layer_shell.create_layer_surface(
            handle,
            surface,
            level,
            Some(place.name.clone()),
            output.as_ref(),
        );
        let along = |full: bool, one: Anchor, other: Anchor| {
            if full {
                one | other
            } else {
                Anchor::empty()
            }
        };
        let anchor = match place.edge {
            Edge::Top => Anchor::TOP | along(place.width == 0, Anchor::LEFT, Anchor::RIGHT),
            Edge::Bottom => Anchor::BOTTOM | along(place.width == 0, Anchor::LEFT, Anchor::RIGHT),
            Edge::Left => Anchor::LEFT | along(place.height == 0, Anchor::TOP, Anchor::BOTTOM),
            Edge::Right => Anchor::RIGHT | along(place.height == 0, Anchor::TOP, Anchor::BOTTOM),
            Edge::TopRight => Anchor::TOP | Anchor::RIGHT,
            Edge::TopLeft => Anchor::TOP | Anchor::LEFT,
            Edge::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
            Edge::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            Edge::Fill => Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            Edge::Centre => Anchor::empty(),
        };
        if matches!(place.edge, Edge::Left | Edge::Right) {
            self.upright.push(id);
        }
        layer.set_anchor(anchor);
        layer.set_size(place.width, place.height);
        layer.set_margin(place.margin, place.margin, place.margin, place.margin);
        if place.reserve {
            let thickness = match place.edge {
                Edge::Top | Edge::Bottom => place.height,
                Edge::Left | Edge::Right => place.width,
                _ => 0,
            };
            // (Its own thickness: the room between it and the display's
            // edge is added by the compositor, as the protocol says.)
            layer.set_exclusive_zone(thickness as i32);
        } else if place.edge == Edge::Fill {
            // The whole display, the room panels keep free too: what
            // fills a display covers it (a frozen picture of it to drag
            // a part out of was squeezed into what the panel left, and
            // the part taken was not the part dragged over).
            layer.set_exclusive_zone(-1);
        }
        layer.set_keyboard_interactivity(match place.keyboard {
            Keyboard::Never => KeyboardInteractivity::None,
            Keyboard::OnDemand => KeyboardInteractivity::OnDemand,
            Keyboard::Exclusive => KeyboardInteractivity::Exclusive,
        });
        if place.level == Level::Marks {
            // Marks: the pointer goes through them.
            if let Ok(nowhere) = smithay_client_toolkit::compositor::Region::new(&self.compositor) {
                layer.wl_surface().set_input_region(Some(nowhere.wl_region()));
            }
        }
        // The first commit has no picture: the compositor answers with the size.
        layer.commit();
        self.finish(id, Role::Layer(layer), draw)
    }

    #[allow(clippy::too_many_arguments)]
    fn make_popup(
        &mut self,
        id: SurfaceId,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        kind: Popped,
        draw: Draw,
        handle: &QueueHandle<Self>,
    ) -> anyhow::Result<Surface> {
        use wayland_protocols::xdg::shell::client::xdg_positioner::{
            Anchor as At, ConstraintAdjustment, Gravity,
        };
        let xdg_shell = self
            .xdg_shell
            .as_ref()
            .context("the compositor has no popups")?;
        let positioner = XdgPositioner::new(xdg_shell)
            .map_err(|error| anyhow::anyhow!("a positioner: {error}"))?;
        positioner.set_size(
            size.x.round().max(1.0) as i32,
            size.y.round().max(1.0) as i32,
        );
        positioner.set_anchor_rect(
            anchor.min.x.round() as i32,
            anchor.min.y.round() as i32,
            anchor.width().round().max(1.0) as i32,
            anchor.height().round().max(1.0) as i32,
        );
        // Below the anchor, its left edges in line; flipped above, or
        // slid along, where that would leave the display.
        if kind == (Popped::Hint { beside: true }) || self.upright.contains(&parent) {
            // Next to the anchor, their middles in line; flipped to the
            // other side, or slid along, where that would leave the display.
            positioner.set_anchor(At::Right);
            positioner.set_gravity(Gravity::Right);
            positioner.set_constraint_adjustment(
                ConstraintAdjustment::FlipX | ConstraintAdjustment::SlideY,
            );
            positioner.set_offset(6, 0);
        } else {
            positioner.set_anchor(At::BottomLeft);
            positioner.set_gravity(Gravity::BottomRight);
            positioner.set_constraint_adjustment(
                ConstraintAdjustment::FlipY | ConstraintAdjustment::SlideX,
            );
            positioner.set_offset(0, 4);
        }
        let surface = self.compositor.create_surface(handle);
        let popup = Popup::from_surface(None, &positioner, handle, surface, xdg_shell)
            .map_err(|error| anyhow::anyhow!("a popup: {error}"))?;
        let Some(Role::Layer(layer)) = self
            .surfaces
            .iter()
            .find(|surface| surface.id == parent)
            .map(|surface| &surface.role)
        else {
            anyhow::bail!("the surface it belongs to is gone");
        };
        layer.get_popup(popup.xdg_popup());
        if kind == Popped::Menu {
            // With the pointer: a click outside it then closes it.
            if let (Some(seat), Some(serial)) = (&self.seat, self.last_press) {
                popup.xdg_popup().grab(seat, serial);
            }
        } else if let Ok(nowhere) =
            smithay_client_toolkit::compositor::Region::new(&self.compositor)
        {
            // A hint: the pointer goes through it.
            popup
                .wl_surface()
                .set_input_region(Some(nowhere.wl_region()));
        }
        popup.wl_surface().commit();
        let mut made = self.finish(id, Role::Popup(popup, parent), draw)?;
        made.size = (
            size.x.round().max(1.0) as u32,
            size.y.round().max(1.0) as u32,
        );
        Ok(made)
    }

    /// The graphics side of a surface that has its place.
    fn finish(&mut self, id: SurfaceId, role: Role, draw: Draw) -> anyhow::Result<Surface> {
        // The graphics card, through the display and the surface as the
        // system's own Wayland library knows them.
        let display = raw_window_handle::RawDisplayHandle::Wayland(
            raw_window_handle::WaylandDisplayHandle::new(
                NonNull::new(self.connection.backend().display_ptr().cast())
                    .context("the display")?,
            ),
        );
        let window = raw_window_handle::RawWindowHandle::Wayland(
            raw_window_handle::WaylandWindowHandle::new(
                NonNull::new(role.wl_surface().id().as_ptr().cast()).context("the surface")?,
            ),
        );
        // SAFETY: the display lives as long as `Host`, which owns the
        // connection and drops its surfaces first; the Wayland surface
        // lives as long as `Surface`, whose graphics surface is its first
        // field and so is dropped before the surface it draws on.
        let target = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: window,
                })
        }
        .context("a graphics surface")?;
        if self.gpu.is_none() {
            let adapter =
                pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&target),
                    ..Default::default()
                }))
                .context("no graphics adapter draws on this surface")?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .context("the graphics device")?;
            self.gpu = Some(Gpu {
                device,
                queue,
                adapter,
            });
        }
        let gpu = self.gpu.as_ref().expect("just made");
        let capabilities = target.get_capabilities(&gpu.adapter);
        // A format without its own colour curve: egui gives colours as
        // they are to be shown.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .context("the surface has no format")?;
        let alpha = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        // The display's scale as a fraction, where the compositor says it.
        let fraction = match (&self.fractional, &self.viewporter) {
            (Some(fractional), Some(viewporter)) => {
                let handle = self.handle.clone();
                let wl_surface = role.wl_surface();
                Some((
                    fractional.get_fractional_scale(wl_surface, &handle, wl_surface.clone()),
                    viewporter.get_viewport(wl_surface, &handle, ()),
                ))
            }
            _ => None,
        };
        Ok(Surface {
            id,
            target,
            renderer,
            format,
            alpha,
            role,
            context: egui::Context::default(),
            draw,
            access: Access::new(self.access_wake.clone()),
            size: (0, 0),
            scale: 1.0,
            fraction,
            configured: false,
            events: Vec::new(),
            focused: false,
            redraw: true,
            again_at: None,
            waiting_for_frame: false,
        })
    }

    fn draw(&mut self, index: usize, handle: &QueueHandle<Self>) {
        let began = Instant::now();
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        surface.redraw = false;
        let points = egui::vec2(surface.size.0 as f32, surface.size.1 as f32);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut surface.events),
            // A surface without the keyboard blinks no cursor.
            focused: surface.focused,
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
        // The pictures egui made or changed (its font atlas first of
        // all) are taken in whatever becomes of this frame.
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                surface
                    .renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        output.textures_delta.set.clear();
        let freed = std::mem::take(&mut output.textures_delta.free);
        // What the frame shows, for a screen reader that listens.
        if let Some(update) = output.platform_output.accesskit_update.take() {
            surface.access.adapter.update_if_active(|| update);
        }
        // A text field that is typed into, for the input method: told
        // when there is one, where its text cursor is, and when no more.
        if self.text.on == Some(surface.id) {
            if let Some(field) = &self.text_input {
                let wanted = output.platform_output.ime.as_ref().map(|ime| {
                    let cursor = ime.cursor_rect;
                    (
                        [
                            cursor.left().round() as i32,
                            cursor.top().round() as i32,
                            cursor.width().round().max(1.0) as i32,
                            cursor.height().round().max(1.0) as i32,
                        ],
                        matches!(ime.purpose, egui::IMEPurpose::Password),
                    )
                });
                self.text.tell(field, wanted);
            }
        }
        if flow == Flow::Close {
            let id = surface.id;
            self.control.close(id);
            return;
        }
        // When egui wants the next frame.
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
            size_in_pixels: surface.pixels().into(),
            pixels_per_point: output.pixels_per_point,
        };
        let frame = match surface.target.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                log::debug!("no frame to draw into: {other:?}");
                surface.configure_target(&gpu.device);
                surface.redraw = true;
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
        // Told when this frame is shown: the next is not drawn before.
        // (Asked for before the frame is handed over: handing over commits.)
        let wl_surface = surface.role.wl_surface();
        wl_surface.frame(handle, wl_surface.clone());
        surface.waiting_for_frame = true;
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

impl LayerShellHandler for Host {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // The compositor took it away (its output is gone).
        if let Some(index) = self.index_of(layer.wl_surface()) {
            let id = self.surfaces[index].id;
            self.control.close(id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.index_of(layer.wl_surface()) else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        let (width, height) = configure.new_size;
        if width > 0 && height > 0 {
            surface.size = (width, height);
        }
        surface.configured = surface.size.0 > 0 && surface.size.1 > 0;
        surface.configure_target(&gpu.device);
        surface.redraw = true;
    }
}

impl Host {
    /// A lock surface for a display, drawn by what the factory makes.
    fn cover(&mut self, output: &wl_output::WlOutput, handle: &QueueHandle<Self>) {
        let (Some(lock), Some(each)) = (&self.lock, &mut self.lock_draw) else {
            return;
        };
        let draw = each();
        let surface = self.compositor.create_surface(handle);
        let lock_surface = lock.create_lock_surface(surface, output, handle);
        self.control.next_id += 1;
        let id = SurfaceId(self.control.next_id);
        match self.finish(id, Role::Lock(lock_surface), draw) {
            Ok(surface) => self.surfaces.push(surface),
            Err(error) => log::error!("a display's lock surface could not be made: {error:#}"),
        }
    }
}

impl SessionLockHandler for Host {
    fn locked(&mut self, _: &Connection, handle: &QueueHandle<Self>, _: SessionLock) {
        let outputs: Vec<wl_output::WlOutput> = self.outputs.outputs().collect();
        for output in &outputs {
            self.cover(output, handle);
        }
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        // Refused (something else locks the session), or ended by the
        // compositor: there is nothing for this program to do.
        log::warn!("the compositor ended the lock");
        self.lock = None;
        self.control.quit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        lock_surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.index_of(lock_surface.wl_surface()) else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        let (width, height) = configure.new_size;
        if width > 0 && height > 0 {
            surface.size = (width, height);
        }
        surface.configured = surface.size.0 > 0 && surface.size.1 > 0;
        surface.configure_target(&gpu.device);
        surface.redraw = true;
    }
}

/// The toolkit's popups come with its windows; the host has none.
impl smithay_client_toolkit::shell::xdg::window::WindowHandler for Host {
    fn request_close(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &smithay_client_toolkit::shell::xdg::window::Window,
    ) {
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &smithay_client_toolkit::shell::xdg::window::Window,
        _: smithay_client_toolkit::shell::xdg::window::WindowConfigure,
        _: u32,
    ) {
    }
}

impl PopupHandler for Host {
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        popup: &Popup,
        configure: PopupConfigure,
    ) {
        let Some(index) = self.index_of(popup.wl_surface()) else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        if configure.width > 0 && configure.height > 0 {
            surface.size = (configure.width as u32, configure.height as u32);
        }
        surface.configured = surface.size.0 > 0 && surface.size.1 > 0;
        surface.configure_target(&gpu.device);
        surface.redraw = true;
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        // Dismissed by the compositor (a click elsewhere).
        if let Some(index) = self.index_of(popup.wl_surface()) {
            let id = self.surfaces[index].id;
            self.control.close(id);
        }
    }
}

impl CompositorHandler for Host {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        wl_surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        let Some(index) = self.index_of(wl_surface) else {
            return;
        };
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        // (Where the display's scale is told as a fraction, that holds,
        // and the surface's picture is of the display's own pixels.)
        if surface.fraction.is_some() {
            return;
        }
        if factor as f32 != surface.scale && factor > 0 {
            surface.scale = factor as f32;
            wl_surface.set_buffer_scale(factor);
            surface.configure_target(&gpu.device);
            surface.redraw = true;
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        wl_surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(index) = self.index_of(wl_surface) {
            self.surfaces[index].waiting_for_frame = false;
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Host {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(
        &mut self,
        _: &Connection,
        handle: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // A display plugged in while the session is locked is covered too.
        if self.lock.is_some() && !self.surfaces.is_empty() {
            self.cover(&output, handle);
        }
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for Host {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        handle: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(handle, &seat, None).ok();
        }
        if let (Some(manager), None) = (&self.text_manager, &self.text_input) {
            self.text_input = Some(manager.get_text_input(&seat, handle, ()));
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(handle, &seat).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seats.get_touch(handle, &seat).ok();
        }
        self.seat.get_or_insert(seat);
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Host {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let modifiers = self.modifiers;
        for event in events {
            let Some(index) = self.index_of(&event.surface) else {
                continue;
            };
            let surface = &mut self.surfaces[index];
            let at = egui::pos2(event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_on = Some(surface.id);
                    surface.events.push(egui::Event::PointerMoved(at));
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_on == Some(surface.id) {
                        self.pointer_on = None;
                    }
                    surface.events.push(egui::Event::PointerGone);
                }
                PointerEventKind::Press { button, .. }
                | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                    if let PointerEventKind::Press { serial, .. } = event.kind {
                        self.last_press = Some(serial);
                    }
                    // The numbers of Linux's buttons: left, right, middle.
                    let button = match button {
                        0x110 => egui::PointerButton::Primary,
                        0x111 => egui::PointerButton::Secondary,
                        0x112 => egui::PointerButton::Middle,
                        _ => continue,
                    };
                    surface.events.push(egui::Event::PointerButton {
                        pos: at,
                        button,
                        pressed,
                        modifiers,
                    });
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    source,
                    ..
                } => {
                    use wl_pointer::AxisSource;
                    let (unit, delta) = wheel(&horizontal, &vertical);
                    let source = match source {
                        Some(AxisSource::Wheel | AxisSource::WheelTilt) => {
                            egui::MouseWheelSource::Wheel
                        }
                        Some(AxisSource::Finger) => egui::MouseWheelSource::Trackpad,
                        _ => egui::MouseWheelSource::Unknown,
                    };
                    surface.events.push(egui::Event::MouseWheel {
                        source,
                        unit,
                        delta,
                        phase: egui::TouchPhase::Move,
                        modifiers,
                    });
                }
            }
            surface.redraw = true;
        }
    }
}

/// What a scroll event turns, as egui counts it. A wheel is counted in
/// its notches (as on Windows, so that a notch is the same everywhere: a
/// line of a list, a workspace on the panel); what has no notches (a
/// touchpad) in the points it moved. Away from the user is more than
/// nothing, which is the other way round from what the compositor sends.
fn wheel(horizontal: &AxisScroll, vertical: &AxisScroll) -> (egui::MouseWheelUnit, egui::Vec2) {
    let turned = egui::vec2(-horizontal.discrete as f32, -vertical.discrete as f32);
    if turned != egui::Vec2::ZERO {
        return (egui::MouseWheelUnit::Line, turned);
    }
    (
        egui::MouseWheelUnit::Point,
        egui::vec2(-horizontal.absolute as f32, -vertical.absolute as f32),
    )
}

impl KeyboardHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        wl_surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if let Some(index) = self.index_of(wl_surface) {
            let surface = &mut self.surfaces[index];
            self.keyboard_on = Some(surface.id);
            surface.focused = true;
            surface.access.adapter.update_window_focus_state(true);
            surface.events.push(egui::Event::WindowFocused(true));
            surface.redraw = true;
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        wl_surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(index) = self.index_of(wl_surface) {
            let surface = &mut self.surfaces[index];
            if self.keyboard_on == Some(surface.id) {
                self.keyboard_on = None;
            }
            surface.focused = false;
            surface.access.adapter.update_window_focus_state(false);
            surface.events.push(egui::Event::WindowFocused(false));
            surface.redraw = true;
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(&event, true);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(&event, false);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        layout: u32,
    ) {
        // Another layout: whoever shows it draws again.
        if layout != self.control.layout {
            self.control.layout = layout;
            for surface in &mut self.surfaces {
                surface.redraw = true;
            }
        }
        self.modifiers = egui::Modifiers {
            alt: modifiers.alt,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            mac_cmd: false,
            command: modifiers.ctrl,
        };
        let now = self.modifiers;
        let on = self.keyboard_on;
        if let Some(surface) = self.by_id(on) {
            surface.events.push(egui::Event::ModifiersChanged(now));
        }
    }
}

impl Host {
    fn key(&mut self, event: &KeyEvent, pressed: bool) {
        let modifiers = self.modifiers;
        let on = self.keyboard_on;
        let Some(surface) = self.by_id(on) else {
            return;
        };
        if let Some(key) = key_of(event.keysym) {
            surface.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        if pressed && !modifiers.ctrl && !modifiers.alt {
            if let Some(text) = event
                .utf8
                .as_ref()
                .filter(|text| text.chars().all(|letter| !letter.is_control()))
            {
                if !text.is_empty() {
                    surface.events.push(egui::Event::Text(text.clone()));
                }
            }
        }
        surface.redraw = true;
    }
}

/// The toolkit's name for a key, for the keys a shell surface acts on.
pub fn key_of(keysym: Keysym) -> Option<egui::Key> {
    use egui::Key;
    Some(match keysym {
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Escape => Key::Escape,
        Keysym::BackSpace => Key::Backspace,
        Keysym::Tab | Keysym::ISO_Left_Tab => Key::Tab,
        Keysym::Delete => Key::Delete,
        Keysym::Home => Key::Home,
        Keysym::End => Key::End,
        Keysym::Left => Key::ArrowLeft,
        Keysym::Right => Key::ArrowRight,
        Keysym::Up => Key::ArrowUp,
        Keysym::Down => Key::ArrowDown,
        Keysym::Page_Up => Key::PageUp,
        Keysym::Page_Down => Key::PageDown,
        Keysym::space => Key::Space,
        other => {
            // Letters and digits by their own names.
            let name = other.name()?.trim_start_matches("XK_").to_string();
            let mut letters = name.chars();
            match (letters.next(), letters.next()) {
                (Some(letter), None) => Key::from_name(&letter.to_ascii_uppercase().to_string())?,
                _ => return None,
            }
        }
    })
}

impl wayland_client::Dispatch<WpFractionalScaleV1, wl_surface::WlSurface> for Host {
    fn event(
        host: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        wl_surface: &wl_surface::WlSurface,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The scale of the display the surface is on, in 120ths.
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let Some(index) = host.index_of(wl_surface) else {
            return;
        };
        let scale = (scale as f32 / 120.0).max(0.25);
        let surface = &mut host.surfaces[index];
        if (surface.scale - scale).abs() < 0.001 {
            return;
        }
        log::debug!(
            "surface {:?} is drawn at {scale} pixels to a point",
            surface.id
        );
        surface.scale = scale;
        wl_surface.set_buffer_scale(1);
        if let Some(gpu) = &host.gpu {
            surface.configure_target(&gpu.device);
        }
        surface.redraw = true;
    }
}

macro_rules! unheard {
    ($interface:ty) => {
        impl wayland_client::Dispatch<$interface, ()> for Host {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
unheard!(WpFractionalScaleManagerV1);
unheard!(WpViewporter);
unheard!(WpViewport);

/// A surface as screen readers and other aids know it (AT-SPI, through
/// AccessKit). The toolkit describes what it draws as a tree of things
/// with names and kinds; that is told to whoever listens, and what a
/// listener asks to be done (a button pressed) is given to the toolkit
/// as input. Nothing is described until somebody listens.
struct Access {
    adapter: accesskit_unix::Adapter,
    shared: std::sync::Arc<AccessShared>,
    /// The toolkit was told to describe this surface.
    described: bool,
}

/// What the thread that speaks with the aids leaves for the loop.
struct AccessShared {
    /// Somebody listens.
    wanted: std::sync::atomic::AtomicBool,
    /// What was asked to be done.
    asked: std::sync::Mutex<Vec<egui::accesskit::ActionRequest>>,
    wake: smithay_client_toolkit::reexports::calloop::ping::Ping,
}

struct AccessHears(std::sync::Arc<AccessShared>);

impl egui::accesskit::ActivationHandler for AccessHears {
    fn request_initial_tree(&mut self) -> Option<egui::accesskit::TreeUpdate> {
        // (The tree comes with the next frame, which this asks for.)
        self.0
            .wanted
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.0.wake.ping();
        None
    }
}

impl egui::accesskit::ActionHandler for AccessHears {
    fn do_action(&mut self, request: egui::accesskit::ActionRequest) {
        self.0
            .asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.0.wake.ping();
    }
}

impl egui::accesskit::DeactivationHandler for AccessHears {
    fn deactivate_accessibility(&mut self) {
        self.0
            .wanted
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Access {
    fn new(wake: smithay_client_toolkit::reexports::calloop::ping::Ping) -> Self {
        let shared = std::sync::Arc::new(AccessShared {
            wanted: std::sync::atomic::AtomicBool::new(false),
            asked: std::sync::Mutex::new(Vec::new()),
            wake,
        });
        let hears = || AccessHears(std::sync::Arc::clone(&shared));
        Self {
            adapter: accesskit_unix::Adapter::new(hears(), hears(), hears()),
            shared,
            described: false,
        }
    }
}

impl Host {
    /// What screen readers asked since the last pass: a surface that is
    /// listened to is described from its next frame on, and what was
    /// asked to be done is input for it.
    fn access_news(&mut self) {
        for surface in &mut self.surfaces {
            let shared = &surface.access.shared;
            if shared.wanted.load(std::sync::atomic::Ordering::Relaxed) && !surface.access.described
            {
                surface.context.enable_accesskit();
                surface.access.described = true;
                surface.redraw = true;
            }
            let asked = std::mem::take(
                &mut *shared
                    .asked
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
            for request in asked {
                surface
                    .events
                    .push(egui::Event::AccessKitActionRequest(request));
                surface.redraw = true;
            }
        }
    }
}

/// What the input method knows of the surfaces' text fields.
#[derive(Default)]
struct TextInput {
    /// The surface the input method may put text into (the one with the
    /// keyboard, as the compositor says it).
    on: Option<SurfaceId>,
    /// What was last said of a text field there: its text cursor's
    /// rectangle and whether it is a password's; `None`: no field.
    said: Option<([i32; 4], bool)>,
    /// What the input method sent since its last `done`: the text that
    /// is being made, and the text that is to be put in.
    making: Option<String>,
    put: Option<String>,
}

impl TextInput {
    /// Say what there is now to the input method, when it is another
    /// than was said last.
    fn tell(&mut self, field: &ZwpTextInputV3, wanted: Option<([i32; 4], bool)>) {
        if wanted == self.said {
            return;
        }
        match wanted {
            Some((cursor, password)) => {
                // (A field that was not there before is switched on; one
                // that is there has only its cursor moved.)
                if self.said.is_none_or(|(_, was)| was != password) {
                    field.enable();
                    // No input method for a password: its letters are
                    // not to be seen, kept or guessed.
                    let (hint, purpose) = if password {
                        (
                            ContentHint::SensitiveData | ContentHint::HiddenText,
                            ContentPurpose::Password,
                        )
                    } else {
                        (ContentHint::None, ContentPurpose::Normal)
                    };
                    field.set_content_type(hint, purpose);
                }
                field.set_cursor_rectangle(cursor[0], cursor[1], cursor[2], cursor[3]);
            }
            None => field.disable(),
        }
        field.commit();
        self.said = wanted;
    }
}

impl wayland_client::Dispatch<ZwpTextInputV3, ()> for Host {
    fn event(
        host: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                host.text.on = host.index_of(&surface).map(|index| host.surfaces[index].id);
                // (Its field is told at the next frame.)
                host.text.said = None;
                if let Some(index) = host.index_of(&surface) {
                    host.surfaces[index].redraw = true;
                }
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                host.text = TextInput::default();
            }
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                host.text.making = Some(text.unwrap_or_default());
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                host.text.put = Some(text.unwrap_or_default());
            }
            // What was sent holds from here: first the text that is put
            // in, then the one that is still being made (none: what was
            // being made is done with).
            zwp_text_input_v3::Event::Done { .. } => {
                let put = host.text.put.take();
                let making = host.text.making.take();
                let on = host.text.on;
                if let Some(surface) = host.by_id(on) {
                    if let Some(text) = put.filter(|text| !text.is_empty()) {
                        surface
                            .events
                            .push(egui::Event::Ime(egui::ImeEvent::Commit(text)));
                    }
                    surface
                        .events
                        .push(egui::Event::Ime(egui::ImeEvent::Preedit {
                            text: making.unwrap_or_default(),
                            active_range_chars: None,
                        }));
                    surface.redraw = true;
                }
            }
            _ => {}
        }
    }
}

impl wayland_client::Dispatch<ZwpTextInputManagerV3, ()> for Host {
    fn event(
        _: &mut Self,
        _: &ZwpTextInputManagerV3,
        _: zwp_text_input_manager_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(Host);
delegate_output!(Host);
delegate_seat!(Host);
delegate_keyboard!(Host);
delegate_pointer!(Host);
smithay_client_toolkit::delegate_touch!(Host);

impl Host {
    /// Tell a surface of a finger: as a touch, and, when it is the
    /// first finger down, as the pointer.
    fn finger(&mut self, id: i32, phase: egui::TouchPhase) {
        let Some(place) = self.fingers.iter().position(|(finger, ..)| *finger == id) else {
            return;
        };
        let first = place == 0;
        let (_, on, at) = self.fingers[place];
        let modifiers = self.modifiers;
        if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
            self.fingers.remove(place);
        }
        let Some(surface) = self.by_id(Some(on)) else {
            return;
        };
        surface.events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(id as u64),
            phase,
            pos: at,
            force: None,
        });
        if first {
            match phase {
                egui::TouchPhase::Start => {
                    surface.events.push(egui::Event::PointerMoved(at));
                    surface.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    });
                }
                egui::TouchPhase::Move => surface.events.push(egui::Event::PointerMoved(at)),
                egui::TouchPhase::End => {
                    surface.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    });
                    // (No finger: nothing is pointed at any more.)
                    surface.events.push(egui::Event::PointerGone);
                }
                egui::TouchPhase::Cancel => surface.events.push(egui::Event::PointerGone),
            }
        }
        surface.redraw = true;
    }
}

impl TouchHandler for Host {
    fn down(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
        serial: u32,
        _: u32,
        wl_surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let Some(index) = self.index_of(&wl_surface) else {
            return;
        };
        // (What lets a menu take the pointer, as a button's press does.)
        self.last_press = Some(serial);
        let on = self.surfaces[index].id;
        self.fingers.retain(|(finger, ..)| *finger != id);
        self.fingers
            .push((id, on, egui::pos2(position.0 as f32, position.1 as f32)));
        self.finger(id, egui::TouchPhase::Start);
    }

    fn up(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
        _: u32,
        _: u32,
        id: i32,
    ) {
        self.finger(id, egui::TouchPhase::End);
    }

    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
        _: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some(finger) = self.fingers.iter_mut().find(|(finger, ..)| *finger == id) {
            finger.2 = egui::pos2(position.0 as f32, position.1 as f32);
        }
        self.finger(id, egui::TouchPhase::Move);
    }

    fn shape(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
        _: i32,
        _: f64,
        _: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
        _: i32,
        _: f64,
    ) {
    }

    fn cancel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_touch::WlTouch,
    ) {
        let down: Vec<i32> = self.fingers.iter().map(|(finger, ..)| *finger).collect();
        for id in down {
            self.finger(id, egui::TouchPhase::Cancel);
        }
    }
}
delegate_layer!(Host);
smithay_client_toolkit::delegate_xdg_shell!(Host);
smithay_client_toolkit::delegate_xdg_popup!(Host);
smithay_client_toolkit::delegate_session_lock!(Host);
delegate_registry!(Host);

impl ProvidesRegistryState for Host {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_is_counted_in_notches_and_a_touchpad_in_points() {
        let still = AxisScroll::default();
        // A mouse's wheel, one notch towards the user: fifteen units and
        // the notch.
        let notch = AxisScroll {
            absolute: 15.0,
            discrete: 1,
            stop: false,
        };
        assert_eq!(
            wheel(&still, &notch),
            (egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0))
        );
        let away = AxisScroll {
            absolute: -30.0,
            discrete: -2,
            stop: false,
        };
        assert_eq!(
            wheel(&away, &still),
            (egui::MouseWheelUnit::Line, egui::vec2(2.0, 0.0))
        );
        // Two fingers on a touchpad: no notches, the way in points.
        let fingers = AxisScroll {
            absolute: 3.25,
            discrete: 0,
            stop: false,
        };
        assert_eq!(
            wheel(&still, &fingers),
            (egui::MouseWheelUnit::Point, egui::vec2(0.0, -3.25))
        );
    }

    #[test]
    fn keys_a_shell_surface_acts_on_have_the_toolkits_names() {
        assert_eq!(key_of(Keysym::Return), Some(egui::Key::Enter));
        assert_eq!(key_of(Keysym::Escape), Some(egui::Key::Escape));
        assert_eq!(key_of(Keysym::a), Some(egui::Key::A));
        assert_eq!(key_of(Keysym::A), Some(egui::Key::A));
        assert_eq!(key_of(Keysym::_7), Some(egui::Key::Num7));
        assert_eq!(key_of(Keysym::Down), Some(egui::Key::ArrowDown));
        assert_eq!(key_of(Keysym::Shift_L), None);
    }
}
