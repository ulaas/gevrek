//! Pictures of the outputs for other programs (**wlr-screencopy**): what
//! `grim` takes a screenshot with, and what `wf-recorder` and OBS record
//! through.
//!
//! A program asks for a frame of an output (all of it, or a part); it is
//! told the size and the format, hands over a buffer of shared memory of
//! that size, and gets the picture copied into it the next time the output
//! is drawn. `copy` asks for a picture now; `copy_with_damage` waits until
//! something on the output changes, which is how a recorder follows the
//! display without asking faster than it changes.
//!
//! The picture is drawn for the program, apart from what goes to the
//! display: with the pointer or without it, as asked. While the session is
//! locked it shows what the display shows, the lock.
//!
//! Only shared-memory buffers; a recorder that wants a dmabuf falls back
//! to them.

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::{Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer};
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Rectangle, Transform};
use smithay::wayland::shm;
use std::sync::atomic::{AtomicBool, Ordering};

/// The version of the protocol offered: 3 has `buffer_done`.
const VERSION: u32 = 3;

/// What a frame was asked for.
pub struct FrameData {
    /// The output, when it was one of ours when asked.
    output: Option<MonitorHandle>,
    /// The part of the output, in its device pixels.
    region: (i32, i32, i32, i32),
    with_pointer: bool,
    /// A frame is copied once.
    used: AtomicBool,
}

/// A frame whose buffer waits for the next drawing of its output.
pub struct PendingCapture {
    pub(crate) frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    pub(crate) output: MonitorHandle,
    region: (i32, i32, i32, i32),
    with_pointer: bool,
    with_damage: bool,
}

/// The image the outputs are drawn into for the programs, kept between
/// frames so a recorder does not cost an allocation per frame.
pub struct CaptureTarget<T> {
    image: Option<((i32, i32), T)>,
}

impl<T> Default for CaptureTarget<T> {
    fn default() -> Self {
        Self { image: None }
    }
}

impl<T> CaptureTarget<T> {
    /// The image, of this size (made anew when it was another).
    pub(crate) fn image_of<R>(
        &mut self,
        renderer: &mut R,
        format: Fourcc,
        size: (i32, i32),
    ) -> anyhow::Result<&mut T>
    where
        R: Renderer + Offscreen<T>,
        R::Error: std::fmt::Debug,
    {
        if self.image.as_ref().map(|(known, _)| *known) != Some(size) {
            let image = renderer
                .create_buffer(format, size.into())
                .map_err(|error| anyhow::anyhow!("an image to draw into: {error:?}"))?;
            self.image = Some((size, image));
        }
        Ok(&mut self.image.as_mut().expect("just made").1)
    }
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, ZwlrScreencopyManagerV1, ()>(VERSION, ());
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let (frame, overlay_cursor, output, part) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (frame, overlay_cursor, output, Some((x, y, width, height))),
            _ => return,
        };
        let known = Output::from_resource(&output).and_then(|output| {
            state
                .outputs
                .iter()
                .find(|(_, candidate)| *candidate == output)
                .map(|(id, _)| *id)
        });
        let region = known.and_then(|id| state.capture_region(id, part));
        let frame = init.init(
            frame,
            FrameData {
                output: known.filter(|_| region.is_some()),
                region: region.unwrap_or_default(),
                with_pointer: overlay_cursor != 0,
                used: AtomicBool::new(false),
            },
        );
        let Some((_, _, width, height)) = region else {
            // An output that is gone, or a part outside of it.
            frame.failed();
            return;
        };
        frame.buffer(
            wl_shm::Format::Xrgb8888,
            width as u32,
            height as u32,
            width as u32 * 4,
        );
        if frame.version() >= 3 {
            // All of an output may also be had straight in a buffer of
            // the graphics card, where the renderer draws into such.
            let whole = known
                .and_then(|id| state.output_capture_size(id))
                .is_some_and(|size| size == (width, height));
            if whole && state.capture_card.is_some() {
                frame.linux_dmabuf(
                    smithay::backend::allocator::Fourcc::Xrgb8888 as u32,
                    width as u32,
                    height as u32,
                );
            }
            frame.buffer_done();
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _display: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        let Some(output) = data.output else {
            // It was told `failed` when it was made.
            return;
        };
        if data.used.swap(true, Ordering::Relaxed) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "the frame was copied already",
            );
            return;
        }
        let (_, _, width, height) = data.region;
        // (A buffer of the graphics card, of the whole output's size.)
        let of_the_card = smithay::wayland::dmabuf::get_dmabuf(&buffer).is_ok_and(|card| {
            use smithay::backend::allocator::Buffer as _;
            state.capture_card.is_some()
                && state.output_capture_size(output) == Some((width, height))
                && (card.width() as i32, card.height() as i32) == (width, height)
        });
        let fits = of_the_card
            || shm::with_buffer_contents(&buffer, |_, _, description| {
                description.format == wl_shm::Format::Xrgb8888
                    && description.width == width
                    && description.height == height
                    && description.stride == width * 4
            })
            .unwrap_or(false);
        if !fits {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "the buffer is not the one the frame was described with",
            );
            return;
        }
        state.captures.push(PendingCapture {
            frame: frame.clone(),
            buffer,
            output,
            region: data.region,
            with_pointer: data.with_pointer,
            with_damage,
        });
        // A picture asked for now is not kept waiting for a change.
        if !with_damage {
            state.desktop.needs_redraw = true;
        }
    }
}

/// A picture that was drawn for a program: its size and its pixels.
pub(crate) type Drawn = ((i32, i32), Vec<u8>);

impl State {
    /// The size of an output in its device pixels.
    pub(crate) fn device_size(&self, output: MonitorHandle) -> Option<(i32, i32, f64)> {
        let record = self
            .desktop
            .outputs
            .iter()
            .find(|record| record.id == output)?;
        Some((
            (f64::from(record.rect.width) * record.scale).round() as i32,
            (f64::from(record.rect.height) * record.scale).round() as i32,
            record.scale,
        ))
    }

    /// The part of an output a frame is of, in device pixels: all of it,
    /// or the asked part (given in logical pixels of the output) as far as
    /// it lies on the output. `None` when nothing is left.
    fn capture_region(
        &self,
        output: MonitorHandle,
        part: Option<(i32, i32, i32, i32)>,
    ) -> Option<(i32, i32, i32, i32)> {
        let (width, height, scale) = self.device_size(output)?;
        let (x, y, part_width, part_height) = match part {
            None => (0, 0, width, height),
            Some((x, y, part_width, part_height)) => {
                let device = |value: i32| (f64::from(value) * scale).round() as i32;
                (
                    device(x),
                    device(y),
                    device(part_width),
                    device(part_height),
                )
            }
        };
        let left = x.max(0);
        let top = y.max(0);
        let right = (x.saturating_add(part_width)).min(width);
        let bottom = (y.saturating_add(part_height)).min(height);
        (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
    }

    /// Give the waiting frames their pictures. Called by a backend after
    /// it has drawn; `format` is a format its renderer can draw into.
    /// `changed` are the outputs on which something was drawn anew (`None`:
    /// all of them): a frame that waits for a change stays waiting on the
    /// others.
    pub fn serve_captures<R, T: 'static>(
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
        // (The newer protocol's frames, of outputs and of single windows.)
        self.serve_image_captures(renderer, target, format, changed);
        if self.captures.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.captures);
        // One drawing serves every frame of the same output and kind.
        let mut drawn: Vec<((MonitorHandle, bool), Option<Drawn>)> = Vec::new();
        for capture in waiting {
            if !capture.frame.is_alive() {
                continue;
            }
            let gone = !self
                .desktop
                .outputs
                .iter()
                .any(|record| record.id == capture.output);
            if capture.with_damage
                && !gone
                && changed.is_some_and(|changed| !changed.contains(&capture.output))
            {
                self.captures.push(capture);
                continue;
            }
            // A buffer of the graphics card's own memory: the output is
            // drawn straight into it.
            if let Ok(card) = smithay::wayland::dmabuf::get_dmabuf(&capture.buffer) {
                let mut card = card.clone();
                let done = self
                    .draw_into_card(renderer, &mut card, capture.output, capture.with_pointer)
                    .map_err(|error| log::debug!("no picture into a program's buffer: {error}"))
                    .is_ok();
                if !done {
                    capture.frame.failed();
                    continue;
                }
                capture
                    .frame
                    .flags(zwlr_screencopy_frame_v1::Flags::empty());
                if capture.with_damage {
                    capture
                        .frame
                        .damage(0, 0, capture.region.2 as u32, capture.region.3 as u32);
                }
                let now = monotonic_now();
                capture.frame.ready(
                    (now.as_secs() >> 32) as u32,
                    (now.as_secs() & 0xffff_ffff) as u32,
                    now.subsec_nanos(),
                );
                self.picture_taken(capture.output);
                continue;
            }
            let key = (capture.output, capture.with_pointer);
            if !drawn.iter().any(|(known, _)| *known == key) {
                let picture = match self.draw_for_capture(renderer, target, format, key.0, key.1) {
                    Ok(picture) => Some(picture),
                    Err(error) => {
                        log::warn!("no picture of the output for a program: {error}");
                        None
                    }
                };
                drawn.push((key, picture));
            }
            let picture = drawn
                .iter()
                .find(|(known, _)| *known == key)
                .and_then(|(_, picture)| picture.as_ref());
            let copied = picture.is_some_and(|(size, pixels)| copy_into(&capture, *size, pixels));
            if !copied {
                capture.frame.failed();
                continue;
            }
            capture
                .frame
                .flags(zwlr_screencopy_frame_v1::Flags::empty());
            if capture.with_damage {
                capture
                    .frame
                    .damage(0, 0, capture.region.2 as u32, capture.region.3 as u32);
            }
            let now = monotonic_now();
            capture.frame.ready(
                (now.as_secs() >> 32) as u32,
                (now.as_secs() & 0xffff_ffff) as u32,
                now.subsec_nanos(),
            );
            self.picture_taken(capture.output);
        }
    }

    /// Draw one window alone, as it is, into the capture image and read
    /// it back: its size in the pixels of the display it is on and its
    /// pixels, four bytes each, red first.
    pub(crate) fn draw_window_for_capture<R, T>(
        &mut self,
        renderer: &mut R,
        target: &mut CaptureTarget<T>,
        format: Fourcc,
        window: gevrek_engine::WindowHandle,
    ) -> anyhow::Result<((i32, i32), Vec<u8>)>
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
    {
        use smithay::backend::renderer::element::surface::{
            render_elements_from_surface_tree, WaylandSurfaceRenderElement,
        };
        use smithay::backend::renderer::element::utils::RescaleRenderElement;
        use smithay::backend::renderer::element::Kind;
        let size = self
            .window_capture_size(window)
            .ok_or_else(|| anyhow::anyhow!("the window is gone"))?;
        let scale = self
            .surface_outputs
            .get(&window)
            .map_or(1.0, |(_, scale)| *scale);
        let record = &self.desktop.windows[&window];
        let surface = record
            .surface
            .wl_surface()
            .ok_or_else(|| anyhow::anyhow!("the window has no surface"))?;
        let origin: smithay::utils::Point<i32, smithay::utils::Physical> =
            (-record.geometry_offset.0, -record.geometry_offset.1).into();
        let surfaces: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
            renderer,
            &surface,
            origin,
            1.0,
            1.0,
            Kind::Unspecified,
        );
        let elements: Vec<RescaleRenderElement<WaylandSurfaceRenderElement<R>>> = surfaces
            .into_iter()
            .map(|element| RescaleRenderElement::from_element(element, (0, 0).into(), scale))
            .collect();
        if target.image.as_ref().map(|(known, _)| *known) != Some(size) {
            let image = renderer
                .create_buffer(format, size.into())
                .map_err(|error| anyhow::anyhow!("capture image: {error:?}"))?;
            target.image = Some((size, image));
        }
        let (_, image) = target.image.as_mut().expect("just made");
        let mut framebuffer = renderer
            .bind(image)
            .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
        OutputDamageTracker::new(size, 1.0, Transform::Normal)
            .render_output(
                renderer,
                &mut framebuffer,
                0,
                &elements,
                smithay::backend::renderer::Color32F::new(0.0, 0.0, 0.0, 1.0),
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
    }

    /// How large a picture of one window is: its frame in the pixels of
    /// the display it is on. `None` for a window that is gone or not
    /// drawn yet.
    pub(crate) fn window_capture_size(
        &self,
        window: gevrek_engine::WindowHandle,
    ) -> Option<(i32, i32)> {
        let record = self.desktop.windows.get(&window)?;
        let scale = self
            .surface_outputs
            .get(&window)
            .map_or(1.0, |(_, scale)| *scale);
        let size = (
            (f64::from(record.rect.width) * scale).round() as i32,
            (f64::from(record.rect.height) * scale).round() as i32,
        );
        (record.mapped && size.0 > 0 && size.1 > 0).then_some(size)
    }

    /// How large a picture of an output is, in its device pixels.
    pub(crate) fn output_capture_size(&self, output: MonitorHandle) -> Option<(i32, i32)> {
        self.device_size(output)
            .map(|(width, height, _)| (width, height))
            .filter(|(width, height)| *width > 0 && *height > 0)
    }

    /// Draw one output straight into a program's buffer of the graphics
    /// card: nothing is read back or copied. An error where the buffer
    /// is not of the output's size or this renderer cannot draw into it.
    pub(crate) fn draw_into_card<R>(
        &mut self,
        renderer: &mut R,
        dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
        output: MonitorHandle,
        with_pointer: bool,
    ) -> anyhow::Result<()>
    where
        R: Renderer + ImportAll + ImportMem + crate::clip::ClipRenderer,
        R::TextureId: Send + Clone + 'static,
        R::Error: std::fmt::Debug,
    {
        use smithay::backend::allocator::Buffer as _;
        let (width, height, _) = self
            .device_size(output)
            .ok_or_else(|| anyhow::anyhow!("the output is gone"))?;
        let size = (width, height);
        anyhow::ensure!(
            (dmabuf.width() as i32, dmabuf.height() as i32) == size,
            "the buffer is not of the output's size"
        );
        self.hide_pointer = !with_pointer;
        // (Another program's picture: without what is kept out of those.)
        self.capture_pass = true;
        let elements = self.scaled_elements(renderer, output);
        self.capture_pass = false;
        self.hide_pointer = false;
        let mut framebuffer = renderer
            .bind_card(dmabuf)
            .ok_or_else(|| anyhow::anyhow!("the buffer cannot be drawn into"))?;
        let mut tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
        let result = tracker
            .render_output(
                renderer,
                &mut framebuffer,
                0,
                &elements,
                crate::render::BACKGROUND,
            )
            .map_err(|error| anyhow::anyhow!("render: {error:?}"))?;
        // (The picture is the program's when it is told so: the card
        // is waited for.)
        let _ = result.sync.wait();
        Ok(())
    }

    /// Draw one output into the capture image and read it back: its size
    /// in device pixels and its pixels, four bytes each, red first.
    pub(crate) fn draw_for_capture<R, T>(
        &mut self,
        renderer: &mut R,
        target: &mut CaptureTarget<T>,
        format: Fourcc,
        output: MonitorHandle,
        with_pointer: bool,
    ) -> anyhow::Result<((i32, i32), Vec<u8>)>
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
    {
        let (width, height, _) = self
            .device_size(output)
            .ok_or_else(|| anyhow::anyhow!("the output is gone"))?;
        anyhow::ensure!(width > 0 && height > 0, "the output has no size");
        let size = (width, height);
        if target.image.as_ref().map(|(known, _)| *known) != Some(size) {
            let image = renderer
                .create_buffer(format, size.into())
                .map_err(|error| anyhow::anyhow!("capture image: {error:?}"))?;
            target.image = Some((size, image));
        }
        let (_, image) = target.image.as_mut().expect("just made");
        self.hide_pointer = !with_pointer;
        // (Another program's picture: without what is kept out of those.)
        self.capture_pass = true;
        let elements = self.scaled_elements(renderer, output);
        self.capture_pass = false;
        self.hide_pointer = false;
        let mut framebuffer = renderer
            .bind(image)
            .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
        // Every picture is drawn whole: the image holds whatever output
        // and kind was drawn into it last.
        OutputDamageTracker::new(size, 1.0, Transform::Normal)
            .render_output(
                renderer,
                &mut framebuffer,
                0,
                &elements,
                crate::render::BACKGROUND,
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
    }
}

/// Copy the part a frame is of into its buffer. The picture has red
/// first; the buffer (`xrgb8888`, little-endian) has blue first.
fn copy_into(capture: &PendingCapture, size: (i32, i32), pixels: &[u8]) -> bool {
    copy_pixels(&capture.buffer, capture.region, size, pixels)
}

/// Copy a part of a picture (`size`, red first) into a program's buffer
/// of shared memory, which must be exactly as large as the part.
pub(crate) fn copy_pixels(
    buffer: &WlBuffer,
    region: (i32, i32, i32, i32),
    size: (i32, i32),
    pixels: &[u8],
) -> bool {
    copy_pixels_as(buffer, region, size, pixels, false)
}

/// The same; `see_through`: the picture keeps how solid each pixel is
/// (the pointer's own picture), where a display's is solid throughout.
pub(crate) fn copy_pixels_as(
    buffer: &WlBuffer,
    region: (i32, i32, i32, i32),
    size: (i32, i32),
    pixels: &[u8],
    see_through: bool,
) -> bool {
    let (x, y, width, height) = region;
    // The output may have changed its size since the frame was described.
    if x + width > size.0
        || y + height > size.1
        || pixels.len() < size.0 as usize * size.1 as usize * 4
    {
        return false;
    }
    shm::with_buffer_contents_mut(buffer, |pointer, length, description| {
        let stride = description.stride as usize;
        let offset = description.offset as usize;
        let (width, height) = (width as usize, height as usize);
        if description.width as usize != width
            || description.height as usize != height
            || stride < width * 4
            || offset + stride * (height - 1) + width * 4 > length
        {
            return false;
        }
        // SAFETY: smithay hands out the mapped pool for the time of this
        // call; the bounds were checked against its length just above.
        let memory = unsafe { std::slice::from_raw_parts_mut(pointer, length) };
        for row in 0..height {
            let from = ((y as usize + row) * size.0 as usize + x as usize) * 4;
            let to = offset + row * stride;
            let source = &pixels[from..from + width * 4];
            let destination = &mut memory[to..to + width * 4];
            let (out, pixel) = (
                destination.as_chunks_mut::<4>().0,
                source.as_chunks::<4>().0,
            );
            for (out, pixel) in out.iter_mut().zip(pixel) {
                out[0] = pixel[2];
                out[1] = pixel[1];
                out[2] = pixel[0];
                out[3] = if see_through { pixel[3] } else { 0xff };
            }
        }
        true
    })
    .unwrap_or(false)
}

/// The time on the clock that never goes back, which frames are stamped
/// with.
pub(crate) fn monotonic_now() -> std::time::Duration {
    smithay::utils::Clock::<smithay::utils::Monotonic>::new()
        .now()
        .into()
}
