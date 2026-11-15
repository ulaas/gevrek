//! `gevrek-testclient --capture FILE --window TITLE` (one window) and
//! `--capture FILE --newer [--output N]` (an output): a picture asked for
//! the newer way (ext-image-copy-capture), written as a PPM file.
//! `--frames N` asks for N frames of one session and says when each came.
//! `--pointer-alone`: the pointer by itself over that output or window:
//! where it is there and its hot spot are said, and the picture is the
//! pointer's own, written with how solid each pixel is (a PAM file).
//! `--card`: the picture is asked for straight into a buffer of the
//! graphics card's own memory, which this program makes; it cannot be
//! written to a file from here, so it is shown instead, as the content
//! of a window of this program's (titled `captured`), until the program
//! is ended.

use super::{say, word};
use smithay_client_toolkit::shm::{raw::RawPool, Shm, ShmHandler};
use std::io::Write;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

pub struct Request {
    pub path: String,
    /// The window with this title; `None`: an output.
    pub window: Option<String>,
    pub output: usize,
    pub frames: u32,
    /// The pointer by itself, over that output or window.
    pub pointer_alone: bool,
    /// Into a buffer of the graphics card, shown in a window afterwards.
    pub card: bool,
    /// With `card`: asked the older way (wlr-screencopy).
    pub older: bool,
}

struct Taking {
    shm: Shm,
    windows: Vec<(ExtForeignToplevelHandleV1, String)>,
    listed: bool,
    size: Option<(u32, u32)>,
    described: bool,
    stopped: bool,
    pool: Option<RawPool>,
    /// `Some(true)`: the frame that was asked for is in the buffer.
    frame: Option<bool>,
    /// The graphics card pictures may be drawn into buffers of, as the
    /// system numbers its device, and the kinds of buffer with their
    /// layouts.
    card: Option<u64>,
    card_kinds: Vec<(u32, Vec<u64>)>,
    /// The window that shows the picture was told its size.
    configured: bool,
}

pub fn run(connection: &Connection, request: Request) -> bool {
    let (globals, mut queue) = registry_queue_init::<Taking>(connection).expect("registry");
    let handle = queue.handle();
    let Ok(manager) = globals.bind::<ExtImageCopyCaptureManagerV1, _, _>(&handle, 1..=1, ()) else {
        say(
            "error",
            &[("message", word("the compositor has no image-copy-capture"))],
        );
        return false;
    };
    let mut taking = Taking {
        shm: Shm::bind(&globals, &handle).expect("wl_shm"),
        windows: Vec::new(),
        listed: false,
        size: None,
        described: false,
        stopped: false,
        pool: None,
        frame: None,
        card: None,
        card_kinds: Vec::new(),
        configured: false,
    };
    if request.card && request.older {
        return older_into_card(&globals, &mut queue, &mut taking, request.output);
    }
    let source: ExtImageCaptureSourceV1 = match &request.window {
        Some(title) => {
            let list: ExtForeignToplevelListV1 =
                globals.bind(&handle, 1..=1, ()).expect("the window list");
            let sources: ExtForeignToplevelImageCaptureSourceManagerV1 = globals
                .bind(&handle, 1..=1, ())
                .expect("sources of windows");
            // The windows that are there, and their titles.
            connection.roundtrip().expect("roundtrip");
            queue.roundtrip(&mut taking).expect("roundtrip");
            queue.roundtrip(&mut taking).expect("roundtrip");
            let _ = &list;
            let Some((window, _)) = taking.windows.iter().find(|(_, known)| known == title) else {
                say("error", &[("message", word("no such window"))]);
                return false;
            };
            sources.create_source(window, &handle, ())
        }
        None => {
            let sources: ExtOutputImageCaptureSourceManagerV1 = globals
                .bind(&handle, 1..=1, ())
                .expect("sources of outputs");
            let outputs: Vec<(u32, u32)> = globals.contents().with_list(|list| {
                list.iter()
                    .filter(|global| global.interface == "wl_output")
                    .map(|global| (global.name, global.version))
                    .collect()
            });
            let Some((name, version)) = outputs.get(request.output).copied() else {
                say("error", &[("message", word("no such output"))]);
                return false;
            };
            let output: wl_output::WlOutput =
                globals.registry().bind(name, version.min(4), &handle, ());
            sources.create_source(&output, &handle, ())
        }
    };
    let session = if request.pointer_alone {
        let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=5, ()).expect("a seat");
        let pointer = seat.get_pointer(&handle, ());
        let followed = manager.create_pointer_cursor_session(&source, &pointer, &handle, ());
        followed.get_capture_session(&handle, ())
    } else {
        manager.create_session(&source, Options::empty(), &handle, ())
    };
    while !taking.described && !taking.stopped {
        queue.blocking_dispatch(&mut taking).expect("dispatch");
    }
    let Some((width, height)) = taking.size.filter(|_| !taking.stopped) else {
        say("capture_failed", &[("reason", word("stopped"))]);
        return false;
    };
    if request.card {
        return into_card(&globals, &mut queue, &mut taking, &session, (width, height));
    }
    let stride = width * 4;
    let mut pool = RawPool::new((stride * height) as usize, &taking.shm).expect("pool");
    let buffer: wl_buffer::WlBuffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        if request.pointer_alone {
            wl_shm::Format::Argb8888
        } else {
            wl_shm::Format::Xrgb8888
        },
        (),
        &handle,
    );
    taking.pool = Some(pool);
    let started = std::time::Instant::now();
    for number in 0..request.frames.max(1) {
        taking.frame = None;
        let frame = session.create_frame(&handle, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        while taking.frame.is_none() {
            queue.blocking_dispatch(&mut taking).expect("dispatch");
        }
        frame.destroy();
        if taking.frame != Some(true) {
            say("capture_failed", &[("frame", number.to_string())]);
            return false;
        }
        say(
            "frame",
            &[
                ("number", number.to_string()),
                ("after_ms", started.elapsed().as_millis().to_string()),
            ],
        );
    }
    let written = (|| -> std::io::Result<()> {
        let pixels = taking.pool.as_mut().expect("pool").mmap();
        let mut file = std::io::BufWriter::new(std::fs::File::create(&request.path)?);
        if request.pointer_alone {
            // With how solid each pixel is.
            write!(
                file,
                "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
            )?;
            for row in 0..height as usize {
                let line =
                    &pixels[row * stride as usize..row * stride as usize + width as usize * 4];
                for pixel in line.as_chunks::<4>().0 {
                    file.write_all(&[pixel[2], pixel[1], pixel[0], pixel[3]])?;
                }
            }
            return file.flush();
        }
        write!(file, "P6\n{width} {height}\n255\n")?;
        for row in 0..height as usize {
            let line = &pixels[row * stride as usize..row * stride as usize + width as usize * 4];
            for pixel in line.as_chunks::<4>().0 {
                file.write_all(&[pixel[2], pixel[1], pixel[0]])?;
            }
        }
        file.flush()
    })();
    match written {
        Ok(()) => {
            say(
                "captured",
                &[("width", width.to_string()), ("height", height.to_string())],
            );
            true
        }
        Err(error) => {
            say("error", &[("message", word(&error.to_string()))]);
            false
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            taking.windows.push((toplevel, String::new()));
            taking.listed = true;
        }
    }

    event_created_child!(Taking, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Taking {
    fn event(
        taking: &mut Self,
        proxy: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_handle_v1::Event::Title { title } = event {
            if let Some((_, known)) = taking
                .windows
                .iter_mut()
                .find(|(window, _)| window == proxy)
            {
                *known = title;
            }
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                taking.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                if format != WEnum::Value(wl_shm::Format::Xrgb8888)
                    && format != WEnum::Value(wl_shm::Format::Argb8888)
                {
                    say("note", &[("format", word("another than xrgb8888 too"))]);
                }
            }
            ext_image_copy_capture_session_v1::Event::DmabufDevice { device } => {
                taking.card = <[u8; 8]>::try_from(device.as_slice())
                    .ok()
                    .map(u64::from_ne_bytes);
            }
            ext_image_copy_capture_session_v1::Event::DmabufFormat { format, modifiers } => {
                let layouts = modifiers
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|bytes| u64::from_ne_bytes(*bytes))
                    .collect();
                taking.card_kinds.push((format, layouts));
            }
            ext_image_copy_capture_session_v1::Event::Done => taking.described = true,
            ext_image_copy_capture_session_v1::Event::Stopped => taking.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => taking.frame = Some(true),
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => taking.frame = Some(false),
            _ => {}
        }
    }
}

/// All of an output asked for the older way (wlr-screencopy), into a
/// buffer of the graphics card that is made here (on the first card
/// there is: that way of asking does not say which), and then shown.
fn older_into_card(
    globals: &wayland_client::globals::GlobalList,
    queue: &mut wayland_client::EventQueue<Taking>,
    taking: &mut Taking,
    output: usize,
) -> bool {
    use std::os::fd::AsFd;
    use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
    let handle = queue.handle();
    let fail = |why: &str| {
        say("capture_failed", &[("reason", word(why))]);
        false
    };
    let Ok(manager) = globals.bind::<ZwlrScreencopyManagerV1, _, _>(&handle, 3..=3, ()) else {
        return fail("the compositor has no screencopy of that age");
    };
    let outputs: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect()
    });
    let Some((name, version)) = outputs.get(output).copied() else {
        return fail("no such output");
    };
    let display: wl_output::WlOutput = globals.registry().bind(name, version.min(4), &handle, ());
    let frame = manager.capture_output(0, &display, &handle, ());
    while !taking.described && taking.frame.is_none() {
        queue.blocking_dispatch(taking).expect("dispatch");
    }
    let Some((width, height)) = taking.size else {
        return fail("no buffer of the graphics card is offered for it");
    };
    let node = (128..136)
        .map(|number| format!("/dev/dri/renderD{number}"))
        .find_map(|path| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .ok()
        });
    let Some(card) = node.and_then(|file| gbm::Device::new(file).ok()) else {
        return fail("the graphics card makes no buffers for this program");
    };
    let Ok(made) = card.create_buffer_object::<()>(
        width,
        height,
        gbm::Format::Xrgb8888,
        gbm::BufferObjectFlags::RENDERING,
    ) else {
        return fail("the graphics card made no buffer");
    };
    let Ok(fd) = made.fd() else {
        return fail("the buffer cannot be handed over");
    };
    let layout = u64::from(made.modifier());
    let Ok(buffers) = globals.bind::<ZwpLinuxDmabufV1, _, _>(&handle, 3..=3, ()) else {
        return fail("the compositor takes no buffers of the graphics card");
    };
    let params = buffers.create_params(&handle, ());
    params.add(
        fd.as_fd(),
        0,
        made.offset(0),
        made.stride(),
        (layout >> 32) as u32,
        layout as u32,
    );
    let buffer = params.create_immed(
        width as i32,
        height as i32,
        XRGB,
        zwp_linux_buffer_params_v1::Flags::empty(),
        &handle,
        (),
    );
    taking.frame = None;
    frame.copy(&buffer);
    while taking.frame.is_none() {
        queue.blocking_dispatch(taking).expect("dispatch");
    }
    if taking.frame != Some(true) {
        return fail("the picture was not drawn into the buffer");
    }
    shown_in_a_window(
        globals,
        queue,
        taking,
        &buffer,
        (width, height),
        (made, card),
    )
}

impl Dispatch<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()>
    for Taking
{
    fn event(
        taking: &mut Self,
        _: &wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event;
        match event {
            Event::LinuxDmabuf { format, width, height } if format == XRGB => {
                taking.size = Some((width, height));
            }
            Event::BufferDone => taking.described = true,
            Event::Ready { .. } => taking.frame = Some(true),
            Event::Failed => taking.frame = Some(false),
            _ => {}
        }
    }
}

/// `XRGB8888`, as the graphics card's buffers name it.
const XRGB: u32 = 0x3432_5258;

/// The picture asked for straight into a buffer of the graphics card
/// that is made here, and then shown as the content of a window.
fn into_card(
    globals: &wayland_client::globals::GlobalList,
    queue: &mut wayland_client::EventQueue<Taking>,
    taking: &mut Taking,
    session: &ExtImageCopyCaptureSessionV1,
    (width, height): (u32, u32),
) -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;
    let handle = queue.handle();
    let fail = |why: &str| {
        say("capture_failed", &[("reason", word(why))]);
        false
    };
    let Some(device) = taking.card else {
        return fail("no graphics card is offered for it");
    };
    let Some((_, layouts)) = taking.card_kinds.iter().find(|(kind, _)| *kind == XRGB) else {
        return fail("the kind of buffer this program makes is not offered");
    };
    // The card's device, by its number.
    let node = std::fs::read_dir("/dev/dri").ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| std::fs::metadata(path).is_ok_and(|about| about.rdev() == device))
    });
    let Some(node) = node else {
        return fail("the graphics card's device is not found");
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
    else {
        return fail("the graphics card's device cannot be opened");
    };
    let Ok(card) = gbm::Device::new(file) else {
        return fail("the graphics card makes no buffers for this program");
    };
    let made = card
        .create_buffer_object_with_modifiers2::<()>(
            width,
            height,
            gbm::Format::Xrgb8888,
            layouts.iter().map(|layout| gbm::Modifier::from(*layout)),
            gbm::BufferObjectFlags::RENDERING,
        )
        .or_else(|_| {
            card.create_buffer_object::<()>(
                width,
                height,
                gbm::Format::Xrgb8888,
                gbm::BufferObjectFlags::RENDERING,
            )
        });
    let Ok(made) = made else {
        return fail("the graphics card made no buffer");
    };
    let Ok(fd) = made.fd() else {
        return fail("the buffer cannot be handed over");
    };
    let layout = u64::from(made.modifier());
    let Ok(buffers) = globals.bind::<ZwpLinuxDmabufV1, _, _>(&handle, 3..=3, ()) else {
        return fail("the compositor takes no buffers of the graphics card");
    };
    let params = buffers.create_params(&handle, ());
    params.add(
        fd.as_fd(),
        0,
        made.offset(0),
        made.stride(),
        (layout >> 32) as u32,
        layout as u32,
    );
    let buffer = params.create_immed(
        width as i32,
        height as i32,
        XRGB,
        zwp_linux_buffer_params_v1::Flags::empty(),
        &handle,
        (),
    );
    let frame = session.create_frame(&handle, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width as i32, height as i32);
    frame.capture();
    taking.frame = None;
    while taking.frame.is_none() {
        queue.blocking_dispatch(taking).expect("dispatch");
    }
    frame.destroy();
    if taking.frame != Some(true) {
        return fail("the picture was not drawn into the buffer");
    }
    shown_in_a_window(
        globals,
        queue,
        taking,
        &buffer,
        (width, height),
        (made, card),
    )
}

/// Say that the picture is in the card's buffer, and show that buffer
/// as the content of a window until the program is ended.
fn shown_in_a_window(
    globals: &wayland_client::globals::GlobalList,
    queue: &mut wayland_client::EventQueue<Taking>,
    taking: &mut Taking,
    buffer: &wl_buffer::WlBuffer,
    (width, height): (u32, u32),
    (made, card): (gbm::BufferObject<()>, gbm::Device<std::fs::File>),
) -> bool {
    let handle = queue.handle();
    say(
        "captured",
        &[
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("into", word("a buffer of the graphics card")),
        ],
    );
    // Shown: a window whose content is that buffer.
    let compositor: wl_compositor::WlCompositor =
        globals.bind(&handle, 4..=4, ()).expect("wl_compositor");
    let shell: xdg_wm_base::XdgWmBase = globals.bind(&handle, 1..=1, ()).expect("xdg_wm_base");
    let surface = compositor.create_surface(&handle, ());
    let shown = shell.get_xdg_surface(&surface, &handle, ());
    let window = shown.get_toplevel(&handle, ());
    window.set_title("captured".to_string());
    window.set_app_id("gevrek-testclient".to_string());
    // (`GEVREK_TEST_FULLSCREEN`: over the whole display, as a game's
    // picture is: a buffer of the card's that may go to the display as
    // it is.)
    // (And as a game does, it shows no pointer over itself.)
    let mut _pointer = None;
    if std::env::var_os("GEVREK_TEST_FULLSCREEN").is_some() {
        window.set_fullscreen(None);
        if let Ok(seat) = globals.bind::<wl_seat::WlSeat, _, _>(&handle, 1..=5, ()) {
            _pointer = Some(seat.get_pointer(&handle, NoPointer));
        }
    }
    surface.commit();
    while !taking.configured {
        queue.blocking_dispatch(taking).expect("dispatch");
    }
    surface.attach(Some(buffer), 0, 0);
    surface.damage_buffer(0, 0, width as i32, height as i32);
    surface.commit();
    say("shown", &[]);
    // (Until the program is ended; the card's buffer is kept meanwhile.)
    let _kept = (made, card);
    loop {
        if queue.blocking_dispatch(taking).is_err() {
            return true;
        }
    }
}

/// Marks a pointer that is hidden over this program's window.
struct NoPointer;

impl Dispatch<wl_pointer::WlPointer, NoPointer> for Taking {
    fn event(
        _: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &NoPointer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter { serial, .. } = event {
            pointer.set_cursor(serial, None, 0, 0);
            say("pointer_hidden", &[]);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Taking {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Taking {
    fn event(
        taking: &mut Self,
        shown: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            shown.ack_configure(serial);
            taking.configured = true;
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for Taking {
    fn event(
        _: &mut Self,
        _: &ExtImageCopyCaptureCursorSessionV1,
        event: ext_image_copy_capture_cursor_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_cursor_session_v1::Event;
        match event {
            Event::Enter => say("pointer_enter", &[]),
            Event::Leave => say("pointer_leave", &[]),
            Event::Position { x, y } => say(
                "pointer_position",
                &[("x", x.to_string()), ("y", y.to_string())],
            ),
            Event::Hotspot { x, y } => say(
                "pointer_hotspot",
                &[("x", x.to_string()), ("y", y.to_string())],
            ),
            _ => {}
        }
    }
}

macro_rules! quiet {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Taking {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
quiet!(ExtImageCopyCaptureManagerV1, ());
quiet!(ExtImageCaptureSourceV1, ());
quiet!(ExtOutputImageCaptureSourceManagerV1, ());
quiet!(ExtForeignToplevelImageCaptureSourceManagerV1, ());
quiet!(wl_output::WlOutput, ());
quiet!(wl_seat::WlSeat, ());
quiet!(wl_compositor::WlCompositor, ());
quiet!(wl_surface::WlSurface, ());
quiet!(xdg_toplevel::XdgToplevel, ());
quiet!(ZwpLinuxDmabufV1, ());
quiet!(
    wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    ()
);
quiet!(ZwpLinuxBufferParamsV1, ());
quiet!(wl_pointer::WlPointer, ());
quiet!(wl_buffer::WlBuffer, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

impl ShmHandler for Taking {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

smithay_client_toolkit::delegate_shm!(Taking);
