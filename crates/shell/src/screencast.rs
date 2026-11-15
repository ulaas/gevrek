//! Screen sharing: a display or one window as a stream of pictures that
//! another program (a browser in a video call, a recorder) takes from
//! PipeWire.
//!
//! The pictures are asked of the compositor as a recorder asks
//! (ext-image-copy-capture: the first at once, each next one when
//! something changed), on a thread and a connection of their own. A
//! second thread is the PipeWire side: a node of the kind `Video/Source`
//! that hands each picture on as it comes. The portal tells the
//! application the node's number (`portal.rs`), and the application takes
//! the stream through the portal's own door to PipeWire.
//!
//! Pictures go through shared memory, one copy from the compositor's
//! buffer and one into PipeWire's; the stream has blue first with a byte
//! to spare (`BGRx`), as the compositor gives it. When what is shared
//! changes its size (a window made larger), the stream is told the new
//! one.

use anyhow::Context as _;
use pipewire as pw;
use pw::spa;
use smithay_client_toolkit::shm::{raw::RawPool, Shm, ShmHandler};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
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
    zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};

/// The most pictures a second that are handed on.
pub const MOST_A_SECOND: u32 = 60;
/// How long the compositor and PipeWire are waited for at the start.
mod card;

const PATIENCE: Duration = Duration::from_secs(5);

/// What is shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    /// A display, by its name (`DP-1`).
    Output(String),
    /// A window, by the name the compositor's window list gives it.
    Window(String),
}

/// Something that can be shared, for the chooser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub what: What,
    /// A display's name, a window's title.
    pub label: String,
    /// A window's program; empty for a display.
    pub detail: String,
    /// A small picture of it as it is now, where one could be taken.
    pub preview: Option<Arc<crate::thumbs::Thumb>>,
}

/// The longer side of a preview, in pixels.
pub const PREVIEW_SIDE: u32 = 480;
/// How long one preview is waited for, and all of them together: the
/// chooser opens after that with the pictures it has.
const PREVIEW_PATIENCE: Duration = Duration::from_millis(300);
const PREVIEWS_PATIENCE: Duration = Duration::from_millis(1500);

/// The pictures a second that fit between this picture and the last:
/// how long to wait before the next is asked for.
pub fn pause(since_last: Duration, most_a_second: u32) -> Duration {
    (Duration::from_secs(1) / most_a_second.max(1)).saturating_sub(since_last)
}

struct Window {
    handle: ExtForeignToplevelHandleV1,
    identifier: String,
    title: String,
    app_id: String,
}

struct Taking {
    shm: Shm,
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    windows: Vec<Window>,
    size: Option<(u32, u32)>,
    described: bool,
    stopped: bool,
    /// `Some(true)`: the frame that was asked for is in the buffer.
    frame: Option<bool>,
    /// The pointer by itself, where it is followed (`Following`).
    pointer: PointerTold,
    /// The graphics card whose buffers a picture may be drawn into (the
    /// number the system has for its device), and the layouts such
    /// buffers of four bytes a pixel may have.
    card: Option<u64>,
    card_layouts: Vec<u64>,
}

/// What the compositor says of the pointer that is followed.
#[derive(Default)]
struct PointerTold {
    size: Option<(u32, u32)>,
    described: bool,
    frame: Option<bool>,
    /// Over what is shared.
    inside: bool,
    at: (i32, i32),
    hotspot: (i32, i32),
    /// Something of it is another than when it was last handed on.
    changed: bool,
}

/// Marks the pointer's own session and its frames among the others.
struct OfPointer;

struct Wayland {
    queue: EventQueue<Taking>,
    taking: Taking,
    manager: ExtImageCopyCaptureManagerV1,
    of_outputs: ExtOutputImageCaptureSourceManagerV1,
    of_windows: ExtForeignToplevelImageCaptureSourceManagerV1,
    seat: Option<wl_seat::WlSeat>,
    /// Where buffers of the graphics card are made known to the
    /// compositor (none where it takes none).
    buffers: Option<ZwpLinuxDmabufV1>,
}

/// A connection to the compositor that knows the displays and the windows.
fn wayland() -> anyhow::Result<Wayland> {
    let connection = Connection::connect_to_env().context("no Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<Taking>(&connection).context("the compositor's globals")?;
    let handle = queue.handle();
    let none = |_| anyhow::anyhow!("the compositor gives no pictures to share");
    let manager = globals
        .bind::<ExtImageCopyCaptureManagerV1, _, _>(&handle, 1..=1, ())
        .map_err(none)?;
    let of_outputs = globals
        .bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(&handle, 1..=1, ())
        .map_err(none)?;
    let of_windows = globals
        .bind::<ExtForeignToplevelImageCaptureSourceManagerV1, _, _>(&handle, 1..=1, ())
        .map_err(none)?;
    let _list = globals
        .bind::<ExtForeignToplevelListV1, _, _>(&handle, 1..=1, ())
        .map_err(none)?;
    let mut taking = Taking {
        shm: Shm::bind(&globals, &handle).context("wl_shm")?,
        outputs: Vec::new(),
        windows: Vec::new(),
        size: None,
        described: false,
        stopped: false,
        frame: None,
        pointer: PointerTold::default(),
        card: None,
        card_layouts: Vec::new(),
    };
    let buffers = globals
        .bind::<ZwpLinuxDmabufV1, _, _>(&handle, 3..=3, ())
        .ok();
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&handle, 1..=5, ())
        .ok();
    let listed: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect()
    });
    for (name, version) in listed {
        let bound: wl_output::WlOutput = globals.registry().bind(name, version.min(4), &handle, ());
        taking.outputs.push((bound, None));
    }
    // The displays say their names, the windows their titles.
    for _ in 0..2 {
        queue
            .roundtrip(&mut taking)
            .context("the displays and the windows")?;
    }
    Ok(Wayland {
        queue,
        taking,
        manager,
        of_outputs,
        of_windows,
        seat,
        buffers,
    })
}

/// One picture of something that can be shared, made small.
fn preview(wayland: &mut Wayland, of: &ExtImageCaptureSourceV1) -> Option<crate::thumbs::Thumb> {
    let handle = wayland.queue.handle();
    let session = wayland
        .manager
        .create_session(of, Options::empty(), &handle, ());
    wayland.taking.size = None;
    wayland.taking.described = false;
    wayland.taking.stopped = false;
    wayland.taking.frame = None;
    let until = Instant::now() + PREVIEW_PATIENCE;
    let taken = (|| {
        while !(wayland.taking.described || wayland.taking.stopped) {
            if Instant::now() >= until {
                return None;
            }
            pump(wayland, Duration::from_millis(20)).ok()?;
        }
        let (width, height) = wayland
            .taking
            .size
            .filter(|(width, height)| !wayland.taking.stopped && *width > 0 && *height > 0)?;
        let stride = width * 4;
        let mut pool = RawPool::new((stride * height) as usize, &wayland.taking.shm).ok()?;
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Xrgb8888,
            (),
            &handle,
        );
        let frame = session.create_frame(&handle, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        while wayland.taking.frame.is_none() && !wayland.taking.stopped {
            if Instant::now() >= until || pump(wayland, Duration::from_millis(20)).is_err() {
                break;
            }
        }
        let small = (wayland.taking.frame == Some(true))
            .then(|| crate::thumbs::shrink(width, height, stride, pool.mmap(), PREVIEW_SIDE))
            .flatten();
        frame.destroy();
        buffer.destroy();
        small
    })();
    session.destroy();
    wayland.taking.stopped = false;
    taken
}

/// Everything that can be shared now: the displays, then the windows,
/// each with a small picture of what it shows at this moment (taken
/// before the chooser is there, so that the chooser is in none).
pub fn sources() -> anyhow::Result<Vec<Source>> {
    listed(true)
}

/// Everything that can be shared or recorded now; with `previews`, each
/// with a small picture of it (which takes a moment for each).
pub fn listed(previews: bool) -> anyhow::Result<Vec<Source>> {
    let mut wayland = wayland()?;
    let handle = wayland.queue.handle();
    let mut all: Vec<(Source, ExtImageCaptureSourceV1)> = Vec::new();
    for (output, name) in &wayland.taking.outputs {
        let Some(name) = name.clone() else {
            continue;
        };
        all.push((
            Source {
                what: What::Output(name.clone()),
                label: name,
                detail: String::new(),
                preview: None,
            },
            wayland.of_outputs.create_source(output, &handle, ()),
        ));
    }
    for window in &wayland.taking.windows {
        if window.identifier.is_empty() {
            continue;
        }
        all.push((
            Source {
                what: What::Window(window.identifier.clone()),
                label: window.title.clone(),
                detail: window.app_id.clone(),
                preview: None,
            },
            wayland
                .of_windows
                .create_source(&window.handle, &handle, ()),
        ));
    }
    let began = Instant::now();
    for (source, of) in &mut all {
        if previews && began.elapsed() < PREVIEWS_PATIENCE {
            source.preview = preview(&mut wayland, of).map(Arc::new);
        }
        of.destroy();
    }
    let _ = wayland.queue.roundtrip(&mut wayland.taking);
    Ok(all.into_iter().map(|(source, _)| source).collect())
}

/// The pictures of a display or a window, one after another, for
/// whoever wants them on its own thread (the recorder): each when
/// something changed, as the compositor gives them to a recorder.
pub struct Grab {
    wayland: Wayland,
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    memory: Option<(RawPool, wl_buffer::WlBuffer, (u32, u32))>,
    /// The picture that is asked for and has not come yet.
    asked: Option<ExtImageCopyCaptureFrameV1>,
}

impl Grab {
    pub fn open(what: &What, with_pointer: bool) -> anyhow::Result<Self> {
        let wayland = wayland()?;
        let handle = wayland.queue.handle();
        let source = match what {
            What::Output(name) => {
                let output = wayland
                    .taking
                    .outputs
                    .iter()
                    .find(|(_, known)| known.as_deref() == Some(name.as_str()))
                    .map(|(output, _)| output.clone())
                    .with_context(|| format!("there is no display {name}"))?;
                wayland.of_outputs.create_source(&output, &handle, ())
            }
            What::Window(identifier) => {
                let window = wayland
                    .taking
                    .windows
                    .iter()
                    .find(|window| window.identifier == *identifier)
                    .map(|window| window.handle.clone())
                    .context("the window is not there any more")?;
                wayland.of_windows.create_source(&window, &handle, ())
            }
        };
        let options = if with_pointer {
            Options::PaintCursors
        } else {
            Options::empty()
        };
        let session = wayland
            .manager
            .create_session(&source, options, &handle, ());
        Ok(Self {
            wayland,
            source,
            session,
            memory: None,
            asked: None,
        })
    }

    /// The next picture, waited for at most `patience`: its size (its
    /// pixels are had with [`Grab::pixels`]). `None`: nothing changed in
    /// that time. An error: no more will come (the window was closed, the
    /// display taken away).
    pub fn next(&mut self, patience: Duration) -> anyhow::Result<Option<(u32, u32)>> {
        let until = Instant::now() + patience;
        let handle = self.wayland.queue.handle();
        loop {
            anyhow::ensure!(
                !self.wayland.taking.stopped,
                "what was recorded is there no more"
            );
            let left = until.saturating_duration_since(Instant::now());
            let size = self.wayland.taking.size.filter(|(width, height)| {
                self.wayland.taking.described && *width > 0 && *height > 0
            });
            let Some(size) = size else {
                if left.is_zero() {
                    return Ok(None);
                }
                pump(&mut self.wayland, left.min(Duration::from_millis(50)))?;
                continue;
            };
            if self.memory.as_ref().map(|(.., of)| *of) != Some(size) {
                if let Some(asked) = self.asked.take() {
                    asked.destroy();
                }
                if let Some((_, buffer, _)) = self.memory.take() {
                    buffer.destroy();
                }
                let stride = size.0 * 4;
                let mut pool = RawPool::new((stride * size.1) as usize, &self.wayland.taking.shm)
                    .context("no memory for the picture")?;
                let buffer = pool.create_buffer(
                    0,
                    size.0 as i32,
                    size.1 as i32,
                    stride as i32,
                    wl_shm::Format::Xrgb8888,
                    (),
                    &handle,
                );
                self.memory = Some((pool, buffer, size));
            }
            if self.asked.is_none() {
                let Some((_, buffer, _)) = self.memory.as_ref() else {
                    continue;
                };
                let asked = self.session.create_frame(&handle, ());
                asked.attach_buffer(buffer);
                asked.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
                asked.capture();
                self.wayland.taking.frame = None;
                self.asked = Some(asked);
            }
            match self.wayland.taking.frame.take() {
                Some(came) => {
                    if let Some(asked) = self.asked.take() {
                        asked.destroy();
                    }
                    if !came {
                        // (Another size now, or not drawn this once.)
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    return Ok(Some(size));
                }
                None => {
                    if left.is_zero() {
                        return Ok(None);
                    }
                    pump(&mut self.wayland, left.min(Duration::from_millis(50)))?;
                }
            }
        }
    }
}

impl Grab {
    /// The pixels of the picture that came last: four bytes each, blue
    /// first, rows of `width * 4` bytes. Empty before the first.
    pub fn pixels(&mut self) -> &[u8] {
        match self.memory.as_mut() {
            Some((pool, _, size)) => {
                let bytes = (size.0 * size.1 * 4) as usize;
                let raw = pool.mmap();
                &raw[..bytes.min(raw.len())]
            }
            None => &[],
        }
    }
}

impl Drop for Grab {
    fn drop(&mut self) {
        if let Some(asked) = self.asked.take() {
            asked.destroy();
        }
        if let Some((_, buffer, _)) = self.memory.take() {
            buffer.destroy();
        }
        self.session.destroy();
        self.source.destroy();
        let _ = self.wayland.queue.roundtrip(&mut self.wayland.taking);
    }
}

/// The newest picture, between the two threads.
#[derive(Default)]
struct Frame {
    width: u32,
    height: u32,
    /// Four bytes a pixel, blue first, rows of `width * 4` bytes.
    pixels: Vec<u8>,
    /// The pointer, where it is handed on beside the pictures.
    pointer: Option<Seen>,
}

/// The pointer as it is handed on beside the pictures.
#[derive(Clone, Default, PartialEq)]
struct Seen {
    /// Where it is in the picture; `None`: not over what is shared.
    at: Option<(i32, i32)>,
    hotspot: (i32, i32),
    /// Its own picture: a square of this side, four bytes a pixel, blue
    /// first; empty until the first one came.
    side: u32,
    pixels: Vec<u8>,
}

/// The pointer that is followed for a stream: its session with the
/// compositor, the memory its picture comes in, and what was last seen.
struct Following {
    followed: ExtImageCopyCaptureCursorSessionV1,
    session: ExtImageCopyCaptureSessionV1,
    _pointer: wl_pointer::WlPointer,
    memory: Option<(RawPool, wl_buffer::WlBuffer, u32)>,
    asked: Option<ExtImageCopyCaptureFrameV1>,
    seen: Seen,
    told: Instant,
}

impl Following {
    fn begin(wayland: &Wayland, source: &ExtImageCaptureSourceV1) -> Option<Self> {
        let handle = wayland.queue.handle();
        let pointer = wayland.seat.as_ref()?.get_pointer(&handle, ());
        let followed = wayland
            .manager
            .create_pointer_cursor_session(source, &pointer, &handle, ());
        let session = followed.get_capture_session(&handle, OfPointer);
        Some(Self {
            followed,
            session,
            _pointer: pointer,
            memory: None,
            asked: None,
            seen: Seen::default(),
            told: Instant::now() - Duration::from_secs(1),
        })
    }

    /// Take in what the compositor said of the pointer since the last
    /// time, ask for its next picture, and hand what changed on to the
    /// stream (no oftener than the pictures themselves go).
    fn step(
        &mut self,
        wayland: &mut Wayland,
        frame: &Mutex<Frame>,
        to_stream: &pw::channel::Sender<ToStream>,
    ) {
        let handle = wayland.queue.handle();
        let told = &mut wayland.taking.pointer;
        // Memory of the picture's size.
        if let Some((side, _)) = told.size.filter(|(side, _)| told.described && *side > 0) {
            if self.memory.as_ref().map(|(.., of)| *of) != Some(side) && self.asked.is_none() {
                if let Some((_, buffer, _)) = self.memory.take() {
                    buffer.destroy();
                }
                let stride = side * 4;
                if let Ok(mut pool) = RawPool::new((stride * side) as usize, &wayland.taking.shm) {
                    let buffer = pool.create_buffer(
                        0,
                        side as i32,
                        side as i32,
                        stride as i32,
                        wl_shm::Format::Argb8888,
                        (),
                        &handle,
                    );
                    self.memory = Some((pool, buffer, side));
                }
            }
        }
        let told = &mut wayland.taking.pointer;
        // The picture that was asked for came (or did not).
        if let (Some(came), Some(asked)) = (told.frame, self.asked.as_ref()) {
            asked.destroy();
            self.asked = None;
            told.frame = None;
            if let (true, Some((pool, _, side))) = (came, self.memory.as_mut()) {
                let bytes = (*side * *side * 4) as usize;
                let raw = pool.mmap();
                if raw.len() >= bytes {
                    self.seen.side = *side;
                    self.seen.pixels.clear();
                    self.seen.pixels.extend_from_slice(&raw[..bytes]);
                    told.changed = true;
                }
            }
        }
        // The next one (the compositor keeps it until the picture is
        // another).
        if let (None, Some((_, buffer, side))) = (&self.asked, self.memory.as_ref()) {
            let asked = self.session.create_frame(&handle, OfPointer);
            asked.attach_buffer(buffer);
            asked.damage_buffer(0, 0, *side as i32, *side as i32);
            asked.capture();
            self.asked = Some(asked);
        }
        if !told.changed
            || self.told.elapsed() < Duration::from_millis(1000 / u64::from(MOST_A_SECOND))
        {
            return;
        }
        told.changed = false;
        self.seen.at = told.inside.then_some(told.at);
        self.seen.hotspot = told.hotspot;
        self.told = Instant::now();
        frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pointer = Some(self.seen.clone());
        let _ = to_stream.send(ToStream::Frame);
    }

    fn end(self) {
        if let Some(asked) = self.asked {
            asked.destroy();
        }
        if let Some((_, buffer, _)) = self.memory {
            buffer.destroy();
        }
        self.session.destroy();
        self.followed.destroy();
    }
}

enum ToStream {
    /// A new picture is in the frame.
    Frame,
    /// What is shared has another size from now on.
    Size(u32, u32),
    Stop,
    /// (Sharing in the card's buffers, `card`.) A buffer to draw the
    /// next picture into is asked for.
    Want,
    /// The picture is in this buffer: it goes out.
    Filled(u64),
    /// Nothing was drawn into this buffer: it goes back.
    Unused(u64),
}

/// Something that is being shared. Dropped, it ends.
pub struct Cast {
    /// The stream's node in PipeWire.
    pub node: u32,
    pub size: (u32, u32),
    stop: Arc<AtomicBool>,
    to_stream: pw::channel::Sender<ToStream>,
}

impl Drop for Cast {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.to_stream.send(ToStream::Stop);
    }
}

/// Begin to share. `ended` is called (on another thread) when the
/// sharing ends by itself: the window was closed, the display taken away.
///
/// `in_card`: in buffers of the graphics card, with nothing copied on
/// the way, where that can be had (a display, a compositor that offers
/// it, a card that makes the buffers); else as ever.
pub fn start(
    what: &What,
    with_pointer: bool,
    beside: bool,
    in_card: bool,
    ended: impl FnOnce() + Send + 'static,
) -> anyhow::Result<Cast> {
    // (What is called at the end is kept for the ordinary way, should
    // the card's way not begin.)
    let ended = Arc::new(Mutex::new(Some(ended)));
    let ended = move || {
        let call = ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(call) = call {
            call();
        }
    };
    if in_card && matches!(what, What::Output(_)) {
        match card::start(what, with_pointer, beside, ended.clone()) {
            Ok(cast) => return Ok(cast),
            Err(error) => {
                log::info!("Sharing in buffers of the graphics card is not to be had ({error:#}): shared the ordinary way");
            }
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let frame = Arc::new(Mutex::new(Frame::default()));
    let (to_stream, from_capture) = pw::channel::channel::<ToStream>();
    // The pictures: the thread says how large they are, or why none come.
    let (described, size) = mpsc::channel::<Result<(u32, u32), String>>();
    {
        let (what, stop, frame, to_stream) = (
            what.clone(),
            Arc::clone(&stop),
            Arc::clone(&frame),
            to_stream.clone(),
        );
        std::thread::Builder::new()
            .name("gevrek-share-pictures".to_string())
            .spawn(move || {
                let told = described.clone();
                if let Err(error) =
                    capture(&what, with_pointer, beside, &stop, &frame, &to_stream, told)
                {
                    let _ = described.send(Err(format!("{error:#}")));
                }
                let by_itself = !stop.swap(true, Ordering::Relaxed);
                let _ = to_stream.send(ToStream::Stop);
                if by_itself {
                    ended();
                }
            })
            .context("no thread for the pictures")?;
    }
    let ended_early = |stop: &AtomicBool| stop.store(true, Ordering::Relaxed);
    let size = match size.recv_timeout(PATIENCE) {
        Ok(Ok(size)) => size,
        Ok(Err(why)) => {
            ended_early(&stop);
            anyhow::bail!(why);
        }
        Err(_) => {
            ended_early(&stop);
            anyhow::bail!("the compositor gave no picture");
        }
    };
    // The stream: its thread says the node's number, or why there is none.
    let (numbered, node) = mpsc::channel::<Result<u32, String>>();
    {
        let (stop, frame, again) = (Arc::clone(&stop), Arc::clone(&frame), to_stream.clone());
        std::thread::Builder::new()
            .name("gevrek-share-stream".to_string())
            .spawn(move || {
                let told = numbered.clone();
                if let Err(error) = stream(size, beside, frame, from_capture, again, told) {
                    let _ = numbered.send(Err(format!("PipeWire: {error}")));
                }
                stop.store(true, Ordering::Relaxed);
            })
            .context("no thread for the stream")?;
    }
    let node = match node.recv_timeout(PATIENCE) {
        Ok(Ok(node)) => node,
        Ok(Err(why)) => {
            ended_early(&stop);
            anyhow::bail!(why);
        }
        Err(_) => {
            ended_early(&stop);
            let _ = to_stream.send(ToStream::Stop);
            anyhow::bail!("PipeWire did not take the stream");
        }
    };
    Ok(Cast {
        node,
        size,
        stop,
        to_stream,
    })
}

/// Take in what the compositor says, waiting at most `patience` for it.
fn pump(wayland: &mut Wayland, patience: Duration) -> anyhow::Result<()> {
    use std::os::fd::AsRawFd;
    let Wayland { queue, taking, .. } = wayland;
    queue.dispatch_pending(taking)?;
    queue.flush()?;
    if let Some(guard) = queue.prepare_read() {
        let mut waited = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one descriptor that lives as long as the guard, and its
        // count.
        let came = unsafe { libc::poll(&mut waited, 1, patience.as_millis() as libc::c_int) };
        if came > 0 {
            guard.read()?;
        }
    }
    queue.dispatch_pending(taking)?;
    Ok(())
}

/// The pictures' thread: one after another into `frame`, until `stop`
/// or until the compositor ends the session.
#[allow(clippy::too_many_arguments)]
fn capture(
    what: &What,
    with_pointer: bool,
    beside: bool,
    stop: &AtomicBool,
    frame: &Mutex<Frame>,
    to_stream: &pw::channel::Sender<ToStream>,
    described: mpsc::Sender<Result<(u32, u32), String>>,
) -> anyhow::Result<()> {
    let mut wayland = wayland()?;
    let handle = wayland.queue.handle();
    let source = match what {
        What::Output(name) => {
            let output = wayland
                .taking
                .outputs
                .iter()
                .find(|(_, known)| known.as_deref() == Some(name.as_str()))
                .map(|(output, _)| output.clone())
                .with_context(|| format!("there is no display {name}"))?;
            wayland.of_outputs.create_source(&output, &handle, ())
        }
        What::Window(identifier) => {
            let window = wayland
                .taking
                .windows
                .iter()
                .find(|window| window.identifier == *identifier)
                .map(|window| window.handle.clone())
                .context("the window is not there any more")?;
            wayland.of_windows.create_source(&window, &handle, ())
        }
    };
    let options = if with_pointer {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let session = wayland
        .manager
        .create_session(&source, options, &handle, ());
    // The pointer by itself, where it is handed on beside the pictures.
    let mut following = beside
        .then(|| Following::begin(&wayland, &source))
        .flatten();
    let mut memory: Option<(RawPool, wl_buffer::WlBuffer, (u32, u32))> = None;
    let mut told = false;
    let mut last = Instant::now() - Duration::from_secs(1);
    let began = Instant::now();
    while !stop.load(Ordering::Relaxed) && !wayland.taking.stopped {
        let size = wayland
            .taking
            .size
            .filter(|(width, height)| wayland.taking.described && *width > 0 && *height > 0);
        let Some(size) = size else {
            anyhow::ensure!(
                told || began.elapsed() < PATIENCE,
                "the compositor did not say how large the picture is"
            );
            pump(&mut wayland, Duration::from_millis(100))?;
            continue;
        };
        // Memory of the picture's size; a new one when the size changed.
        if memory.as_ref().map(|(.., of)| *of) != Some(size) {
            if let Some((_, buffer, _)) = memory.take() {
                buffer.destroy();
            }
            let stride = size.0 * 4;
            let mut pool = RawPool::new((stride * size.1) as usize, &wayland.taking.shm)
                .context("no memory for the picture")?;
            let buffer = pool.create_buffer(
                0,
                size.0 as i32,
                size.1 as i32,
                stride as i32,
                wl_shm::Format::Xrgb8888,
                (),
                &handle,
            );
            memory = Some((pool, buffer, size));
            if told {
                let _ = to_stream.send(ToStream::Size(size.0, size.1));
            } else {
                let _ = described.send(Ok(size));
                told = true;
            }
        }
        let Some((_, buffer, _)) = memory.as_ref() else {
            continue;
        };
        // No faster than the most a second.
        let rest = pause(last.elapsed(), MOST_A_SECOND);
        if !rest.is_zero() {
            std::thread::sleep(rest);
        }
        let asked = session.create_frame(&handle, ());
        asked.attach_buffer(buffer);
        asked.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        asked.capture();
        wayland.taking.frame = None;
        while wayland.taking.frame.is_none()
            && !wayland.taking.stopped
            && !stop.load(Ordering::Relaxed)
        {
            pump(&mut wayland, Duration::from_millis(100))?;
            if let Some(following) = following.as_mut() {
                following.step(&mut wayland, frame, to_stream);
            }
        }
        asked.destroy();
        match wayland.taking.frame {
            Some(true) => {
                last = Instant::now();
                let bytes = (size.0 * size.1 * 4) as usize;
                let Some((pool, ..)) = memory.as_mut() else {
                    continue;
                };
                let raw = pool.mmap();
                if raw.len() < bytes {
                    continue;
                }
                let mut frame = frame
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                frame.width = size.0;
                frame.height = size.1;
                frame.pixels.resize(bytes, 0);
                frame.pixels.copy_from_slice(&raw[..bytes]);
                drop(frame);
                let _ = to_stream.send(ToStream::Frame);
            }
            // The picture's size is another now (the session says the
            // new one), or the compositor could not draw it this once.
            Some(false) => std::thread::sleep(Duration::from_millis(20)),
            None => {}
        }
    }
    if let Some(following) = following {
        following.end();
    }
    session.destroy();
    source.destroy();
    let _ = wayland.queue.roundtrip(&mut wayland.taking);
    Ok(())
}

/// What the stream offers: pictures of this size, blue first.
fn format(width: u32, height: u32) -> Vec<u8> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::utils::{Fraction, Rectangle};
    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Id,
            spa::param::video::VideoFormat::BGRx
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle { width, height }
        ),
        // Pictures come when something changed, not by a clock.
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction { num: 0, denom: 1 }
        ),
        spa::pod::property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            Fraction {
                num: MOST_A_SECOND,
                denom: 1
            },
            Fraction { num: 1, denom: 1 },
            Fraction {
                num: MOST_A_SECOND,
                denom: 1
            }
        ),
    );
    serialized(spa::pod::Value::Object(object))
}

/// The memory a picture of this size needs, for PipeWire to set aside.
fn buffers(width: u32, height: u32) -> Vec<u8> {
    use spa::pod::{Property, PropertyFlags, Value};
    use spa::utils::{Choice, ChoiceEnum, ChoiceFlags};
    let whole = |key: u32, value: i32| Property {
        key,
        flags: PropertyFlags::empty(),
        value: Value::Int(value),
    };
    let stride = width as i32 * 4;
    let kinds = (1 << spa::sys::SPA_DATA_MemFd) | (1 << spa::sys::SPA_DATA_MemPtr);
    let object = spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_buffers,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 4,
                        min: 2,
                        max: 8,
                    },
                ))),
            },
            whole(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            whole(spa::sys::SPA_PARAM_BUFFERS_size, stride * height as i32),
            whole(spa::sys::SPA_PARAM_BUFFERS_stride, stride),
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_dataType,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: kinds,
                        flags: vec![kinds],
                    },
                ))),
            },
        ],
    };
    serialized(Value::Object(object))
}

fn serialized(value: spa::pod::Value) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
        .map(|(cursor, _)| cursor.into_inner())
        .unwrap_or_default()
}

/// What the stream's thread keeps between PipeWire's calls.
struct Kept {
    frame: Arc<Mutex<Frame>>,
    /// The size PipeWire and the taker agreed on.
    agreed: Option<(u32, u32)>,
    numbered: Option<mpsc::Sender<Result<u32, String>>>,
    /// To this thread's own loop: hand the newest picture on again.
    again: pw::channel::Sender<ToStream>,
    /// The pointer goes beside the pictures (as what PipeWire calls a
    /// buffer's metadata), not into them.
    beside: bool,
}

/// The side of the pointer's picture there is room for beside a
/// picture, at the most.
const POINTER_ROOM: u32 = 256;

/// How much room the pointer takes beside a picture: where it is, and
/// its own picture of this side.
fn pointer_room(side: u32) -> usize {
    std::mem::size_of::<spa::sys::spa_meta_cursor>()
        + std::mem::size_of::<spa::sys::spa_meta_bitmap>()
        + (side * side * 4) as usize
}

/// What the stream asks room for beside each picture: the pointer.
fn pointer_param() -> Vec<u8> {
    use spa::pod::{Property, PropertyFlags, Value};
    let object = spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamMeta,
        id: spa::sys::SPA_PARAM_Meta,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_META_type,
                flags: PropertyFlags::empty(),
                value: Value::Id(spa::utils::Id(spa::sys::SPA_META_Cursor)),
            },
            Property {
                key: spa::sys::SPA_PARAM_META_size,
                flags: PropertyFlags::empty(),
                value: Value::Int(pointer_room(POINTER_ROOM) as i32),
            },
        ],
    };
    serialized(Value::Object(object))
}

/// Write the pointer beside a picture.
///
/// # Safety
/// `buffer` is a buffer of the stream that is being filled.
unsafe fn write_pointer(buffer: *mut spa::sys::spa_buffer, seen: &Seen) {
    use spa::sys::{spa_meta_bitmap, spa_meta_cursor};
    let head = std::mem::size_of::<spa_meta_cursor>();
    let picture = (seen.side * seen.side * 4) as usize;
    let whole = seen.side > 0 && seen.side <= POINTER_ROOM && seen.pixels.len() >= picture;
    // With room for the picture where there is one to give; else for
    // where the pointer is alone.
    let mut meta = std::ptr::null_mut::<spa_meta_cursor>();
    if whole {
        meta = spa::sys::spa_buffer_find_meta_data(
            buffer,
            spa::sys::SPA_META_Cursor,
            pointer_room(seen.side),
        ) as *mut spa_meta_cursor;
    }
    let with_picture = !meta.is_null();
    if meta.is_null() {
        meta = spa::sys::spa_buffer_find_meta_data(buffer, spa::sys::SPA_META_Cursor, head)
            as *mut spa_meta_cursor;
    }
    if meta.is_null() {
        return;
    }
    let Some(at) = seen.at else {
        // Not over what is shared: no pointer.
        (*meta).id = 0;
        (*meta).bitmap_offset = 0;
        return;
    };
    (*meta).id = 1;
    (*meta).flags = 0;
    (*meta).position.x = at.0;
    (*meta).position.y = at.1;
    (*meta).hotspot.x = seen.hotspot.0;
    (*meta).hotspot.y = seen.hotspot.1;
    if !with_picture {
        (*meta).bitmap_offset = 0;
        return;
    }
    (*meta).bitmap_offset = head as u32;
    let bitmap = (meta as *mut u8).add(head) as *mut spa_meta_bitmap;
    (*bitmap).format = spa::sys::SPA_VIDEO_FORMAT_BGRA;
    (*bitmap).size.width = seen.side;
    (*bitmap).size.height = seen.side;
    (*bitmap).stride = (seen.side * 4) as i32;
    (*bitmap).offset = std::mem::size_of::<spa_meta_bitmap>() as u32;
    std::ptr::copy_nonoverlapping(
        seen.pixels.as_ptr(),
        (bitmap as *mut u8).add(std::mem::size_of::<spa_meta_bitmap>()),
        picture,
    );
}

/// The stream's thread: a node in PipeWire that hands the pictures on,
/// until it is told to stop.
fn stream(
    size: (u32, u32),
    beside: bool,
    frame: Arc<Mutex<Frame>>,
    from_capture: pw::channel::Receiver<ToStream>,
    again: pw::channel::Sender<ToStream>,
    numbered: mpsc::Sender<Result<u32, String>>,
) -> Result<(), pw::Error> {
    use pw::stream::{StreamFlags, StreamState};
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "gevrek-screencast",
        pw::properties::properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::NODE_NAME => "gevrek-screencast",
            *pw::keys::NODE_DESCRIPTION => "gevrek screen sharing",
        },
    )?;
    let kept = Kept {
        frame,
        agreed: None,
        numbered: Some(numbered),
        again,
        beside,
    };
    let _listener = stream
        .add_local_listener_with_user_data(kept)
        .state_changed(|stream, kept, _, new| match new {
            StreamState::Paused | StreamState::Streaming => {
                if let Some(numbered) = kept.numbered.take() {
                    let _ = numbered.send(Ok(stream.node_id()));
                }
                // Somebody takes the stream from now on: the picture
                // that is there is theirs at once (the next comes only
                // when something on the screen changes).
                if matches!(new, StreamState::Streaming) {
                    let _ = kept.again.send(ToStream::Frame);
                }
            }
            StreamState::Error(why) => {
                if let Some(numbered) = kept.numbered.take() {
                    let _ = numbered.send(Err(why));
                }
            }
            _ => {}
        })
        .param_changed(|stream, kept, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                kept.agreed = None;
                return;
            };
            let mut info = spa::param::video::VideoInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            let size = info.size();
            kept.agreed = Some((size.width, size.height));
            let bytes = buffers(size.width, size.height);
            let room = pointer_param();
            let mut params: Vec<&spa::pod::Pod> =
                spa::pod::Pod::from_bytes(&bytes).into_iter().collect();
            if kept.beside {
                params.extend(spa::pod::Pod::from_bytes(&room));
            }
            let _ = stream.update_params(&mut params);
        })
        .process(|stream, kept| {
            // SAFETY: a buffer of this stream (or none), handed back to
            // it below whatever comes in between.
            let taken = unsafe { stream.dequeue_raw_buffer() };
            if taken.is_null() {
                return;
            }
            // SAFETY: PipeWire's own buffer, valid until it is queued.
            let buffer = unsafe { (*taken).buffer };
            let frame = kept
                .frame
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // SAFETY: the buffer's first data and its chunk are read and
            // written within what PipeWire says they hold (`maxsize`).
            unsafe {
                if !buffer.is_null() && (*buffer).n_datas > 0 && !(*buffer).datas.is_null() {
                    // The pointer beside the picture, where it is asked
                    // for so.
                    if let (true, Some(seen)) = (kept.beside, frame.pointer.as_ref()) {
                        write_pointer(buffer, seen);
                    }
                    let data = &mut *(*buffer).datas;
                    let stride = frame.width as usize * 4;
                    let bytes = stride * frame.height as usize;
                    // (A picture of another size than agreed on waits for
                    // the new agreement: nothing is handed on.)
                    let fits = kept.agreed == Some((frame.width, frame.height))
                        && frame.pixels.len() >= bytes
                        && bytes > 0;
                    let written = if fits && !data.data.is_null() && data.maxsize as usize >= bytes
                    {
                        std::ptr::copy_nonoverlapping(
                            frame.pixels.as_ptr(),
                            data.data as *mut u8,
                            bytes,
                        );
                        bytes
                    } else {
                        0
                    };
                    if !data.chunk.is_null() {
                        (*data.chunk).offset = 0;
                        (*data.chunk).stride = stride as i32;
                        (*data.chunk).size = written as u32;
                    }
                }
                stream.queue_raw_buffer(taken);
            }
        })
        .register()?;
    let offered = format(size.0, size.1);
    let mut params: Vec<&spa::pod::Pod> = spa::pod::Pod::from_bytes(&offered).into_iter().collect();
    stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::DRIVER | StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    let (told, quit) = (stream.clone(), mainloop.clone());
    let _attached = from_capture.attach(mainloop.loop_(), move |message| match message {
        ToStream::Frame => {
            if told.state() == StreamState::Streaming {
                let _ = told.trigger_process();
            }
        }
        ToStream::Size(width, height) => {
            let offered = format(width, height);
            if let Some(pod) = spa::pod::Pod::from_bytes(&offered) {
                let _ = told.update_params(&mut [pod]);
            }
        }
        ToStream::Stop => quit.quit(),
        // (Only where the card's buffers are shared.)
        ToStream::Want | ToStream::Filled(_) | ToStream::Unused(_) => {}
    });
    mainloop.run();
    Ok(())
}

impl ShmHandler for Taking {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Taking {
    fn event(
        taking: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some((_, known)) = taking
                .outputs
                .iter_mut()
                .find(|(output, _)| output == proxy)
            {
                *known = Some(name);
            }
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
            taking.windows.push(Window {
                handle: toplevel,
                identifier: String::new(),
                title: String::new(),
                app_id: String::new(),
            });
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
        use ext_foreign_toplevel_handle_v1::Event;
        if matches!(event, Event::Closed) {
            taking.windows.retain(|window| window.handle != *proxy);
            return;
        }
        let Some(window) = taking
            .windows
            .iter_mut()
            .find(|window| window.handle == *proxy)
        else {
            return;
        };
        match event {
            Event::Title { title } => window.title = title,
            Event::AppId { app_id } => window.app_id = app_id,
            Event::Identifier { identifier } => window.identifier = identifier,
            _ => {}
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
            ext_image_copy_capture_session_v1::Event::DmabufDevice { device } => {
                taking.card = <[u8; 8]>::try_from(device.as_slice())
                    .ok()
                    .map(u64::from_ne_bytes);
            }
            ext_image_copy_capture_session_v1::Event::DmabufFormat { format, modifiers }
                if format == card::XRGB =>
            {
                taking.card_layouts = modifiers
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|bytes| u64::from_ne_bytes(*bytes))
                    .collect();
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
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                // What the pictures were of is gone: no more will come.
                if matches!(
                    reason,
                    wayland_client::WEnum::Value(
                        ext_image_copy_capture_frame_v1::FailureReason::Stopped
                    )
                ) {
                    taking.stopped = true;
                }
                taking.frame = Some(false);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, OfPointer> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &OfPointer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                taking.pointer.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::Done => taking.pointer.described = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, OfPointer> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &OfPointer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => taking.pointer.frame = Some(true),
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                taking.pointer.frame = Some(false);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for Taking {
    fn event(
        taking: &mut Self,
        _: &ExtImageCopyCaptureCursorSessionV1,
        event: ext_image_copy_capture_cursor_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_cursor_session_v1::Event;
        let told = &mut taking.pointer;
        match event {
            Event::Enter => told.inside = true,
            Event::Leave => told.inside = false,
            Event::Position { x, y } => told.at = (x, y),
            Event::Hotspot { x, y } => told.hotspot = (x, y),
            _ => return,
        }
        told.changed = true;
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
quiet!(wl_buffer::WlBuffer, ());
quiet!(ZwpLinuxDmabufV1, ());
quiet!(ZwpLinuxBufferParamsV1, ());
quiet!(wl_seat::WlSeat, ());
quiet!(wl_pointer::WlPointer, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

smithay_client_toolkit::delegate_shm!(Taking);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_asked_for_no_faster_than_the_most_a_second() {
        // Sixty a second: a sixtieth between two, less what has passed.
        let sixtieth = Duration::from_secs(1) / 60;
        assert_eq!(pause(Duration::ZERO, 60), sixtieth);
        assert_eq!(
            pause(Duration::from_millis(10), 60),
            sixtieth - Duration::from_millis(10)
        );
        assert_eq!(pause(Duration::from_millis(40), 60), Duration::ZERO);
        assert_eq!(pause(Duration::ZERO, 0), Duration::from_secs(1));
    }

    #[test]
    fn what_the_stream_offers_and_asks_for_is_written() {
        // Both descriptions are whole pods that PipeWire's reader takes.
        let offered = format(1920, 1080);
        let pod = spa::pod::Pod::from_bytes(&offered).expect("a pod");
        let mut info = spa::param::video::VideoInfoRaw::new();
        info.parse(pod).expect("a video format");
        assert_eq!((info.size().width, info.size().height), (1920, 1080));
        assert_eq!(info.format(), spa::param::video::VideoFormat::BGRx);
        assert!(spa::pod::Pod::from_bytes(&buffers(1920, 1080)).is_some());
    }
}
