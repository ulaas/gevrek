//! `gevrek-testclient --capture FILE`: ask the compositor for a picture of
//! an output the way a screenshot tool does (wlr-screencopy), write it as
//! a PPM file and say how large it was.

use super::{say, word};
use smithay_client_toolkit::shm::{raw::RawPool, Shm, ShmHandler};
use std::io::Write;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

pub struct Request {
    pub path: String,
    /// Which output, counted from 0 in the order the compositor lists them.
    pub output: usize,
    /// A part of the output, in its logical pixels.
    pub region: Option<(i32, i32, i32, i32)>,
    pub pointer: bool,
}

struct Capture {
    shm: Shm,
    path: String,
    described: Option<(u32, u32, u32)>,
    pool: Option<RawPool>,
    buffer: Option<wl_buffer::WlBuffer>,
    /// `Some(true)` when the picture was written, `Some(false)` when the
    /// compositor refused.
    done: Option<bool>,
}

pub fn run(connection: &Connection, request: Request) -> bool {
    let (globals, mut queue) = registry_queue_init::<Capture>(connection).expect("registry");
    let handle = queue.handle();
    let Ok(manager) = globals.bind::<ZwlrScreencopyManagerV1, _, _>(&handle, 1..=3, ()) else {
        say(
            "error",
            &[("message", word("the compositor has no screencopy"))],
        );
        return false;
    };
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
    let output: wl_output::WlOutput = globals.registry().bind(name, version.min(4), &handle, ());
    let mut capture = Capture {
        shm: Shm::bind(&globals, &handle).expect("wl_shm"),
        path: request.path,
        described: None,
        pool: None,
        buffer: None,
        done: None,
    };
    let pointer = i32::from(request.pointer);
    match request.region {
        Some((x, y, width, height)) => {
            manager.capture_output_region(pointer, &output, x, y, width, height, &handle, ())
        }
        None => manager.capture_output(pointer, &output, &handle, ()),
    };
    while capture.done.is_none() {
        queue.blocking_dispatch(&mut capture).expect("dispatch");
    }
    capture.done == Some(true)
}

impl Capture {
    /// The compositor said what buffer it wants: make it and hand it over.
    fn hand_over(&mut self, frame: &ZwlrScreencopyFrameV1, handle: &QueueHandle<Self>) {
        let Some((width, height, stride)) = self.described else {
            return;
        };
        if self.buffer.is_some() {
            return;
        }
        let mut pool = RawPool::new((stride * height) as usize, &self.shm).expect("pool");
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Xrgb8888,
            (),
            handle,
        );
        frame.copy(&buffer);
        self.pool = Some(pool);
        self.buffer = Some(buffer);
    }

    fn write(&mut self) -> std::io::Result<(u32, u32)> {
        let (width, height, stride) = self.described.expect("described");
        let pool = self.pool.as_mut().expect("pool");
        let pixels = pool.mmap();
        let mut file = std::io::BufWriter::new(std::fs::File::create(&self.path)?);
        write!(file, "P6\n{width} {height}\n255\n")?;
        for row in 0..height as usize {
            let line = &pixels[row * stride as usize..row * stride as usize + width as usize * 4];
            for pixel in line.as_chunks::<4>().0 {
                file.write_all(&[pixel[2], pixel[1], pixel[0]])?;
            }
        }
        file.flush()?;
        Ok((width, height))
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Capture {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if format == WEnum::Value(wl_shm::Format::Xrgb8888) {
                    state.described = Some((width, height, stride));
                }
                // Before version 3 nothing says the description is over.
                if frame.version() < 3 {
                    state.hand_over(frame, handle);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.hand_over(frame, handle),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => match state.write() {
                Ok((width, height)) => {
                    say(
                        "captured",
                        &[("width", width.to_string()), ("height", height.to_string())],
                    );
                    state.done = Some(true);
                }
                Err(error) => {
                    say("error", &[("message", word(&error.to_string()))]);
                    state.done = Some(false);
                }
            },
            zwlr_screencopy_frame_v1::Event::Failed => {
                say("capture_failed", &[]);
                state.done = Some(false);
            }
            _ => {}
        }
    }
}

impl ShmHandler for Capture {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
smithay_client_toolkit::delegate_shm!(Capture);

macro_rules! ignore {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Capture {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
ignore!(wl_registry::WlRegistry, GlobalListContents);
ignore!(wl_output::WlOutput, ());
ignore!(wl_buffer::WlBuffer, ());
ignore!(ZwlrScreencopyManagerV1, ());
