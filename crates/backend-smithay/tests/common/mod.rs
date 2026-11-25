//! A headless compositor with real Wayland clients in the same process:
//! the clients speak the protocol over a socket pair, attach real buffers
//! and answer configures, and nothing is drawn.
#![allow(dead_code)]

use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::{Rect, WindowHandle};
use gevrek_model::config::Config;
use gevrek_smithay::{ClientState, State};
use smithay::reexports::wayland_server::Display;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

pub const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

pub struct Harness {
    pub display: Display<State>,
    pub state: State,
    pub output: MonitorHandle,
    pub clients: Vec<TestClient>,
}

impl Harness {
    /// A compositor with one 1920x1080 output, started with `config`.
    pub fn new(config: Config) -> Self {
        let display: Display<State> = Display::new().expect("display");
        let mut state = State::new(display.handle(), config);
        let output = state.add_output("TEST-1", SCREEN);
        state.start().expect("start");
        Self {
            display,
            state,
            output,
            clients: Vec::new(),
        }
    }

    /// Connect a client and wait until it has bound the globals.
    pub fn connect(&mut self) -> usize {
        let (server, client) = UnixStream::pair().expect("socket pair");
        client.set_nonblocking(true).expect("nonblocking");
        self.display
            .handle()
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("client inserted");
        let connection = Connection::from_socket(client).expect("connection");
        let queue: EventQueue<ClientApp> = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        self.clients.push(TestClient {
            connection,
            queue,
            app: ClientApp::default(),
        });
        self.pump();
        let index = self.clients.len() - 1;
        assert!(
            self.clients[index].app.ready(),
            "the client bound wl_compositor, wl_shm and xdg_wm_base"
        );
        index
    }

    /// A client asks to lock the session. What it was told: locked, or
    /// finished (refused).
    pub fn lock(&mut self, client: usize) -> Option<bool> {
        let handle = self.clients[client].queue.handle();
        let app = &mut self.clients[client].app;
        app.lock_told = None;
        app.lock = Some(
            app.lock_manager
                .as_ref()
                .expect("the lock manager")
                .lock(&handle, ()),
        );
        self.pump();
        self.clients[client].app.lock_told
    }

    /// The client that locked unlocks.
    pub fn unlock(&mut self, client: usize) {
        if let Some(lock) = self.clients[client].app.lock.take() {
            lock.unlock_and_destroy();
        }
        self.pump();
    }

    /// Let the clients and the compositor talk until nothing is left to say.
    pub fn pump(&mut self) {
        for _ in 0..12 {
            for client in &mut self.clients {
                client.queue.flush().expect("client flush");
            }
            self.display
                .dispatch_clients(&mut self.state)
                .expect("dispatch clients");
            self.display.flush_clients().expect("flush clients");
            for client in &mut self.clients {
                if let Some(guard) = client.connection.prepare_read() {
                    // Nothing to read is fine.
                    let _ = guard.read();
                }
                client
                    .queue
                    .dispatch_pending(&mut client.app)
                    .expect("client dispatch");
            }
        }
    }

    /// Open a window from a client and wait until the WM has placed it.
    /// Returns the client-side index of the window and its id in the WM.
    pub fn open(&mut self, client: usize, title: &str, app_id: &str) -> (usize, WindowHandle) {
        let before: Vec<WindowHandle> = self.state.desktop.windows.keys().copied().collect();
        let window = {
            let client = &mut self.clients[client];
            let handle = client.queue.handle();
            client.app.open(&handle, title, app_id)
        };
        self.pump();
        let id = self
            .state
            .desktop
            .windows
            .keys()
            .copied()
            .find(|id| !before.contains(id))
            .expect("the compositor tracks the new toplevel");
        (window, id)
    }

    /// Open a window that belongs to another window of the same client
    /// (its dialog: a toplevel with that one as its parent).
    pub fn open_dialog(
        &mut self,
        client: usize,
        title: &str,
        app_id: &str,
        parent: usize,
    ) -> (usize, WindowHandle) {
        let before: Vec<WindowHandle> = self.state.desktop.windows.keys().copied().collect();
        let window = {
            let client = &mut self.clients[client];
            let handle = client.queue.handle();
            client
                .app
                .open_with_parent(&handle, title, app_id, Some(parent))
        };
        self.pump();
        let id = self
            .state
            .desktop
            .windows
            .keys()
            .copied()
            .find(|id| !before.contains(id))
            .expect("the compositor tracks the new toplevel");
        (window, id)
    }

    pub fn rect(&self, id: WindowHandle) -> Rect {
        self.state.desktop.windows[&id].rect
    }

    pub fn shown(&self, id: WindowHandle) -> bool {
        self.state.desktop.shown(id)
    }

    /// Press and release keys in order, as a keyboard would: `codes` are
    /// evdev key codes; all are held, then released in reverse.
    pub fn chord(&mut self, codes: &[u32]) {
        use smithay::backend::input::KeyState;
        for code in codes {
            self.state
                .key_event((code + 8).into(), KeyState::Pressed, 0);
        }
        for code in codes.iter().rev() {
            self.state
                .key_event((code + 8).into(), KeyState::Released, 0);
        }
        self.pump();
        self.finish_transitions();
    }

    /// Let every transition in flight run to its end (a window that is
    /// closed is asked to close when its exit transition is over).
    pub fn finish_transitions(&mut self) {
        self.state
            .desktop
            .tick_animations(std::time::Instant::now() + std::time::Duration::from_secs(5));
        self.pump();
    }
}

pub struct TestClient {
    pub connection: Connection,
    pub queue: EventQueue<ClientApp>,
    pub app: ClientApp,
}

/// A bar (a layer surface) as its client sees it.
pub struct ClientBar {
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub height: i32,
    pub configured: Option<(u32, u32)>,
}

/// One toplevel as its client sees it.
pub struct ClientWindow {
    pub surface: wl_surface::WlSurface,
    pub xdg_surface: xdg_surface::XdgSurface,
    pub toplevel: xdg_toplevel::XdgToplevel,
    /// The size of the last toplevel configure (0 = the client chooses).
    pub pending: (i32, i32),
    pub activated: bool,
    /// The size of every buffer the client attached, in order.
    pub committed: Vec<(i32, i32)>,
    /// The compositor asked the window to close.
    pub close_requested: bool,
    /// The client never gets narrower than this.
    pub min_width: i32,
}

#[derive(Default)]
pub struct ClientApp {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,
    pub wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub seat: Option<wl_seat::WlSeat>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    /// The lock this client asked for, and what it was told of it:
    /// `Some(true)` locked, `Some(false)` finished (refused, or over).
    pub lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    pub lock_told: Option<bool>,
    /// Every global the compositor announced, by interface name.
    pub globals: Vec<String>,
    /// The bars this client put up: surface, layer surface, height, and
    /// the size of the last configure.
    pub bars: Vec<ClientBar>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The keys this client was sent: evdev code and whether it went down.
    pub keys: Vec<(u32, bool)>,
    pub windows: Vec<ClientWindow>,
    buffers: usize,
}

impl ClientApp {
    fn ready(&self) -> bool {
        self.compositor.is_some() && self.shm.is_some() && self.wm_base.is_some()
    }

    /// Create a toplevel and make the first, bufferless commit.
    fn open(&mut self, handle: &QueueHandle<Self>, title: &str, app_id: &str) -> usize {
        self.open_with_parent(handle, title, app_id, None)
    }

    /// As `open`; with `parent`, the toplevel belongs to that window of
    /// this client (it says so before it is first shown, as a dialog does).
    fn open_with_parent(
        &mut self,
        handle: &QueueHandle<Self>,
        title: &str,
        app_id: &str,
        parent: Option<usize>,
    ) -> usize {
        let index = self.windows.len();
        let surface = self
            .compositor
            .as_ref()
            .expect("compositor")
            .create_surface(handle, ());
        let xdg_surface = self
            .wm_base
            .as_ref()
            .expect("wm base")
            .get_xdg_surface(&surface, handle, index);
        let toplevel = xdg_surface.get_toplevel(handle, index);
        toplevel.set_title(title.to_string());
        toplevel.set_app_id(app_id.to_string());
        if let Some(parent) = parent {
            toplevel.set_parent(Some(&self.windows[parent].toplevel));
        }
        surface.commit();
        self.windows.push(ClientWindow {
            surface,
            xdg_surface,
            toplevel,
            pending: (0, 0),
            activated: false,
            committed: Vec::new(),
            close_requested: false,
            min_width: 0,
        });
        index
    }

    /// Attach a buffer of the configured size (or of the client's own
    /// choice when the compositor left it open) and commit.
    fn draw(&mut self, handle: &QueueHandle<Self>, index: usize) {
        let (width, height) = {
            let window = &self.windows[index];
            let (width, height) = window.pending;
            let width = if width > 0 { width } else { 640 };
            let height = if height > 0 { height } else { 480 };
            (width.max(window.min_width), height)
        };
        let buffer = self.buffer(handle, width, height);
        let window = &mut self.windows[index];
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        window.committed.push((width, height));
    }

    /// A shared-memory buffer of this size.
    fn buffer(
        &mut self,
        handle: &QueueHandle<Self>,
        width: i32,
        height: i32,
    ) -> wl_buffer::WlBuffer {
        self.buffers += 1;
        // (Numbered across the process: the tests run side by side, and
        // two of them must not open the same file.)
        static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = format!(
            "/dev/shm/gevrek-test-{}-{}",
            std::process::id(),
            FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("shm file");
        let _ = std::fs::remove_file(&path);
        let bytes = width * height * 4;
        file.set_len(bytes as u64).expect("shm size");
        let pool = self
            .shm
            .as_ref()
            .expect("shm")
            .create_pool(file.as_fd(), bytes, handle, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Argb8888,
            handle,
            (),
        );
        pool.destroy();
        buffer
    }

    /// Put a bar at the top of the output that reserves its height.
    pub fn open_bar(&mut self, handle: &QueueHandle<Self>, height: i32) -> usize {
        let index = self.bars.len();
        let surface = self
            .compositor
            .as_ref()
            .expect("compositor")
            .create_surface(handle, ());
        let layer_surface = self
            .layer_shell
            .as_ref()
            .expect("layer shell")
            .get_layer_surface(
                &surface,
                None,
                zwlr_layer_shell_v1::Layer::Top,
                "test-bar".to_string(),
                handle,
                index,
            );
        use zwlr_layer_surface_v1::Anchor;
        layer_surface.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right);
        layer_surface.set_size(0, height as u32);
        layer_surface.set_exclusive_zone(height);
        surface.commit();
        self.bars.push(ClientBar {
            surface,
            layer_surface,
            height,
            configured: None,
        });
        index
    }

    /// Take a bar down.
    pub fn close_bar(&mut self, index: usize) {
        let bar = &self.bars[index];
        bar.layer_surface.destroy();
        bar.surface.destroy();
    }

    /// Close a window from the client side.
    pub fn close(&mut self, index: usize) {
        let window = &self.windows[index];
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClientApp {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            state.globals.push(interface.clone());
            match interface.as_str() {
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), handle, ()));
                }
                "ext_session_lock_manager_v1" => {
                    state.lock_manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), handle, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, version.min(2), handle, ()));
                }
                "wl_seat" => {
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(5), handle, ());
                    state.keyboard = Some(seat.get_keyboard(handle, ()));
                    state.seat = Some(seat);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for ClientApp {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: ext_session_lock_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for ClientApp {
    fn event(
        state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.lock_told = Some(true),
            ext_session_lock_v1::Event::Finished => state.lock_told = Some(false),
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientApp {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for ClientApp {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.draw(handle, *index);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, usize> for ClientApp {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let window = &mut state.windows[*index];
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                window.pending = (width, height);
                window.activated = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    .any(|value| {
                        WEnum::<xdg_toplevel::State>::from(value)
                            == WEnum::Value(xdg_toplevel::State::Activated)
                    });
            }
            xdg_toplevel::Event::Close => window.close_requested = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for ClientApp {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: key_state,
            ..
        } = event
        {
            state.keys.push((
                key,
                key_state == WEnum::Value(wl_keyboard::KeyState::Pressed),
            ));
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize> for ClientApp {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &usize,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            state.bars[*index].configured = Some((width, height));
            let surface = state.bars[*index].surface.clone();
            let buffer = state.buffer(handle, width.max(1) as i32, height.max(1) as i32);
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
        }
    }
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {
        $(
            impl Dispatch<$interface, ()> for ClientApp {
                fn event(
                    _: &mut Self,
                    _: &$interface,
                    _: <$interface as wayland_client::Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface,
    wl_seat::WlSeat,
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
);
