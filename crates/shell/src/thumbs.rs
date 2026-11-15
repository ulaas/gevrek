//! Pictures of the windows that are open, small: what the window
//! switcher shows of the window the choice is on.
//!
//! Taken through the protocols a screenshot tool uses for one window
//! (ext-image-copy-capture with the window list as the source), on a
//! connection and a thread of their own, each window once. A window the
//! compositor gives no picture of in a moment is left without one.

use smithay_client_toolkit::shm::{raw::RawPool, Shm, ShmHandler};
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

pub use crate::switcher::Thumb;

/// How long one window's picture is waited for.
const PATIENCE: Duration = Duration::from_millis(400);
/// And all of them together.
const ALL: Duration = Duration::from_secs(3);

/// A picture as the compositor gives it (blue, green, red and a byte to
/// spare, rows of `stride` bytes) made no larger than `side` on its
/// longer side, by taking every so-manieth pixel. `None` when the bytes
/// are fewer than the size says.
pub fn shrink(width: u32, height: u32, stride: u32, pixels: &[u8], side: u32) -> Option<Thumb> {
    let (width, height, stride) = (width as usize, height as usize, stride as usize);
    if width == 0
        || height == 0
        || stride < width * 4
        || pixels.len() < stride * (height - 1) + width * 4
    {
        return None;
    }
    let by = (width.max(height) as f32 / side.max(1) as f32).max(1.0);
    let (out_width, out_height) = (
        ((width as f32 / by) as usize).max(1),
        ((height as f32 / by) as usize).max(1),
    );
    let mut rgba = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        let from_y = ((y as f32 * by) as usize).min(height - 1);
        for x in 0..out_width {
            let at = from_y * stride + ((x as f32 * by) as usize).min(width - 1) * 4;
            rgba.extend_from_slice(&[pixels[at + 2], pixels[at + 1], pixels[at], 255]);
        }
    }
    Some(Thumb {
        title: String::new(),
        app_id: String::new(),
        width: out_width as u32,
        height: out_height as u32,
        rgba,
    })
}

struct Window {
    handle: ExtForeignToplevelHandleV1,
    title: String,
    app_id: String,
}

struct Taking {
    shm: Shm,
    windows: Vec<Window>,
    size: Option<(u32, u32)>,
    described: bool,
    stopped: bool,
    /// `Some(true)`: the frame that was asked for is in the buffer.
    frame: Option<bool>,
}

/// Go on taking in what the compositor says until `done`, or until
/// `until` has come. Whether it was done.
fn wait(
    queue: &mut EventQueue<Taking>,
    taking: &mut Taking,
    until: Instant,
    done: impl Fn(&Taking) -> bool,
) -> bool {
    loop {
        if queue.roundtrip(taking).is_err() {
            return false;
        }
        if done(taking) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// A picture of every window that is open, none larger than `side`.
/// Empty where the compositor does not give pictures of windows.
pub fn take_all(side: u32) -> Vec<Thumb> {
    let Ok(connection) = Connection::connect_to_env() else {
        return Vec::new();
    };
    let Ok((globals, mut queue)) = registry_queue_init::<Taking>(&connection) else {
        return Vec::new();
    };
    let handle = queue.handle();
    let (Ok(manager), Ok(_list), Ok(sources), Ok(shm)) = (
        globals.bind::<ExtImageCopyCaptureManagerV1, _, _>(&handle, 1..=1, ()),
        globals.bind::<ExtForeignToplevelListV1, _, _>(&handle, 1..=1, ()),
        globals.bind::<ExtForeignToplevelImageCaptureSourceManagerV1, _, _>(&handle, 1..=1, ()),
        Shm::bind(&globals, &handle),
    ) else {
        return Vec::new();
    };
    let mut taking = Taking {
        shm,
        windows: Vec::new(),
        size: None,
        described: false,
        stopped: false,
        frame: None,
    };
    // The windows that are there, with their titles and programs.
    for _ in 0..2 {
        if queue.roundtrip(&mut taking).is_err() {
            return Vec::new();
        }
    }
    let started = Instant::now();
    let mut thumbs = Vec::new();
    for index in 0..taking.windows.len() {
        if started.elapsed() > ALL {
            break;
        }
        let source = sources.create_source(&taking.windows[index].handle, &handle, ());
        let session = manager.create_session(&source, Options::empty(), &handle, ());
        taking.size = None;
        taking.described = false;
        taking.stopped = false;
        taking.frame = None;
        let until = Instant::now() + PATIENCE;
        let described = wait(&mut queue, &mut taking, until, |taking| {
            taking.described || taking.stopped
        });
        let picture = (|| {
            let (width, height) = taking.size.filter(|(width, height)| {
                described && !taking.stopped && *width > 0 && *height > 0
            })?;
            let stride = width * 4;
            let mut pool = RawPool::new((stride * height) as usize, &taking.shm).ok()?;
            let buffer: wl_buffer::WlBuffer = pool.create_buffer(
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
            let came = wait(&mut queue, &mut taking, until, |taking| {
                taking.frame.is_some()
            });
            frame.destroy();
            let thumb = (came && taking.frame == Some(true))
                .then(|| shrink(width, height, stride, pool.mmap(), side))
                .flatten();
            buffer.destroy();
            thumb
        })();
        session.destroy();
        source.destroy();
        if let Some(mut thumb) = picture {
            thumb.title = taking.windows[index].title.clone();
            thumb.app_id = taking.windows[index].app_id.clone();
            thumbs.push(thumb);
        }
    }
    let _ = queue.roundtrip(&mut taking);
    thumbs
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
        let Some(window) = taking
            .windows
            .iter_mut()
            .find(|window| window.handle == *proxy)
        else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => window.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => window.app_id = app_id,
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
quiet!(ExtForeignToplevelImageCaptureSourceManagerV1, ());
quiet!(wl_buffer::WlBuffer, ());
quiet!(wl_registry::WlRegistry, GlobalListContents);

impl ShmHandler for Taking {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

smithay_client_toolkit::delegate_shm!(Taking);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_made_small_and_its_colours_put_in_order() {
        // Two by one, rows of twelve bytes: blue-first, with a byte to spare.
        let pixels = [30, 20, 10, 0, 60, 50, 40, 0, 9, 9, 9, 9];
        let thumb = shrink(2, 1, 12, &pixels, 480).expect("a picture");
        assert_eq!((thumb.width, thumb.height), (2, 1));
        assert_eq!(thumb.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
        // Larger than wanted: smaller, its shape kept.
        let large = vec![7u8; 1920 * 1080 * 4];
        let thumb = shrink(1920, 1080, 1920 * 4, &large, 480).expect("a picture");
        assert_eq!((thumb.width, thumb.height), (480, 270));
        assert_eq!(thumb.rgba.len(), 480 * 270 * 4);
        // Fewer bytes than the size says.
        assert_eq!(shrink(2, 2, 8, &pixels, 480), None);
        assert_eq!(shrink(0, 1, 8, &pixels, 480), None);
    }
}
