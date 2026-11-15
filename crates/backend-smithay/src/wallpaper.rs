//! The wallpaper: a picture behind the windows, drawn by gevrek itself.
//!
//! The file is read and put on a picture as large as the display, once,
//! and that picture is kept until the display's size or the setting
//! changes; a frame then costs one more picture to put down. A file that
//! is not there or is not a picture leaves the colour, and says so once.
//!
//! Reading a large file takes longer than a frame may, so it is done on
//! a thread of its own: what was shown stays until the new picture is
//! ready. With a slideshow the picture is one of a folder's, the next
//! when its time is over.

use gevrek_engine::outputs::MonitorHandle;
use gevrek_model::wallpaper::WallpaperFit;
use image::{imageops, Rgba, RgbaImage};
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use std::collections::HashMap;
use std::path::PathBuf;

/// What a display's picture was made from.
#[derive(Debug, Clone, PartialEq)]
struct Made {
    path: PathBuf,
    fit: WallpaperFit,
    size: (i32, i32),
    /// How many of the display's own pixels one of its points is: a
    /// picture shown at its own size is as large in points as it is in
    /// pixels.
    scale: f64,
    color: (u8, u8, u8),
}

/// How long one wallpaper takes to give way to the next
/// (`GEVREK_WALLPAPER_FADE_MS` sets another time: the checks look at it
/// more slowly than it happens).
fn fade() -> std::time::Duration {
    static FADE: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
    *FADE.get_or_init(|| {
        std::env::var("GEVREK_WALLPAPER_FADE_MS")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .filter(|milliseconds| *milliseconds > 0)
            .map_or(
                std::time::Duration::from_millis(600),
                std::time::Duration::from_millis,
            )
    })
}

/// How long a picture of a slideshow stays, where the checks set another
/// time than the configuration can say (`GEVREK_WALLPAPER_SLIDE_MS`).
fn slide_override() -> Option<std::time::Duration> {
    static SLIDE: std::sync::OnceLock<Option<std::time::Duration>> = std::sync::OnceLock::new();
    *SLIDE.get_or_init(|| {
        std::env::var("GEVREK_WALLPAPER_SLIDE_MS")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .filter(|milliseconds| *milliseconds > 0)
            .map(std::time::Duration::from_millis)
    })
}

/// The pictures of a folder, by name.
pub fn pictures_in(folder: &std::path::Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["png", "jpg", "jpeg", "webp"]
                        .iter()
                        .any(|known| extension.eq_ignore_ascii_case(known))
                })
                && path.is_file()
        })
        .collect();
    found.sort();
    found
}

/// A colour: red, green, blue.
type Colour = (u8, u8, u8);

/// A picture as it was read: its bytes as the buffer wants them, the
/// colour that stands out in it, and a small blurred copy of it (its
/// bytes and its size).
struct Read {
    bytes: Vec<u8>,
    accent: Option<Colour>,
    blurred: (Vec<u8>, (i32, i32)),
}

/// A picture being read: what came of it, or why nothing did.
type Reading = std::sync::mpsc::Receiver<Result<Read, String>>;

/// A quarter of the picture's sides, blurred: shown large again it is
/// soft enough to stand behind something that is to be read, and costs
/// little to make and to keep.
pub fn blurred(canvas: &RgbaImage) -> RgbaImage {
    let small = imageops::resize(
        canvas,
        (canvas.width() / 4).max(1),
        (canvas.height() / 4).max(1),
        imageops::FilterType::Triangle,
    );
    imageops::fast_blur(&small, 6.0)
}

/// The bytes of a picture as the buffer wants them: blue first.
fn buffer_bytes(picture: RgbaImage) -> Vec<u8> {
    let mut bytes = picture.into_raw();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    bytes
}

/// Read a file and put it on a picture for a display, on a thread of its
/// own.
fn read(wanted: &Made) -> Reading {
    let (tell, reading) = std::sync::mpsc::channel();
    let made = wanted.clone();
    let work = move || {
        let result = image::open(&made.path)
            .map(|image| {
                // (A small copy says as much of its colours as all of it.)
                let accent = gevrek_model::theme::accent_of_pixels(
                    image
                        .thumbnail(64, 64)
                        .to_rgb8()
                        .pixels()
                        .map(|pixel| (pixel[0], pixel[1], pixel[2])),
                );
                let canvas = compose(
                    &image.to_rgba8(),
                    made.fit,
                    (made.size.0.max(1) as u32, made.size.1.max(1) as u32),
                    made.scale,
                    made.color,
                );
                let soft = blurred(&canvas);
                let soft_size = (soft.width() as i32, soft.height() as i32);
                Read {
                    blurred: (buffer_bytes(soft), soft_size),
                    bytes: buffer_bytes(canvas),
                    accent,
                }
            })
            .map_err(|error| error.to_string());
        let _ = tell.send(result);
    };
    if let Err(error) = std::thread::Builder::new()
        .name("gevrek-wallpaper".to_string())
        .spawn(work)
    {
        log::warn!("no thread to read the wallpaper on: {error}");
    }
    reading
}

#[derive(Default)]
pub struct Wallpapers {
    /// Per display: the colour that stands out in its picture.
    accents: HashMap<MonitorHandle, Colour>,
    /// Per display: its picture blurred, small (drawn large again), and
    /// the small picture's size.
    blurred: HashMap<MonitorHandle, (MemoryRenderBuffer, (i32, i32))>,
    /// What the veil over the blurred wallpaper of an overview is known
    /// by to the drawing.
    veil: Option<smithay::backend::renderer::element::Id>,
    /// Per display: the picture that is being read for it.
    reading: HashMap<MonitorHandle, (Made, Reading)>,
    /// The pictures of the slideshow's folder, and when they were listed
    /// (the folder is looked into again now and then).
    slides: Option<(PathBuf, Vec<PathBuf>, std::time::Instant)>,
    /// The step of the slideshow that was last asked for: a frame is due
    /// when the clock has gone on to the next.
    slide_step: Option<u64>,
    /// Per display: what its picture is of, and the picture (`None`: the
    /// file could not be shown, which was said when it was tried).
    pictures: HashMap<MonitorHandle, (Made, Option<MemoryRenderBuffer>)>,
    /// Per display: the picture that was there before, and since when the
    /// new one is coming in over it.
    leaving: HashMap<MonitorHandle, (Option<MemoryRenderBuffer>, std::time::Instant)>,
}

/// Put `image` on a picture of `size` in the way `fit` says, on `color`.
pub fn compose(
    image: &RgbaImage,
    fit: WallpaperFit,
    size: (u32, u32),
    scale: f64,
    color: (u8, u8, u8),
) -> RgbaImage {
    let (width, height) = (size.0.max(1), size.1.max(1));
    // "At its own size" is in the display's points: on a display of
    // twice the scale the picture takes twice its pixels.
    let own_size;
    let image = if matches!(fit, WallpaperFit::Centre | WallpaperFit::Tile)
        && (scale - 1.0).abs() > 0.01
        && image.width() > 0
        && image.height() > 0
    {
        own_size = imageops::resize(
            image,
            ((f64::from(image.width()) * scale).round() as u32).max(1),
            ((f64::from(image.height()) * scale).round() as u32).max(1),
            imageops::FilterType::Triangle,
        );
        &own_size
    } else {
        image
    };
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([color.0, color.1, color.2, 255]));
    let centred = |canvas: &mut RgbaImage, picture: &RgbaImage| {
        let x = (i64::from(width) - i64::from(picture.width())) / 2;
        let y = (i64::from(height) - i64::from(picture.height())) / 2;
        imageops::overlay(canvas, picture, x, y);
    };
    if image.width() == 0 || image.height() == 0 {
        return canvas;
    }
    match fit {
        WallpaperFit::Stretch => {
            let scaled = imageops::resize(image, width, height, imageops::FilterType::Triangle);
            imageops::overlay(&mut canvas, &scaled, 0, 0);
        }
        WallpaperFit::Fill | WallpaperFit::Fit => {
            let across = f64::from(width) / f64::from(image.width());
            let down = f64::from(height) / f64::from(image.height());
            // Fill: large enough to cover; Fit: small enough to be whole.
            let scale = if fit == WallpaperFit::Fill {
                across.max(down)
            } else {
                across.min(down)
            };
            let to = (
                (f64::from(image.width()) * scale).round().max(1.0) as u32,
                (f64::from(image.height()) * scale).round().max(1.0) as u32,
            );
            let scaled = imageops::resize(image, to.0, to.1, imageops::FilterType::Triangle);
            centred(&mut canvas, &scaled);
        }
        WallpaperFit::Centre => centred(&mut canvas, image),
        WallpaperFit::Tile => {
            let mut y = 0;
            while y < height {
                let mut x = 0;
                while x < width {
                    imageops::overlay(&mut canvas, image, i64::from(x), i64::from(y));
                    x += image.width();
                }
                y += image.height();
            }
        }
    }
    canvas
}

impl Wallpapers {
    /// The picture for a display of `size` (its own pixels, `scale` of
    /// them to a point), made if it is not there or was made from other
    /// settings. `None`: no picture is set, or the file cannot be shown.
    pub fn picture(
        &mut self,
        output: MonitorHandle,
        path: Option<PathBuf>,
        fit: WallpaperFit,
        size: (i32, i32),
        scale: f64,
        color: (u8, u8, u8),
    ) -> Option<&MemoryRenderBuffer> {
        let Some(path) = path else {
            // The picture gives way to the colour.
            self.accents.remove(&output);
            self.blurred.remove(&output);
            self.reading.remove(&output);
            if let Some((_, old)) = self.pictures.remove(&output) {
                self.leaving
                    .insert(output, (old, std::time::Instant::now()));
            }
            return None;
        };
        let wanted = Made {
            path,
            fit,
            size,
            scale,
            color,
        };
        if self.pictures.get(&output).map(|(made, _)| made) == Some(&wanted) {
            // (Something else was being read meanwhile: not wanted now.)
            self.reading.remove(&output);
        } else {
            if self.reading.get(&output).map(|(made, _)| made) != Some(&wanted) {
                self.reading.insert(output, (wanted.clone(), read(&wanted)));
            }
            let read = self
                .reading
                .get(&output)
                .map(|(_, reading)| reading.try_recv());
            let arrived = match read {
                Some(Ok(result)) => Some(result),
                // The thread ended without a word.
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    Some(Err("it could not be read".to_string()))
                }
                _ => None,
            };
            if let Some(result) = arrived {
                self.reading.remove(&output);
                let buffer = match result {
                    Ok(Read {
                        bytes,
                        accent,
                        blurred: (soft, soft_size),
                    }) => {
                        match accent {
                            Some(accent) => self.accents.insert(output, accent),
                            None => self.accents.remove(&output),
                        };
                        self.blurred.insert(
                            output,
                            (
                                MemoryRenderBuffer::from_slice(
                                    &soft,
                                    smithay::backend::allocator::Fourcc::Argb8888,
                                    soft_size,
                                    1,
                                    smithay::utils::Transform::Normal,
                                    None,
                                ),
                                soft_size,
                            ),
                        );
                        log::info!(
                            "Wallpaper: {} ({}x{})",
                            wanted.path.display(),
                            size.0,
                            size.1
                        );
                        Some(MemoryRenderBuffer::from_slice(
                            &bytes,
                            smithay::backend::allocator::Fourcc::Argb8888,
                            (size.0.max(1), size.1.max(1)),
                            1,
                            smithay::utils::Transform::Normal,
                            None,
                        ))
                    }
                    Err(error) => {
                        log::warn!(
                            "The wallpaper {} cannot be shown: {error}",
                            wanted.path.display()
                        );
                        self.accents.remove(&output);
                        self.blurred.remove(&output);
                        None
                    }
                };
                // What was there stays under the new one while that comes
                // in (not when only the display's size changed).
                if let Some((made, old)) = self.pictures.remove(&output) {
                    if made.size == wanted.size {
                        self.leaving
                            .insert(output, (old, std::time::Instant::now()));
                    }
                }
                self.pictures.insert(output, (wanted, buffer));
            }
        }
        self.pictures
            .get(&output)
            .and_then(|(_, buffer)| buffer.as_ref())
    }

    /// How far the new wallpaper of a display has come in (0 to 1), and
    /// the one it comes in over, while that is still going on.
    pub fn fading(&mut self, output: MonitorHandle) -> Option<(f32, Option<&MemoryRenderBuffer>)> {
        let since = self.leaving.get(&output)?.1;
        let part = since.elapsed().as_secs_f32() / fade().as_secs_f32();
        if part >= 1.0 {
            self.leaving.remove(&output);
            return None;
        }
        // Slow at both ends.
        let eased = part * part * (3.0 - 2.0 * part);
        self.leaving
            .get(&output)
            .map(|(old, _)| (eased, old.as_ref()))
    }

    /// The colour that stands out in a display's wallpaper; `None`: it
    /// has no picture, the picture has no colour to speak of, or it has
    /// not been read yet.
    pub fn accent(&self, output: MonitorHandle) -> Option<Colour> {
        self.accents.get(&output).copied()
    }

    /// A display's wallpaper blurred, and that small picture's size;
    /// `None`: the display has no picture (its colour needs no blur).
    pub fn blurred(&self, output: MonitorHandle) -> Option<(MemoryRenderBuffer, (i32, i32))> {
        self.blurred.get(&output).cloned()
    }

    /// What the veil over an overview's wallpaper is known by.
    pub fn veil_id(&mut self) -> smithay::backend::renderer::element::Id {
        self.veil
            .get_or_insert_with(smithay::backend::renderer::element::Id::new)
            .clone()
    }

    /// Whether any display's wallpaper is still coming in, or being read.
    pub fn any_fading(&self) -> bool {
        !self.reading.is_empty()
            || self
                .leaving
                .values()
                .any(|(_, since)| since.elapsed() < fade())
    }

    /// The step of the slideshow the clock is in.
    fn step_now(show: &gevrek_model::wallpaper::Slideshow) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        match slide_override() {
            Some(period) => (now.as_millis() / period.as_millis().max(1)) as u64,
            None => now.as_secs() / show.period_seconds(),
        }
    }

    /// The picture of the slideshow that is to be shown now; `None`:
    /// there is no slideshow, or its folder has no picture.
    pub fn slide(&mut self, show: &gevrek_model::wallpaper::Slideshow) -> Option<PathBuf> {
        if !show.is_on() {
            self.slides = None;
            self.slide_step = None;
            return None;
        }
        let folder = show.folder_path();
        let stale = self.slides.as_ref().is_none_or(|(known, _, listed)| {
            *known != folder || listed.elapsed() > std::time::Duration::from_secs(60)
        });
        if stale {
            let pictures = pictures_in(&folder);
            if pictures.is_empty()
                && self
                    .slides
                    .as_ref()
                    .is_none_or(|(known, _, _)| *known != folder)
            {
                log::warn!(
                    "The slideshow's folder {} has no pictures",
                    folder.display()
                );
            }
            self.slides = Some((folder, pictures, std::time::Instant::now()));
        }
        let step = Self::step_now(show);
        self.slide_step = Some(step);
        let pictures = &self.slides.as_ref()?.1;
        show.slide(pictures.len(), step)
            .and_then(|at| pictures.get(at).cloned())
    }

    /// Whether the slideshow's next picture is due: a frame is to be
    /// drawn for it.
    pub fn slide_due(&self, show: &gevrek_model::wallpaper::Slideshow) -> bool {
        show.is_on() && self.slide_step != Some(Self::step_now(show))
    }

    /// Without the fade (things do not move by themselves): what was
    /// there is gone at once.
    pub fn settle(&mut self) {
        self.leaving.clear();
    }

    /// A display is gone: its picture is let go.
    pub fn forget(&mut self, output: MonitorHandle) {
        self.pictures.remove(&output);
        self.leaving.remove(&output);
        self.reading.remove(&output);
        self.accents.remove(&output);
        self.blurred.remove(&output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba<u8> = Rgba([200, 0, 0, 255]);
    const DESK: (u8, u8, u8) = (10, 20, 30);
    const DESK_PIXEL: Rgba<u8> = Rgba([10, 20, 30, 255]);

    #[test]
    fn the_blurred_copy_is_small_and_soft() {
        // Lines one pixel wide, black and white.
        let lines = RgbaImage::from_fn(400, 200, |x, _| {
            if x % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let soft = blurred(&lines);
        assert_eq!((soft.width(), soft.height()), (100, 50));
        // No line is left: every pixel is a grey near the middle.
        for x in 10..90 {
            let grey = soft.get_pixel(x, 25).0[0];
            assert!((110..=145).contains(&grey), "at {x}: {grey}");
        }
        // A picture of one colour stays that colour.
        let plain = blurred(&RgbaImage::from_pixel(40, 40, Rgba([10, 200, 30, 255])));
        assert_eq!(plain.get_pixel(5, 5).0, [10, 200, 30, 255]);
    }

    #[test]
    fn the_pictures_of_a_folder_are_listed_by_name() {
        let folder = std::env::temp_dir().join(format!("gevrek-slides-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("more.png")).expect("a folder");
        for name in ["b.PNG", "a.jpg", "notes.txt", "c.webp"] {
            std::fs::write(folder.join(name), b"x").expect("a file");
        }
        let names: Vec<String> = pictures_in(&folder)
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
            .collect();
        // Pictures only, no folder that is named like one.
        assert_eq!(names, ["a.jpg", "b.PNG", "c.webp"]);
        assert!(pictures_in(&folder.join("none")).is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn red(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, RED)
    }

    #[test]
    fn fill_covers_the_display_and_fit_shows_the_whole_picture() {
        // A wide picture on a square display.
        let filled = compose(&red(200, 100), WallpaperFit::Fill, (100, 100), 1.0, DESK);
        assert_eq!(filled.dimensions(), (100, 100));
        assert!(filled.pixels().all(|pixel| *pixel == RED));
        let fitted = compose(&red(200, 100), WallpaperFit::Fit, (100, 100), 1.0, DESK);
        // The picture is a band across the middle, the colour above and below.
        assert_eq!(*fitted.get_pixel(50, 50), RED);
        assert_eq!(*fitted.get_pixel(50, 5), DESK_PIXEL);
        assert_eq!(*fitted.get_pixel(50, 95), DESK_PIXEL);
        assert_eq!(*fitted.get_pixel(2, 50), RED);
    }

    #[test]
    fn centre_keeps_the_size_and_tile_repeats_it() {
        let centred = compose(&red(20, 20), WallpaperFit::Centre, (100, 60), 1.0, DESK);
        assert_eq!(*centred.get_pixel(50, 30), RED);
        assert_eq!(*centred.get_pixel(5, 5), DESK_PIXEL);
        assert_eq!(*centred.get_pixel(39, 30), DESK_PIXEL);
        assert_eq!(*centred.get_pixel(41, 30), RED);
        // Larger than the display: the middle of it is what shows.
        // On a display of twice the scale it takes twice its pixels.
        let doubled = compose(&red(20, 20), WallpaperFit::Centre, (200, 120), 2.0, DESK);
        assert_eq!(doubled.get_pixel(100, 60).0, [200, 0, 0, 255]);
        assert_eq!(doubled.get_pixel(82, 42).0, [200, 0, 0, 255]);
        assert_eq!(doubled.get_pixel(78, 38).0[..3], [DESK.0, DESK.1, DESK.2]);
        let large = compose(&red(300, 300), WallpaperFit::Centre, (100, 60), 1.0, DESK);
        assert!(large.pixels().all(|pixel| *pixel == RED));
        let mut tile = red(10, 10);
        tile.put_pixel(0, 0, Rgba([0, 0, 255, 255]));
        let tiled = compose(&tile, WallpaperFit::Tile, (35, 25), 1.0, DESK);
        for (x, y) in [(0, 0), (10, 0), (30, 20), (20, 10)] {
            assert_eq!(
                *tiled.get_pixel(x, y),
                Rgba([0, 0, 255, 255]),
                "a tile starts at {x}, {y}"
            );
        }
        assert_eq!(*tiled.get_pixel(34, 24), RED);
    }

    #[test]
    fn stretch_takes_the_displays_shape() {
        let stretched = compose(&red(10, 80), WallpaperFit::Stretch, (120, 40), 1.0, DESK);
        assert_eq!(stretched.dimensions(), (120, 40));
        assert!(stretched.pixels().all(|pixel| *pixel == RED));
    }
}
