//! The colour that stands out in the system's wallpaper, for a theme whose
//! accent is the wallpaper's (`desktop.appearance.theme.accent:
//! "wallpaper"`). On Windows the system draws the wallpaper; gevrek only
//! reads the picture the system says it shows (it changes nothing), and
//! takes the colour by the rule every system uses
//! (`gevrek_model::theme::accent_of_pixels`).
//!
//! The picture is read on a thread of its own (a large one takes a tenth
//! of a second to open) and again when its file is another or was written
//! since. A desktop without a picture has its plain colour.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::SystemTime;

type Colour = (u8, u8, u8);

/// What the wallpaper is now, as far as its colour depends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    /// A picture: its file, and when that was last written.
    Picture(PathBuf, Option<SystemTime>),
    /// No picture: the desktop's plain colour.
    Plain(Colour),
}

/// Follows the system's wallpaper for its colour.
#[derive(Default)]
pub struct Watch {
    /// What the colour was last taken from.
    taken_from: Option<Shown>,
    /// A picture that is being read.
    reading: Option<mpsc::Receiver<Option<Colour>>>,
    /// The colour, once it is known.
    colour: Option<Colour>,
}

impl Watch {
    /// Look at the wallpaper again (cheap: a name and a date; the picture
    /// is opened elsewhere and only when it is another). Returns the
    /// colour as far as it is known now.
    pub fn look(&mut self) -> Option<Colour> {
        if let Some(reading) = &self.reading {
            match reading.try_recv() {
                Ok(colour) => {
                    self.colour = colour;
                    self.reading = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.reading = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let shown = shown();
        if self.taken_from.as_ref() != Some(&shown) && self.reading.is_none() {
            match &shown {
                Shown::Plain(colour) => {
                    self.colour = gevrek_model::theme::accent_of_pixels([*colour]);
                }
                Shown::Picture(file, _) => {
                    let (tell, told) = mpsc::channel();
                    let file = file.clone();
                    let started = std::thread::Builder::new()
                        .name("gevrek-wallpaper-colour".to_string())
                        .spawn(move || {
                            let _ = tell.send(colour_of_file(&file));
                        });
                    if started.is_ok() {
                        self.reading = Some(told);
                    }
                }
            }
            self.taken_from = Some(shown);
        }
        self.colour
    }

    /// Nothing is followed any more (the theme has an accent of its own).
    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

/// The colour that stands out in a picture file; `None` when it cannot be
/// read or has no colour to speak of.
pub fn colour_of_file(file: &std::path::Path) -> Option<Colour> {
    let picture = match image::open(file) {
        Ok(picture) => picture,
        Err(error) => {
            log::debug!("the wallpaper {} was not read: {error}", file.display());
            return None;
        }
    };
    // (A small copy says as much of its colours as all of it.)
    gevrek_model::theme::accent_of_pixels(
        picture
            .thumbnail(64, 64)
            .to_rgb8()
            .pixels()
            .map(|pixel| (pixel[0], pixel[1], pixel[2])),
    )
}

/// What the system shows as the desktop's background.
fn shown() -> Shown {
    use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_DESKTOP};
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut name = [0u16; 520];
    // SAFETY: the system writes the file's name into the buffer, whose
    // length it is told.
    let asked = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            name.len() as u32,
            Some(name.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    let length = name.iter().position(|unit| *unit == 0).unwrap_or(0);
    if asked.is_ok() && length > 0 {
        let file = PathBuf::from(String::from_utf16_lossy(&name[..length]));
        let written = std::fs::metadata(&file)
            .and_then(|about| about.modified())
            .ok();
        return Shown::Picture(file, written);
    }
    // SAFETY: a plain query. (The colour is 0x00BBGGRR.)
    let colour = unsafe { GetSysColor(COLOR_DESKTOP) };
    Shown::Plain((
        (colour & 0xFF) as u8,
        ((colour >> 8) & 0xFF) as u8,
        ((colour >> 16) & 0xFF) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pictures_colour_is_read_from_its_file() {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-wallpaper-colour-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        std::fs::create_dir_all(&folder).unwrap();
        // Mostly a strong orange, with some grey: the orange stands out.
        let file = folder.join("picture.png");
        let mut picture = image::RgbImage::from_pixel(40, 40, image::Rgb([230, 120, 20]));
        for x in 0..40 {
            for y in 0..8 {
                picture.put_pixel(x, y, image::Rgb([90, 90, 90]));
            }
        }
        picture.save(&file).unwrap();
        let (red, green, blue) = colour_of_file(&file).expect("a colour");
        assert!(red > green && green > blue, "{red} {green} {blue}");

        // A picture without colour, and what is no picture at all.
        let grey = folder.join("grey.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([120, 120, 120]))
            .save(&grey)
            .unwrap();
        assert_eq!(colour_of_file(&grey), None);
        let none = folder.join("none.png");
        std::fs::write(&none, b"not a picture").unwrap();
        assert_eq!(colour_of_file(&none), None);
        assert_eq!(colour_of_file(&folder.join("missing.png")), None);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_system_says_what_it_shows() {
        // Whatever this machine shows: asking does not fail, and asked
        // twice it is the same.
        assert_eq!(shown(), shown());
    }
}
