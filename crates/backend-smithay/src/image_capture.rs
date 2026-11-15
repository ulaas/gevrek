//! Pictures of an output or of one window for other programs
//! (**ext-image-copy-capture** with **ext-image-capture-source**): the
//! newer way screenshot tools, recorders and the screen-sharing portal ask,
//! and the one by which a single window can be had.
//!
//! A program names what it wants a picture of (an output, or a window by
//! its entry in the window list), opens a session and is told how large a
//! buffer must be; for each frame it hands a buffer over and is told when
//! the picture is in it. The first frame of a session comes at once; the
//! next ones when something changed on the output the picture is of, so a
//! recorder follows the display without asking faster than it changes.
//!
//! Only shared-memory buffers (as with wlr-screencopy, `capture.rs`), and
//! no session of the pointer's own picture: such a session is told at once
//! that it has stopped.

use crate::capture::CaptureTarget;
use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::WindowHandle;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::{
    ext_foreign_toplevel_image_capture_source_manager_v1::{
        self, ExtForeignToplevelImageCaptureSourceManagerV1,
    },
    ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
    ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
};
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::{
    ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What a picture is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Output(MonitorHandle),
    Window(WindowHandle),
    /// The pointer's own picture, as fine as this display is.
    Pointer(MonitorHandle),
}

/// The side of the pointer's picture on a display of this scale, in
/// the display's own pixels: room for a pointer of 64 points.
pub fn pointer_side(scale: f64) -> i32 {
    (64.0 * scale.max(1.0)).ceil() as i32
}

/// A program that follows the pointer over a display or a window.
pub struct PointerWatch {
    session: ExtImageCopyCaptureCursorSessionV1,
    source: Source,
    /// What it was last told: over the thing or not, where, and the
    /// hot spot.
    inside: bool,
    at: Option<(i32, i32)>,
    hotspot: Option<(i32, i32)>,
}

/// Where a point of the desktop is in a picture of something that
/// stands at `place` (points) on a display of this scale: in the
/// picture's own pixels, and whether it is in the picture at all.
pub fn in_picture(
    point: (i32, i32),
    place: (i32, i32, i32, i32),
    scale: f64,
) -> ((i32, i32), bool) {
    let (x, y) = (point.0 - place.0, point.1 - place.1);
    let inside = x >= 0 && y >= 0 && x < place.2 && y < place.3;
    (
        (
            (f64::from(x) * scale).round() as i32,
            (f64::from(y) * scale).round() as i32,
        ),
        inside,
    )
}

/// A session: what it is of, and how its frames are made.
pub struct Session {
    /// `None`: of something gevrek has no pictures of (it has stopped).
    source: Option<Source>,
    with_pointer: bool,
    /// How many frames it has had: the first comes at once, the others
    /// wait for a change.
    frames: AtomicU32,
    /// The count of changes of what it is of (see [`State::changes_of`])
    /// at its last picture: a change that came between two frames is in
    /// the next one, not lost.
    seen: AtomicU64,
    /// The pointer's picture it was last given, by its hash: the next
    /// frame of the pointer waits for another picture.
    shown: AtomicU64,
}

/// A frame: its session, and the buffer it was handed.
pub struct Frame {
    session: Arc<Session>,
    buffer: Mutex<Option<WlBuffer>>,
    captured: AtomicBool,
}

/// A frame whose buffer waits for its picture.
pub struct PendingImage {
    frame: ExtImageCopyCaptureFrameV1,
    buffer: WlBuffer,
    source: Source,
    with_pointer: bool,
    /// Wait until something changed on the output the picture is of.
    wait: bool,
    session: Arc<Session>,
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
    display.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, ()>(1, ());
    display.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
}

impl State {
    /// How large a picture of `source` is now; `None` when it is gone.
    fn image_size(&self, source: Source) -> Option<(i32, i32)> {
        match source {
            Source::Output(output) => self.output_capture_size(output),
            Source::Window(window) => self.window_capture_size(window),
            Source::Pointer(output) => self.device_size(output).map(|(_, _, scale)| {
                let side = pointer_side(scale);
                (side, side)
            }),
        }
    }

    /// Tell the programs that follow the pointer where it is over what
    /// they watch, and where its hot spot is in its picture.
    pub(crate) fn tell_pointer_watches(&mut self) {
        if self.pointer_watches.is_empty() {
            return;
        }
        let mut watches = std::mem::take(&mut self.pointer_watches);
        watches.retain(|watch| watch.session.is_alive());
        for watch in &mut watches {
            let on = self.output_of_source(watch.source);
            let scale = on
                .and_then(|on| self.device_size(on))
                .map_or(1.0, |(_, _, scale)| scale);
            // Where the thing stands on the desktop, in points.
            let place = match watch.source {
                Source::Output(output) | Source::Pointer(output) => self
                    .desktop
                    .output_rectangle(output)
                    .map(|screen| (screen.x, screen.y, screen.width, screen.height)),
                Source::Window(window) => self
                    .desktop
                    .windows
                    .get(&window)
                    .filter(|record| record.mapped)
                    .map(|record| {
                        (
                            record.rect.x,
                            record.rect.y,
                            record.rect.width,
                            record.rect.height,
                        )
                    }),
            };
            let (at, inside) = match place {
                Some(place) => in_picture(self.desktop.pointer, place, scale),
                None => ((0, 0), false),
            };
            if inside != watch.inside {
                watch.inside = inside;
                if inside {
                    watch.session.enter();
                } else {
                    watch.session.leave();
                    watch.at = None;
                }
            }
            if !inside {
                continue;
            }
            if watch.at != Some(at) {
                watch.at = Some(at);
                watch.session.position(at.0, at.1);
            }
            let hotspot = on.map_or((0, 0), |on| self.pointer_hotspot(on));
            let hotspot = (
                (f64::from(hotspot.0) * scale).round() as i32,
                (f64::from(hotspot.1) * scale).round() as i32,
            );
            if watch.hotspot != Some(hotspot) {
                watch.hotspot = Some(hotspot);
                watch.session.hotspot(hotspot.0, hotspot.1);
            }
        }
        self.pointer_watches = watches;
    }

    /// Draw the pointer's picture alone, as fine as a display is, into
    /// an image of its own and read it back: its size and its pixels,
    /// four bytes each, red first, see-through where the pointer is not.
    fn draw_pointer_for_capture<R, T>(
        &mut self,
        renderer: &mut R,
        format: Fourcc,
        output: MonitorHandle,
    ) -> anyhow::Result<crate::capture::Drawn>
    where
        R: Renderer
            + ImportAll
            + ImportMem
            + Offscreen<T>
            + Bind<T>
            + ExportMem
            + crate::clip::ClipRenderer,
        R::TextureId: Send + Clone + 'static,
        R::Error: std::fmt::Debug,
        T: 'static,
    {
        use smithay::backend::renderer::damage::OutputDamageTracker;
        use smithay::backend::renderer::element::utils::RescaleRenderElement;
        use smithay::utils::{Rectangle, Transform};
        let (_, _, scale) = self
            .device_size(output)
            .ok_or_else(|| anyhow::anyhow!("the output is gone"))?;
        let side = pointer_side(scale);
        let size = (side, side);
        let mut target = self
            .pointer_target
            .take()
            .and_then(|kept| kept.downcast::<CaptureTarget<T>>().ok())
            .unwrap_or_default();
        let drawn = (|| {
            let image = target.image_of(renderer, format, size)?;
            // (Its hot spot where it is in the picture: the picture's
            // corner is the pointer's own.)
            let hotspot = self.pointer_hotspot(output);
            let mut elements = Vec::new();
            self.pointer_elements(renderer, output, hotspot.into(), true, &mut elements);
            let elements: Vec<_> = elements
                .into_iter()
                .map(|element| RescaleRenderElement::from_element(element, (0, 0).into(), scale))
                .collect();
            let mut framebuffer = renderer
                .bind(image)
                .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
            OutputDamageTracker::new(size, 1.0, Transform::Normal)
                .render_output(
                    renderer,
                    &mut framebuffer,
                    0,
                    &elements,
                    smithay::backend::renderer::Color32F::TRANSPARENT,
                )
                .map_err(|error| anyhow::anyhow!("render: {error:?}"))?;
            let mapping = renderer
                .copy_framebuffer(
                    &framebuffer,
                    Rectangle::from_size(size.into()),
                    Fourcc::Abgr8888,
                )
                .map_err(|error| anyhow::anyhow!("read back: {error:?}"))?;
            let pixels = renderer
                .map_texture(&mapping)
                .map_err(|error| anyhow::anyhow!("map: {error:?}"))?;
            Ok((size, pixels.to_vec()))
        })();
        self.pointer_target = Some(target);
        drawn
    }

    /// The output a change of which is a change of `source`.
    /// How often something was drawn anew on an output, as far as the
    /// captures were told.
    pub(crate) fn changes_of(&self, output: MonitorHandle) -> u64 {
        self.capture_changes_everywhere + self.capture_changes.get(&output).copied().unwrap_or(0)
    }

    fn output_of_source(&self, source: Source) -> Option<MonitorHandle> {
        match source {
            Source::Output(output) | Source::Pointer(output) => Some(output),
            Source::Window(window) => self.surface_outputs.get(&window).map(|(output, _)| *output),
        }
    }

    /// Give the waiting frames their pictures (see
    /// [`State::serve_captures`], which calls this).
    pub(crate) fn serve_image_captures<R, T: 'static>(
        &mut self,
        renderer: &mut R,
        target: &mut CaptureTarget<T>,
        format: Fourcc,
        changed: Option<&[MonitorHandle]>,
    ) where
        R: Renderer
            + ImportAll
            + ImportMem
            + Offscreen<T>
            + Bind<T>
            + ExportMem
            + crate::clip::ClipRenderer,
        R::TextureId: Send + Clone + 'static,
        R::Error: std::fmt::Debug,
    {
        // What was drawn anew is counted, whether or not a frame waits:
        // a session's next frame is asked for a moment after its last.
        match changed {
            Some(changed) => {
                for output in changed {
                    *self.capture_changes.entry(*output).or_default() += 1;
                }
            }
            None => self.capture_changes_everywhere += 1,
        }
        self.tell_pointer_watches();
        if self.image_captures.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.image_captures);
        // One drawing serves every frame of the same thing and kind.
        let mut drawn: Vec<((Source, bool), Option<crate::capture::Drawn>)> = Vec::new();
        for capture in waiting {
            if !capture.frame.is_alive() {
                continue;
            }
            let Some(size) = self.image_size(capture.source) else {
                capture.frame.failed(FailureReason::Stopped);
                continue;
            };
            let on = self.output_of_source(capture.source);
            let changes = on.map(|on| self.changes_of(on));
            if capture.wait
                && changes.is_some_and(|now| capture.session.seen.load(Ordering::SeqCst) >= now)
            {
                self.image_captures.push(capture);
                continue;
            }
            if let Some(now) = changes {
                capture.session.seen.store(now, Ordering::SeqCst);
            }
            // A buffer of the graphics card's own memory: the output is
            // drawn straight into it.
            if let (Source::Output(output), Ok(card)) = (
                capture.source,
                smithay::wayland::dmabuf::get_dmabuf(&capture.buffer),
            ) {
                let mut card = card.clone();
                match self.draw_into_card(renderer, &mut card, output, capture.with_pointer) {
                    Ok(()) => {
                        capture.frame.transform(
                            smithay::reexports::wayland_server::protocol::wl_output::Transform::Normal,
                        );
                        capture.frame.damage(0, 0, size.0, size.1);
                        let now = crate::capture::monotonic_now();
                        capture.frame.presentation_time(
                            (now.as_secs() >> 32) as u32,
                            (now.as_secs() & 0xffff_ffff) as u32,
                            now.subsec_nanos(),
                        );
                        capture.frame.ready();
                    }
                    Err(error) => {
                        log::debug!("no picture into a program's buffer: {error}");
                        capture.frame.failed(FailureReason::BufferConstraints);
                    }
                }
                continue;
            }
            let key = (capture.source, capture.with_pointer);
            if !drawn.iter().any(|(known, _)| *known == key) {
                let picture = match capture.source {
                    Source::Output(output) => {
                        self.draw_for_capture(renderer, target, format, output, key.1)
                    }
                    Source::Window(window) => {
                        self.draw_window_for_capture(renderer, target, format, window)
                    }
                    Source::Pointer(output) => {
                        self.draw_pointer_for_capture::<R, T>(renderer, format, output)
                    }
                };
                if let Err(error) = &picture {
                    log::warn!("no picture for a program: {error}");
                }
                drawn.push((key, picture.ok()));
            }
            let picture = drawn
                .iter()
                .find(|(known, _)| *known == key)
                .and_then(|(_, picture)| picture.as_ref());
            let of_pointer = matches!(capture.source, Source::Pointer(_));
            // The pointer's picture: a frame after the first waits for
            // another picture than the last one it was given.
            if let (true, Some((_, pixels))) = (of_pointer, picture) {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                pixels.hash(&mut hasher);
                let now = hasher.finish().max(1);
                if capture.wait && capture.session.shown.load(Ordering::SeqCst) == now {
                    self.image_captures.push(capture);
                    continue;
                }
                capture.session.shown.store(now, Ordering::SeqCst);
            }
            let copied = picture.is_some_and(|(drawn_size, pixels)| {
                *drawn_size == size
                    && crate::capture::copy_pixels_as(
                        &capture.buffer,
                        (0, 0, size.0, size.1),
                        *drawn_size,
                        pixels,
                        of_pointer,
                    )
            });
            if !copied {
                // A buffer of another size than the picture: the program
                // made it for what the picture was before.
                capture.frame.failed(FailureReason::BufferConstraints);
                continue;
            }
            capture.frame.transform(
                smithay::reexports::wayland_server::protocol::wl_output::Transform::Normal,
            );
            capture.frame.damage(0, 0, size.0, size.1);
            let now = crate::capture::monotonic_now();
            capture.frame.presentation_time(
                (now.as_secs() >> 32) as u32,
                (now.as_secs() & 0xffff_ffff) as u32,
                now.subsec_nanos(),
            );
            capture.frame.ready();
        }
    }
}

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtOutputImageCaptureSourceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtOutputImageCaptureSourceManagerV1,
        request: ext_output_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource {
            source,
            output,
        } = request
        {
            let known = Output::from_resource(&output).and_then(|output| {
                state
                    .outputs
                    .iter()
                    .find(|(_, candidate)| *candidate == output)
                    .map(|(id, _)| Source::Output(*id))
            });
            init.init(source, known);
        }
    }
}

impl GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            // The window whose entry in the window list this is.
            let known = ForeignToplevelHandle::from_resource(&toplevel_handle).and_then(|handle| {
                let wanted = handle.identifier();
                state
                    .foreign_handles
                    .iter()
                    .find(|(_, candidate)| candidate.identifier() == wanted)
                    .map(|(window, _)| Source::Window(*window))
            });
            init.init(source, known);
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, Option<Source>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtImageCaptureSourceV1,
        _: ext_image_capture_source_v1::Request,
        _: &Option<Source>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtImageCopyCaptureManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession {
                session,
                source,
                options,
            } => {
                let with_pointer = match options {
                    WEnum::Value(options) => options.contains(Options::PaintCursors),
                    WEnum::Unknown(_) => {
                        manager.post_error(
                            ext_image_copy_capture_manager_v1::Error::InvalidOption,
                            "an option this version does not know",
                        );
                        return;
                    }
                };
                let source = source.data::<Option<Source>>().copied().flatten();
                let size = source.and_then(|source| state.image_size(source));
                let session = init.init(
                    session,
                    Arc::new(Session {
                        source: source.filter(|_| size.is_some()),
                        with_pointer,
                        frames: AtomicU32::new(0),
                        seen: AtomicU64::new(0),
                        shown: AtomicU64::new(0),
                    }),
                );
                match size {
                    Some((width, height)) => {
                        session.buffer_size(width as u32, height as u32);
                        session.shm_format(wl_shm::Format::Xrgb8888);
                        // An output may also be had straight in a buffer
                        // of the graphics card, where the renderer draws
                        // into such.
                        if let (Some(Source::Output(_)), Some((device, kinds))) =
                            (source, &state.capture_card)
                        {
                            session.dmabuf_device(device.to_ne_bytes().to_vec());
                            for (kind, layouts) in kinds {
                                session.dmabuf_format(
                                    *kind,
                                    layouts
                                        .iter()
                                        .flat_map(|layout| layout.to_ne_bytes())
                                        .collect(),
                                );
                            }
                        }
                        session.done();
                    }
                    // An output or a window that is gone.
                    None => session.stopped(),
                }
            }
            // The pointer by itself, over a display or a window: where
            // it is there, and (as a session of its own) its picture.
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession {
                session,
                source,
                ..
            } => {
                let source = source.data::<Option<Source>>().copied().flatten();
                let session = init.init(session, source);
                if let Some(source) = source {
                    state.pointer_watches.push(PointerWatch {
                        session,
                        source,
                        inside: false,
                        at: None,
                        hotspot: None,
                    });
                    state.tell_pointer_watches();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, Arc<Session>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        session: &Arc<Session>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            init.init(
                frame,
                Frame {
                    session: Arc::clone(session),
                    buffer: Mutex::new(None),
                    captured: AtomicBool::new(false),
                },
            );
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, Frame> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        frame: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &Frame,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_image_copy_capture_frame_v1::Request::AttachBuffer { buffer } => {
                *data
                    .buffer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(buffer);
            }
            ext_image_copy_capture_frame_v1::Request::Capture => {
                if data.captured.swap(true, Ordering::SeqCst) {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::AlreadyCaptured,
                        "the frame was captured already",
                    );
                    return;
                }
                let buffer = data
                    .buffer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                let Some(buffer) = buffer else {
                    frame.post_error(
                        ext_image_copy_capture_frame_v1::Error::NoBuffer,
                        "capture without a buffer",
                    );
                    return;
                };
                let Some(source) = data.session.source else {
                    frame.failed(FailureReason::Stopped);
                    return;
                };
                let had = data.session.frames.fetch_add(1, Ordering::SeqCst);
                state.image_captures.push(PendingImage {
                    frame: frame.clone(),
                    buffer,
                    source,
                    with_pointer: data.session.with_pointer,
                    wait: had > 0,
                    session: Arc::clone(&data.session),
                });
                // The first picture of a session is drawn for it now; and
                // so is one of something that changed since its last.
                let behind = state.output_of_source(source).is_some_and(|on| {
                    state.changes_of(on) > data.session.seen.load(Ordering::SeqCst)
                });
                if had == 0 || behind {
                    state.desktop.needs_redraw = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, Option<Source>> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        source: &Option<Source>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        // The pointer's own picture, as fine as the display is that
        // what is watched is on; of something that is gone, a session
        // that has stopped from the start.
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } =
            request
        {
            let of = source
                .and_then(|source| state.output_of_source(source))
                .map(Source::Pointer);
            let size = of.and_then(|of| state.image_size(of));
            let session = init.init(
                session,
                Arc::new(Session {
                    source: of.filter(|_| size.is_some()),
                    with_pointer: true,
                    frames: AtomicU32::new(0),
                    seen: AtomicU64::new(0),
                    shown: AtomicU64::new(0),
                }),
            );
            match size {
                Some((width, height)) => {
                    session.buffer_size(width as u32, height as u32);
                    session.shm_format(wl_shm::Format::Argb8888);
                    session.done();
                }
                None => session.stopped(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointers_place_in_a_picture_is_in_the_pictures_own_pixels() {
        // A display that stands beside another, at twice the scale.
        let display = (1280, 0, 640, 360);
        assert_eq!(
            in_picture((1280 + 50, 60), display, 2.0),
            ((100, 120), true)
        );
        assert_eq!(in_picture((1280, 0), display, 2.0), ((0, 0), true));
        // Its last point is in it; the one beyond is not.
        assert!(in_picture((1280 + 639, 359), display, 2.0).1);
        assert!(!in_picture((1280 + 640, 100), display, 2.0).1);
        assert!(
            !in_picture((300, 200), display, 2.0).1,
            "over the other display"
        );
        // A window at a scale of one and a half.
        assert_eq!(
            in_picture((140, 130), (100, 100, 400, 300), 1.5),
            ((60, 45), true)
        );
    }

    #[test]
    fn the_pointers_picture_has_room_for_it_at_every_scale() {
        assert_eq!(pointer_side(1.0), 64);
        assert_eq!(pointer_side(2.0), 128);
        assert_eq!(pointer_side(1.5), 96);
        assert_eq!(pointer_side(0.5), 64, "never smaller than at one");
    }
}
