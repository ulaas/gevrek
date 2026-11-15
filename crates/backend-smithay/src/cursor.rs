//! The pointer's picture. A client may give its own (a surface), or ask
//! for a named one (an arrow, a hand, a text cursor, a resize arrow).
//! Where no host session draws the pointer (on real hardware) gevrek draws
//! the named one from the cursor theme of the system, at the size the
//! output's scale needs; a plain arrow of its own is what is left when the
//! theme has no such picture.

/// The built-in arrow: `#` is the black outline, `.` the white inside.
const ARROW: [&str; 19] = [
    "#           ",
    "##          ",
    "#.#         ",
    "#..#        ",
    "#...#       ",
    "#....#      ",
    "#.....#     ",
    "#......#    ",
    "#.......#   ",
    "#........#  ",
    "#.........# ",
    "#......#####",
    "#...#..#    ",
    "#..# #..#   ",
    "#.#  #..#   ",
    "##    #..#  ",
    "#     #..#  ",
    "       #..# ",
    "        ##  ",
];

pub const ARROW_WIDTH: i32 = 12;
pub const ARROW_HEIGHT: i32 = 19;

/// The arrow as pixels: four bytes each in the order blue, green, red,
/// alpha. Its tip (the hot spot) is the top left pixel.
pub fn arrow() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((ARROW_WIDTH * ARROW_HEIGHT * 4) as usize);
    for row in ARROW {
        for cell in row.bytes() {
            pixels.extend_from_slice(match cell {
                b'#' => &[0, 0, 0, 255],
                b'.' => &[255, 255, 255, 255],
                _ => &[0, 0, 0, 0],
            });
        }
    }
    pixels
}

/// One picture of a cursor theme, ready for the renderer.
pub struct ThemedCursor {
    pub buffer: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    /// The pixel that is the pointer's place, in logical pixels.
    pub hotspot: (i32, i32),
}

/// The cursor theme, and the pictures taken from it so far.
pub struct Cursors {
    theme: xcursor::CursorTheme,
    /// The size asked for, in logical pixels.
    size: u32,
    loaded: std::cell::RefCell<Loaded>,
}

/// The pointers that were looked for, by name and size (`None`: the
/// theme has no such one).
type Loaded = std::collections::HashMap<(String, u32), Option<std::rc::Rc<ThemedCursor>>>;

impl Cursors {
    /// The theme and size of the configuration; where it names none, of
    /// the environment (`XCURSOR_THEME`, `XCURSOR_SIZE`); else "default"
    /// at 24.
    pub fn new(theme: &str, size: u32) -> Self {
        let theme = if theme.trim().is_empty() {
            std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string())
        } else {
            theme.trim().to_string()
        };
        let size = if size > 0 {
            size
        } else {
            std::env::var("XCURSOR_SIZE")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|size| (8..=256).contains(size))
                .unwrap_or(24)
        };
        Self {
            theme: xcursor::CursorTheme::load(&theme),
            size,
            loaded: Default::default(),
        }
    }

    /// The picture of a named cursor for an output of this scale, if the
    /// theme has one.
    pub fn get(
        &self,
        icon: smithay::input::pointer::CursorIcon,
        scale: f64,
    ) -> Option<std::rc::Rc<ThemedCursor>> {
        // Drawn for whole multiples of the size, so it is sharp; a
        // fractional scale takes the next whole one and is scaled down.
        let factor = scale.ceil().clamp(1.0, 4.0) as u32;
        let key = (icon.name().to_string(), factor);
        if let Some(found) = self.loaded.borrow().get(&key) {
            return found.clone();
        }
        let loaded = self.load(icon, factor).map(std::rc::Rc::new);
        self.loaded.borrow_mut().insert(key, loaded.clone());
        loaded
    }

    fn load(&self, icon: smithay::input::pointer::CursorIcon, factor: u32) -> Option<ThemedCursor> {
        // Themes name their pictures by the CSS name or by an older one.
        let path = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .find_map(|name| self.theme.load_icon(name))?;
        let images = xcursor::parser::parse_xcursor(&std::fs::read(path).ok()?)?;
        // The size nearest to the one wanted (the first frame of it: the
        // pictures of one size that follow are an animation).
        let wanted = self.size * factor;
        let image = images
            .iter()
            .min_by_key(|image| (i64::from(image.size) - i64::from(wanted)).abs())?;
        if image.width == 0 || image.height == 0 {
            return None;
        }
        // A theme without that size is shown as large as its picture is.
        let buffer = smithay::backend::renderer::element::memory::MemoryRenderBuffer::from_slice(
            &image.pixels_rgba,
            smithay::backend::allocator::Fourcc::Abgr8888,
            (image.width as i32, image.height as i32),
            factor as i32,
            smithay::utils::Transform::Normal,
            None,
        );
        Some(ThemedCursor {
            buffer,
            hotspot: ((image.xhot / factor) as i32, (image.yhot / factor) as i32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrow_is_an_outlined_shape_with_its_tip_at_the_top_left() {
        assert!(ARROW.iter().all(|row| row.len() == ARROW_WIDTH as usize));
        let pixels = arrow();
        assert_eq!(pixels.len(), (ARROW_WIDTH * ARROW_HEIGHT * 4) as usize);
        let at = |x: i32, y: i32| {
            let index = ((y * ARROW_WIDTH + x) * 4) as usize;
            (pixels[index], pixels[index + 3])
        };
        assert_eq!(at(0, 0), (0, 255), "the tip is outline");
        assert_eq!(at(2, 5), (255, 255), "white inside");
        assert_eq!(at(11, 0).1, 0, "clear beside the tip");
        // Every white pixel has outline or white on both sides in its row.
        for (y, row) in ARROW.iter().enumerate() {
            let bytes = row.as_bytes();
            for (x, cell) in bytes.iter().enumerate() {
                if *cell == b'.' {
                    assert!(x > 0 && bytes[x - 1] != b' ', "row {y}");
                    assert!(x + 1 < bytes.len() && bytes[x + 1] != b' ', "row {y}");
                }
            }
        }
    }
}
