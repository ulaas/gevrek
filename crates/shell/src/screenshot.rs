//! Pictures of the screen: a whole display, or a part chosen by dragging.
//!
//! The picture is asked of the compositor the way any screenshot tool
//! asks (wlr-screencopy), on a connection of its own that lasts as long
//! as the one picture. It is written as a PNG file into the pictures
//! folder (`Screenshots` in it), named by the time.

use anyhow::Context as _;
use smithay_client_toolkit::shm::{raw::RawPool, Shm, ShmHandler};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// A picture: its size and its pixels, four bytes each, red first.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

struct Taking {
    shm: Shm,
    described: Option<(u32, u32, u32)>,
    pool: Option<RawPool>,
    buffer: Option<wl_buffer::WlBuffer>,
    names: Vec<(wl_output::WlOutput, Option<String>)>,
    done: Option<Result<(), String>>,
}

/// Take a picture of a display: the one named, else the first.
pub fn take(output: Option<&str>) -> anyhow::Result<Picture> {
    let connection = Connection::connect_to_env().context("no Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<Taking>(&connection).context("the compositor's globals")?;
    let handle = queue.handle();
    let manager: ZwlrScreencopyManagerV1 = globals
        .bind(&handle, 1..=3, ())
        .map_err(|_| anyhow::anyhow!("the compositor gives no pictures of the screen"))?;
    let mut taking = Taking {
        shm: Shm::bind(&globals, &handle).context("wl_shm")?,
        described: None,
        pool: None,
        buffer: None,
        names: Vec::new(),
        done: None,
    };
    let listed: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect()
    });
    for (name, version) in listed {
        let bound: wl_output::WlOutput = globals.registry().bind(name, version.min(4), &handle, ());
        taking.names.push((bound, None));
    }
    // The outputs say their names.
    queue
        .roundtrip(&mut taking)
        .context("the displays' names")?;
    let chosen = match output {
        Some(wanted) => taking
            .names
            .iter()
            .find(|(_, name)| name.as_deref() == Some(wanted)),
        None => taking.names.first(),
    }
    .map(|(output, _)| output.clone())
    .ok_or_else(|| anyhow::anyhow!("there is no such display"))?;
    // Without the pointer in it.
    manager.capture_output(0, &chosen, &handle, ());
    while taking.done.is_none() {
        queue
            .blocking_dispatch(&mut taking)
            .context("waiting for the picture")?;
    }
    taking
        .done
        .take()
        .unwrap_or(Ok(()))
        .map_err(|why| anyhow::anyhow!(why))?;
    let (width, height, stride) = taking.described.context("the picture was not described")?;
    let pool = taking.pool.as_mut().context("the picture has no memory")?;
    let raw = pool.mmap();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let line = &raw[row * stride as usize..row * stride as usize + width as usize * 4];
        for pixel in line.as_chunks::<4>().0 {
            // The buffer has blue first.
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    Ok(Picture {
        width,
        height,
        pixels,
    })
}

impl Taking {
    fn hand_over(&mut self, frame: &ZwlrScreencopyFrameV1, handle: &QueueHandle<Self>) {
        let Some((width, height, stride)) = self.described else {
            self.done = Some(Err(
                "the compositor offered no format this program reads".to_string()
            ));
            return;
        };
        if self.buffer.is_some() {
            return;
        }
        match RawPool::new((stride * height) as usize, &self.shm) {
            Ok(mut pool) => {
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
            Err(error) => self.done = Some(Err(format!("no memory for the picture: {error}"))),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Taking {
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
                if frame.version() < 3 {
                    state.hand_over(frame, handle);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.hand_over(frame, handle),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.done = Some(Ok(())),
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.done = Some(Err("the compositor could not take the picture".to_string()))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Taking {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(entry) = state.names.iter_mut().find(|(known, _)| known == output) {
                entry.1 = Some(name);
            }
        }
    }
}

impl ShmHandler for Taking {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
smithay_client_toolkit::delegate_shm!(Taking);

macro_rules! ignore {
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
ignore!(wl_registry::WlRegistry, GlobalListContents);
ignore!(wl_buffer::WlBuffer, ());
ignore!(ZwlrScreencopyManagerV1, ());

/// The part `(x, y, width, height)` of a picture, as far as it lies in it.
pub fn crop(picture: &Picture, part: (u32, u32, u32, u32)) -> Option<Picture> {
    let (x, y) = (part.0.min(picture.width), part.1.min(picture.height));
    let width = part.2.min(picture.width - x);
    let height = part.3.min(picture.height - y);
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in y..y + height {
        let from = (row as usize * picture.width as usize + x as usize) * 4;
        pixels.extend_from_slice(&picture.pixels[from..from + width as usize * 4]);
    }
    Some(Picture {
        width,
        height,
        pixels,
    })
}

/// Where pictures of the screen are kept: `Screenshots` in the pictures
/// folder (the home folder when there is none).
/// The folder pictures go into: the one the configuration names (`~` is
/// the home folder), else `Screenshots` in the pictures folder.
pub fn folder_of(configured: &str) -> std::path::PathBuf {
    let configured = configured.trim();
    if configured.is_empty() {
        return folder();
    }
    match (configured.strip_prefix("~/"), dirs_home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if configured == "~" => dirs_home().unwrap_or_else(folder),
        _ => std::path::PathBuf::from(configured),
    }
}

pub fn folder() -> std::path::PathBuf {
    std::env::var_os("XDG_PICTURES_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs_home().map(|home| home.join("Pictures")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Screenshots")
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Write a picture as a PNG file named by the time, and say where it is.
/// The picture as the bytes of a PNG file (what goes on the clipboard).
pub fn png(picture: &Picture) -> anyhow::Result<Vec<u8>> {
    use image::ImageEncoder as _;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &picture.pixels,
            picture.width,
            picture.height,
            image::ExtendedColorType::Rgba8,
        )
        .context("the picture as PNG")?;
    Ok(bytes)
}

pub fn save(picture: &Picture, folder: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let folder = folder.to_path_buf();
    std::fs::create_dir_all(&folder).with_context(|| format!("{}", folder.display()))?;
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d %H-%M-%S");
    let mut path = folder.join(format!("Screenshot {stamp}.png"));
    let mut number = 2;
    while path.exists() {
        path = folder.join(format!("Screenshot {stamp} ({number}).png"));
        number += 1;
    }
    image::save_buffer(
        &path,
        &picture.pixels,
        picture.width,
        picture.height,
        image::ExtendedColorType::Rgba8,
    )
    .with_context(|| format!("{}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> Picture {
        // 4 by 3; each pixel's red is its place in the row, green its row.
        let mut pixels = Vec::new();
        for y in 0..3u8 {
            for x in 0..4u8 {
                pixels.extend_from_slice(&[x, y, 0, 255]);
            }
        }
        Picture {
            width: 4,
            height: 3,
            pixels,
        }
    }

    #[test]
    fn a_part_of_a_picture_is_cut_out_and_stays_inside_it() {
        let part = crop(&picture(), (1, 1, 2, 2)).expect("a part");
        assert_eq!((part.width, part.height), (2, 2));
        assert_eq!(&part.pixels[..4], &[1, 1, 0, 255]);
        assert_eq!(&part.pixels[12..16], &[2, 2, 0, 255]);
        // Reaching past the edge: cut at the edge.
        let edge = crop(&picture(), (3, 2, 10, 10)).expect("a part");
        assert_eq!((edge.width, edge.height), (1, 1));
        assert_eq!(&edge.pixels[..], &[3, 2, 0, 255]);
        assert!(crop(&picture(), (4, 0, 2, 2)).is_none());
        assert!(crop(&picture(), (0, 0, 0, 5)).is_none());
    }

    #[test]
    fn a_picture_is_written_as_a_file_that_reads_back_the_same() {
        let folder = std::env::temp_dir().join(format!("gevrek-shots-{}", std::process::id()));
        std::env::set_var("XDG_PICTURES_DIR", &folder);
        // Where the configuration names no folder: Screenshots in the
        // pictures folder. Where it names one, that one.
        assert_eq!(folder_of(""), folder.join("Screenshots"));
        assert_eq!(
            folder_of("  /srv/shots "),
            std::path::PathBuf::from("/srv/shots")
        );
        if let Some(home) = dirs_home() {
            assert_eq!(folder_of("~/Desktop/shots"), home.join("Desktop/shots"));
        }
        let first = save(&picture(), &folder_of("")).expect("saved");
        let second = save(&picture(), &folder_of("")).expect("saved again");
        assert_ne!(first, second, "two in one second have two names");
        assert!(first.starts_with(folder.join("Screenshots")));
        let read = image::open(&first).expect("a picture").to_rgba8();
        assert_eq!(read.dimensions(), (4, 3));
        assert_eq!(read.into_raw(), picture().pixels);
        std::env::remove_var("XDG_PICTURES_DIR");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
