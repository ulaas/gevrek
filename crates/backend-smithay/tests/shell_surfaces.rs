//! What the compositor does for a shell's surfaces that lie on the
//! desktop and are raised over the windows (the zones): another layer
//! while they are shown, the desktop soft behind parts of them, drags
//! that reach them, and pictures of the screen that leave them out.
//!
//! A real client in the test's own process speaks the protocols; the
//! pictures are drawn by the renderer in memory.
#![cfg(target_os = "linux")]

mod common;

use common::Harness;
use gevrek_model::config::Config;
use gevrek_smithay::{ClientState, State};
use smithay::backend::allocator::Fourcc;
use smithay::backend::input::KeyState;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::reexports::pixman;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

type DndAction = wl_data_device_manager::DndAction;

const LEFT: u32 = 0x110;
/// The keys as the kernel numbers them.
const CTRL: u32 = 29;
const SHIFT: u32 = 42;
/// A letter, which says nothing of what a drag does.
const LETTER: u32 = 30;

/// A layer surface as its client sees it.
struct Sheet {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    /// What it is filled with: blue, green, red, how solid.
    colour: [u8; 4],
    size: Option<(u32, u32)>,
}

#[derive(Default)]
struct App {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    effects: Option<ExtBackgroundEffectManagerV1>,
    /// What the compositor said it does behind a surface that asks.
    blurs: Option<bool>,
    data: Option<wl_data_device_manager::WlDataDeviceManager>,
    device: Option<wl_data_device::WlDataDevice>,
    screencopy: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    sheets: Vec<Sheet>,
    buffers: usize,
    /// The sheet under the pointer and the one with the keyboard.
    pointer_on: Option<usize>,
    keyboard_on: Option<usize>,
    /// The numbers of the presses told.
    presses: Vec<u32>,
    /// A drag over a sheet: which one, and how often one came and went.
    drag_on: Option<usize>,
    drag_entered: u32,
    drag_left: u32,
    dropped: bool,
    offer: Option<wl_data_offer::WlDataOffer>,
    /// What the compositor told the drag's source and its target to do.
    told_source: Vec<DndAction>,
    told_target: Vec<DndAction>,
    finished: bool,
    cancelled: bool,
    /// A picture of the display: how its buffer must be, and its end.
    frame_buffer: Option<(u32, u32, u32)>,
    frame_ready: bool,
    frame_failed: bool,
}

struct Client {
    connection: Connection,
    queue: EventQueue<App>,
    app: App,
}

fn connect(harness: &mut Harness) -> Client {
    let (server, client) = UnixStream::pair().expect("socket pair");
    client.set_nonblocking(true).expect("nonblocking");
    harness
        .display
        .handle()
        .insert_client(server, Arc::new(ClientState::default()))
        .expect("client inserted");
    let connection = Connection::from_socket(client).expect("connection");
    let queue: EventQueue<App> = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut client = Client {
        connection,
        queue,
        app: App::default(),
    };
    pump(harness, &mut client);
    assert!(client.app.layer_shell.is_some(), "the layer shell is there");
    client
}

/// Let the client and the compositor talk until nothing is left to say.
fn pump(harness: &mut Harness, client: &mut Client) {
    for _ in 0..12 {
        client.queue.flush().expect("client flush");
        harness
            .display
            .dispatch_clients(&mut harness.state)
            .expect("dispatch clients");
        harness.display.flush_clients().expect("flush clients");
        if let Some(guard) = client.connection.prepare_read() {
            let _ = guard.read();
        }
        client
            .queue
            .dispatch_pending(&mut client.app)
            .expect("client dispatch");
    }
    harness.pump();
}

impl App {
    /// A buffer of this size filled with one colour, and the file its
    /// pixels are in.
    fn buffer(
        &mut self,
        handle: &QueueHandle<Self>,
        size: (i32, i32),
        format: wl_shm::Format,
        colour: [u8; 4],
    ) -> (wl_buffer::WlBuffer, std::fs::File) {
        self.buffers += 1;
        let path = format!(
            "/dev/shm/gevrek-shell-surfaces-{}-{}",
            std::process::id(),
            self.buffers
        );
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("shm file");
        let _ = std::fs::remove_file(&path);
        let bytes = size.0 * size.1 * 4;
        if colour == [0; 4] {
            file.set_len(bytes as u64).expect("shm size");
        } else {
            file.write_all_at(&colour.repeat((size.0 * size.1) as usize), 0)
                .expect("pixels");
        }
        let pool = self
            .shm
            .as_ref()
            .expect("shm")
            .create_pool(file.as_fd(), bytes, handle, ());
        let buffer = pool.create_buffer(0, size.0, size.1, size.0 * 4, format, handle, ());
        pool.destroy();
        (buffer, file)
    }

    /// Put a surface on a layer: over the whole display (`size` of
    /// nothing), or of a size in a corner of it.
    fn sheet(
        &mut self,
        handle: &QueueHandle<Self>,
        name: &str,
        layer: Layer,
        anchor: Anchor,
        size: (u32, u32),
        colour: [u8; 4],
    ) -> usize {
        let index = self.sheets.len();
        let surface = self
            .compositor
            .as_ref()
            .expect("compositor")
            .create_surface(handle, ());
        let layer_surface = self.layer_shell.as_ref().expect("layer shell").get_layer_surface(
            &surface,
            None,
            layer,
            name.to_string(),
            handle,
            index,
        );
        layer_surface.set_anchor(anchor);
        layer_surface.set_size(size.0, size.1);
        layer_surface.set_exclusive_zone(-1);
        surface.commit();
        self.sheets.push(Sheet {
            surface,
            layer: layer_surface,
            colour,
            size: None,
        });
        index
    }

    fn sheet_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.sheets
            .iter()
            .position(|sheet| sheet.surface == *surface)
    }
}

fn everywhere() -> Anchor {
    Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_compositor" => {
                app.compositor = Some(registry.bind(name, version.min(5), handle, ()));
            }
            "wl_shm" => app.shm = Some(registry.bind(name, 1, handle, ())),
            "wl_output" => app.output = Some(registry.bind(name, version.min(2), handle, ())),
            "wl_seat" => {
                let seat: wl_seat::WlSeat = registry.bind(name, version.min(5), handle, ());
                seat.get_pointer(handle, ());
                seat.get_keyboard(handle, ());
                app.seat = Some(seat);
            }
            "zwlr_layer_shell_v1" => {
                app.layer_shell = Some(registry.bind(name, version.min(4), handle, ()));
            }
            "ext_background_effect_manager_v1" => {
                app.effects = Some(registry.bind(name, 1, handle, ()));
            }
            "wl_data_device_manager" => {
                app.data = Some(registry.bind(name, version.min(3), handle, ()));
            }
            "zwlr_screencopy_manager_v1" => {
                app.screencopy = Some(registry.bind(name, version.min(3), handle, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize> for App {
    fn event(
        app: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
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
            layer.ack_configure(serial);
            let size = (width.max(1), height.max(1));
            let colour = app.sheets[*index].colour;
            let (buffer, _) = app.buffer(
                handle,
                (size.0 as i32, size.1 as i32),
                wl_shm::Format::Argb8888,
                colour,
            );
            let sheet = &mut app.sheets[*index];
            sheet.size = Some(size);
            sheet.surface.attach(Some(&buffer), 0, 0);
            sheet.surface.commit();
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => app.pointer_on = app.sheet_of(&surface),
            wl_pointer::Event::Leave { .. } => app.pointer_on = None,
            wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => app.presses.push(serial),
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => app.keyboard_on = app.sheet_of(&surface),
            wl_keyboard::Event::Leave { .. } => app.keyboard_on = None,
            _ => {}
        }
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            app.blurs = Some(matches!(
                flags,
                WEnum::Value(flags)
                    if flags.contains(ext_background_effect_manager_v1::Capability::Blur)
            ));
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id,
                ..
            } => {
                app.drag_on = app.sheet_of(&surface);
                app.drag_entered += 1;
                // It is taken, as a move by choice, a copy if need be.
                if let Some(offer) = &id {
                    offer.accept(serial, Some("text/uri-list".to_string()));
                    offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Move);
                }
                app.offer = id;
            }
            wl_data_device::Event::Leave => {
                app.drag_on = None;
                app.drag_left += 1;
            }
            wl_data_device::Event::Drop => {
                app.dropped = true;
                if let Some(offer) = app.offer.take() {
                    offer.finish();
                    offer.destroy();
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Action {
            dnd_action: WEnum::Value(action),
        } = event
        {
            app.told_target.push(action);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Action {
                dnd_action: WEnum::Value(action),
            } => app.told_source.push(action),
            wl_data_source::Event::DndFinished => app.finished = true,
            wl_data_source::Event::Cancelled => app.cancelled = true,
            _ => {}
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                width,
                height,
                stride,
                ..
            } => app.frame_buffer = Some((width, height, stride)),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => app.frame_ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => app.frame_failed = true,
            _ => {}
        }
    }
}

macro_rules! unheard {
    ($($interface:ty),* $(,)?) => {
        $(
            impl Dispatch<$interface, ()> for App {
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
        )*
    };
}

unheard!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface,
    wl_region::WlRegion,
    wl_seat::WlSeat,
    wl_output::WlOutput,
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    ExtBackgroundEffectSurfaceV1,
    wl_data_device_manager::WlDataDeviceManager,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
);

/// Whether a window of the desktop is what the pointer would find at a
/// place, and whether it would find anything.
fn at(state: &State, place: (f64, f64)) -> (bool, bool) {
    let (window, under) = state.surface_under(place.into());
    (window.is_some(), under.is_some())
}

#[test]
fn a_surface_goes_over_the_windows_and_back_while_it_is_shown() {
    let mut harness = Harness::new(Config::new());
    let windows = harness.connect();
    let (window, id) = harness.open(windows, "editor", "org.test.editor");
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();
    // The desktop's own surface: all of the display, below the windows.
    let desk = client.app.sheet(
        &handle,
        "test-zones",
        Layer::Bottom,
        everywhere(),
        (0, 0),
        [40, 40, 40, 255],
    );
    client.app.sheets[desk]
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(client.app.sheets[desk].size, Some((1920, 1080)));

    // The pointer rests in the middle of the window.
    let rect = harness.rect(id);
    let middle = (
        f64::from(rect.x + rect.width / 2),
        f64::from(rect.y + rect.height / 2),
    );
    harness.state.pointer_moved(middle.into(), 0);
    pump(&mut harness, &mut client);
    assert_eq!(at(&harness.state, middle), (true, true), "the window is there");
    assert_eq!(client.app.pointer_on, None);
    assert!(harness.clients[windows].app.windows[window].activated);

    // Raised over the windows, with the keyboard, as for a look at it.
    let sheet = &client.app.sheets[desk];
    sheet.layer.set_layer(Layer::Overlay);
    sheet
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    sheet.surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(
        at(&harness.state, middle),
        (false, true),
        "the surface is over the window"
    );
    // The pointer has not moved, and is on the surface now.
    assert_eq!(client.app.pointer_on, Some(desk));
    assert_eq!(client.app.keyboard_on, Some(desk));
    // (The room for the windows is what it was: the surface keeps none.)
    assert_eq!(harness.rect(id), rect);

    // And put back: the window has the pointer and the keyboard again.
    let sheet = &client.app.sheets[desk];
    sheet.layer.set_layer(Layer::Bottom);
    sheet
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    sheet.surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(at(&harness.state, middle), (true, true));
    assert_eq!(client.app.pointer_on, None);
    assert_eq!(client.app.keyboard_on, None);
    assert!(harness.clients[windows].app.windows[window].activated);
    // (Its place and its size never changed.)
    assert_eq!(client.app.sheets[desk].size, Some((1920, 1080)));
}

#[test]
fn a_surface_given_the_keyboard_by_a_press_keeps_it_on_another_layer() {
    let mut harness = Harness::new(Config::new());
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();
    let desk = client.app.sheet(
        &handle,
        "test-zones",
        Layer::Bottom,
        everywhere(),
        (0, 0),
        [40, 40, 40, 255],
    );
    client.app.sheets[desk]
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    harness.state.pointer_moved((300.0, 300.0).into(), 0);
    harness.state.pointer_button(LEFT, true, 0);
    harness.state.pointer_button(LEFT, false, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.keyboard_on, Some(desk), "pressed: it has the keyboard");

    client.app.sheets[desk].layer.set_layer(Layer::Overlay);
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(client.app.keyboard_on, Some(desk));

    // It takes no keyboard any more: the keyboard is not left with it.
    client.app.sheets[desk]
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::None);
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(client.app.keyboard_on, None);
}

#[test]
fn a_drag_reaches_a_surface_over_the_windows_and_the_keys_say_what_it_does() {
    let mut harness = Harness::new(Config::new());
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();
    // The desktop's surface, raised over everything: where an icon of a
    // zone is pressed and dragged, and where it is let go.
    let desk = client.app.sheet(
        &handle,
        "test-zones",
        Layer::Bottom,
        everywhere(),
        (0, 0),
        [40, 40, 40, 255],
    );
    pump(&mut harness, &mut client);
    client.app.sheets[desk].layer.set_layer(Layer::Overlay);
    client.app.sheets[desk].surface.commit();
    let device = client.app.data.as_ref().expect("data devices").get_data_device(
        client.app.seat.as_ref().expect("a seat"),
        &handle,
        (),
    );
    client.app.device = Some(device);
    pump(&mut harness, &mut client);

    harness.state.pointer_moved((100.0, 100.0).into(), 0);
    harness.state.pointer_button(LEFT, true, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.pointer_on, Some(desk));
    let press = *client.app.presses.last().expect("the press was told");

    // The drag: files, as a copy or a move, with a picture held eight
    // across and six down from its corner.
    let source = client
        .app
        .data
        .as_ref()
        .expect("data devices")
        .create_data_source(&handle, ());
    source.offer("text/uri-list".to_string());
    source.set_actions(DndAction::Copy | DndAction::Move);
    let picture = client
        .app
        .compositor
        .as_ref()
        .expect("compositor")
        .create_surface(&handle, ());
    client.app.device.as_ref().expect("a device").start_drag(
        Some(&source),
        &client.app.sheets[desk].surface,
        Some(&picture),
        press,
    );
    let (buffer, _) = client
        .app
        .buffer(&handle, (32, 32), wl_shm::Format::Argb8888, [0, 0, 200, 255]);
    picture.attach(Some(&buffer), 0, 0);
    picture.offset(-8, -6);
    picture.commit();
    pump(&mut harness, &mut client);

    harness.state.pointer_moved((900.0, 500.0).into(), 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.drag_on, Some(desk), "the drag is over the surface");
    assert_eq!(client.app.drag_entered, 1);
    // The picture is held where its program holds it.
    assert_eq!(harness.state.drag_picture_place(), Some((892, 494)));
    // No key: what the target likes best.
    assert_eq!(client.app.told_source.last(), Some(&DndAction::Move));
    assert_eq!(client.app.told_target.last(), Some(&DndAction::Move));

    // Ctrl, with the pointer at rest: a copy. The surface is asked
    // again by the drag leaving and coming back.
    harness
        .state
        .key_event((CTRL + 8).into(), KeyState::Pressed, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.drag_entered, 2);
    assert_eq!(client.app.drag_left, 1);
    assert_eq!(client.app.drag_on, Some(desk));
    assert_eq!(client.app.told_source.last(), Some(&DndAction::Copy));
    assert_eq!(client.app.told_target.last(), Some(&DndAction::Copy));

    // Ctrl let go, Shift held: a move.
    harness
        .state
        .key_event((CTRL + 8).into(), KeyState::Released, 0);
    harness
        .state
        .key_event((SHIFT + 8).into(), KeyState::Pressed, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.told_source.last(), Some(&DndAction::Move));
    let entered = client.app.drag_entered;
    // A key that says nothing new asks nothing again.
    harness.state.key_event((LETTER + 8).into(), KeyState::Pressed, 0);
    harness.state.key_event((LETTER + 8).into(), KeyState::Released, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.drag_entered, entered);
    harness
        .state
        .key_event((SHIFT + 8).into(), KeyState::Released, 0);
    pump(&mut harness, &mut client);

    // Let go: dropped on the surface, and the source is told it is done.
    harness.state.pointer_button(LEFT, false, 0);
    pump(&mut harness, &mut client);
    assert!(client.app.dropped, "the surface was told of the drop");
    assert!(client.app.finished, "the source was told it is done");
    assert!(!client.app.cancelled);
    assert_eq!(harness.state.drag_picture_place(), None);
    // (Keys after the drag ask nothing of anybody.)
    let entered = client.app.drag_entered;
    harness
        .state
        .key_event((CTRL + 8).into(), KeyState::Pressed, 0);
    harness
        .state
        .key_event((CTRL + 8).into(), KeyState::Released, 0);
    pump(&mut harness, &mut client);
    assert_eq!(client.app.drag_entered, entered);
}

/// How many of what is drawn on the display are pieces of its soft
/// picture kept in memory.
fn soft_pieces(harness: &mut Harness, renderer: &mut PixmanRenderer) -> usize {
    harness
        .state
        .render_elements(renderer, harness.output)
        .iter()
        .filter(|element| matches!(element, gevrek_smithay::render::OutputElement::Memory(_)))
        .count()
}

#[test]
fn the_desktop_is_soft_behind_the_parts_a_surface_names() {
    let mut harness = Harness::new(Config::new());
    let mut renderer = PixmanRenderer::new().expect("a renderer in memory");
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();
    assert_eq!(client.app.blurs, Some(true), "the compositor says it blurs");
    let desk = client.app.sheet(
        &handle,
        "test-zones",
        Layer::Bottom,
        everywhere(),
        (0, 0),
        [40, 40, 40, 120],
    );
    pump(&mut harness, &mut client);
    let server_surface = harness
        .state
        .surface_under((500.0, 500.0).into())
        .1
        .expect("the surface is there")
        .0;
    let before = soft_pieces(&mut harness, &mut renderer);

    // Two grounds: one inside, one that reaches beyond the display.
    let effect = client
        .app
        .effects
        .as_ref()
        .expect("the protocol")
        .get_background_effect(&client.app.sheets[desk].surface, &handle, ());
    let region = client
        .app
        .compositor
        .as_ref()
        .expect("compositor")
        .create_region(&handle, ());
    region.add(100, 100, 300, 200);
    region.add(1800, 1000, 400, 400);
    effect.set_blur_region(Some(&region));
    region.destroy();
    pump(&mut harness, &mut client);
    // Not before the surface's next commit.
    assert!(gevrek_smithay::background_effect::blur_parts(&server_surface).is_empty());
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    let parts = gevrek_smithay::background_effect::blur_parts(&server_surface);
    assert_eq!(parts.len(), 2);
    assert_eq!(
        (parts[0].loc.x, parts[0].loc.y, parts[0].size.w, parts[0].size.h),
        (100, 100, 300, 200)
    );

    // The first drawing asks for the display's soft picture, which is
    // made after it; from the next one a piece of it lies behind each
    // part.
    let _ = soft_pieces(&mut harness, &mut renderer);
    harness
        .state
        .refresh_soft::<_, pixman::Image<'static, 'static>>(&mut renderer, Fourcc::Argb8888, None);
    assert_eq!(soft_pieces(&mut harness, &mut renderer), before + 2);

    // A second object for the same surface is an error of the program's;
    // letting go of the first ends the softness with the next commit.
    effect.destroy();
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    assert!(gevrek_smithay::background_effect::blur_parts(&server_surface).is_empty());
    assert_eq!(soft_pieces(&mut harness, &mut renderer), before);
    // (And the surface can be given another.)
    let again = client
        .app
        .effects
        .as_ref()
        .expect("the protocol")
        .get_background_effect(&client.app.sheets[desk].surface, &handle, ());
    again.set_blur_region(None);
    client.app.sheets[desk].surface.commit();
    pump(&mut harness, &mut client);
    assert!(client.connection.protocol_error().is_none());
}

/// Take a picture of the display as a screenshot tool does, and give
/// back the pixel in its middle: blue, green, red.
fn picture_of_the_display(
    harness: &mut Harness,
    client: &mut Client,
    renderer: &mut PixmanRenderer,
    target: &mut gevrek_smithay::capture::CaptureTarget<pixman::Image<'static, 'static>>,
) -> [u8; 3] {
    let handle = client.queue.handle();
    client.app.frame_buffer = None;
    client.app.frame_ready = false;
    let frame = client
        .app
        .screencopy
        .as_ref()
        .expect("screencopy")
        .capture_output(0, client.app.output.as_ref().expect("an output"), &handle, ());
    pump(harness, client);
    let (width, height, stride) = client.app.frame_buffer.expect("told how the buffer must be");
    assert_eq!(stride, width * 4);
    let (buffer, file) = client.app.buffer(
        &handle,
        (width as i32, height as i32),
        wl_shm::Format::Xrgb8888,
        [0; 4],
    );
    frame.copy(&buffer);
    pump(harness, client);
    harness.state.serve_captures(renderer, target, Fourcc::Argb8888, None);
    pump(harness, client);
    assert!(client.app.frame_ready, "the picture came");
    assert!(!client.app.frame_failed);
    let mut pixel = [0u8; 4];
    let middle = u64::from((height / 2) * stride + (width / 2) * 4);
    file.read_exact_at(&mut pixel, middle).expect("the pixel");
    frame.destroy();
    buffer.destroy();
    [pixel[0], pixel[1], pixel[2]]
}

fn is_red(pixel: [u8; 3]) -> bool {
    pixel[2] > 200 && pixel[0] < 60 && pixel[1] < 60
}

#[test]
fn a_surface_whose_rule_says_so_is_left_out_of_pictures_of_the_screen() {
    let mut config = Config::new();
    config.desktop.layer_rules = serde_json::from_value(serde_json::json!([
        { "namespace": "test-private", "exclude_from_capture": true },
    ]))
    .expect("rules");
    let mut harness = Harness::new(config);
    let mut renderer = PixmanRenderer::new().expect("a renderer in memory");
    let mut target = gevrek_smithay::capture::CaptureTarget::default();
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();

    // A surface like any other, red all over: it is in the picture.
    let plain = client.app.sheet(
        &handle,
        "test-plain",
        Layer::Top,
        everywhere(),
        (0, 0),
        [0, 0, 255, 255],
    );
    pump(&mut harness, &mut client);
    let seen = picture_of_the_display(&mut harness, &mut client, &mut renderer, &mut target);
    assert!(is_red(seen), "a surface is in the picture: {seen:?}");
    client.app.sheets[plain].layer.destroy();
    client.app.sheets[plain].surface.destroy();
    pump(&mut harness, &mut client);

    // One screenshot is no sharing of the screen.
    assert!(harness.state.shared_displays().is_empty());

    // The same, of a name the rule keeps out: on the display (the
    // pointer finds it), and not in the picture.
    client.app.sheet(
        &handle,
        "test-private",
        Layer::Top,
        everywhere(),
        (0, 0),
        [0, 0, 255, 255],
    );
    pump(&mut harness, &mut client);
    assert!(harness.state.surface_under((900.0, 500.0).into()).1.is_some());
    let on_display = harness
        .state
        .render_elements(&mut renderer, harness.output)
        .len();
    assert!(on_display > 0, "it is drawn on the display");
    let seen = picture_of_the_display(&mut harness, &mut client, &mut renderer, &mut target);
    assert!(!is_red(seen), "the surface is not in the picture: {seen:?}");

    // Pictures again and again: the display is shared, and no longer
    // when they stop.
    assert_eq!(harness.state.shared_displays(), ["TEST-1"]);
    harness.state.tick_sharing(Instant::now() + Duration::from_secs(1));
    assert_eq!(harness.state.shared_displays(), ["TEST-1"]);
    harness.state.tick_sharing(Instant::now() + Duration::from_secs(10));
    assert!(harness.state.shared_displays().is_empty());
}

#[test]
fn the_room_the_zones_keep_is_told_apart_from_a_panels() {
    use gevrek_engine::shell_room::Side;
    use gevrek_smithay::protocols::kept_side;
    use smithay::wayland::shell::wlr_layer::Anchor as Fixed;
    // A strip along a side, with or without the edges beside it.
    assert_eq!(kept_side(Fixed::TOP, 96), Some((Side::Top, 96)));
    assert_eq!(
        kept_side(Fixed::RIGHT | Fixed::TOP | Fixed::BOTTOM, 280),
        Some((Side::Right, 280))
    );
    assert_eq!(
        kept_side(Fixed::BOTTOM | Fixed::LEFT | Fixed::RIGHT, 40),
        Some((Side::Bottom, 40))
    );
    assert_eq!(kept_side(Fixed::LEFT, 64), Some((Side::Left, 64)));
    // What keeps no room, or is at no single side.
    assert_eq!(kept_side(Fixed::TOP, 0), None);
    assert_eq!(kept_side(Fixed::TOP, -1), None);
    assert_eq!(kept_side(Fixed::TOP | Fixed::LEFT, 40), None);
    assert_eq!(kept_side(Fixed::all(), 40), None);
    assert_eq!(kept_side(Fixed::empty(), 40), None);

    let mut harness = Harness::new(Config::new());
    let mut client = connect(&mut harness);
    let handle = client.queue.handle();
    let output = harness.output;
    assert_eq!(harness.state.zones_room(output), Default::default());
    // A panel at the top keeps room that is not the zones'.
    let panel = client.app.sheet(
        &handle,
        "test-panel",
        Layer::Top,
        Anchor::Top | Anchor::Left | Anchor::Right,
        (0, 40),
        [40, 40, 40, 255],
    );
    client.app.sheets[panel].layer.set_exclusive_zone(40);
    client.app.sheets[panel].surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(harness.state.zones_room(output), Default::default());
    // The zones' strip at the right, nothing drawn on it that matters.
    let strip = client.app.sheet(
        &handle,
        "gevrek-zones-room",
        Layer::Background,
        Anchor::Right | Anchor::Top | Anchor::Bottom,
        (280, 0),
        [0, 0, 0, 0],
    );
    client.app.sheets[strip].layer.set_exclusive_zone(280);
    client.app.sheets[strip].surface.commit();
    pump(&mut harness, &mut client);
    let room = harness.state.zones_room(output);
    assert_eq!((room.left, room.top, room.right, room.bottom), (0, 0, 280, 0));
    // The room is deeper: what is told follows.
    client.app.sheets[strip].layer.set_size(320, 0);
    client.app.sheets[strip].layer.set_exclusive_zone(320);
    client.app.sheets[strip].surface.commit();
    pump(&mut harness, &mut client);
    assert_eq!(harness.state.zones_room(output).right, 320);
    // Gone with its surface.
    client.app.sheets[strip].layer.destroy();
    client.app.sheets[strip].surface.destroy();
    pump(&mut harness, &mut client);
    assert_eq!(harness.state.zones_room(output), Default::default());
}
