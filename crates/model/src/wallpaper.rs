//! What is behind the windows.

use serde::{Deserialize, Serialize};

/// How a picture that is not the display's shape is put on it.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperFit {
    /// Cover the display; what does not fit is cut off at the sides.
    #[default]
    Fill,
    /// All of the picture, with the colour beside it.
    Fit,
    /// Pulled to the display's shape.
    Stretch,
    /// At its own size, in the middle.
    Centre,
    /// At its own size, again and again.
    Tile,
}

/// The wallpaper of one display that differs from the others'.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct OutputWallpaper {
    /// The connector the display is on (`DP-1`).
    pub name: String,
    /// The picture's file; empty: the colour only.
    pub image: String,
    pub fit: WallpaperFit,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// Pictures of a folder shown one after another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Slideshow {
    /// The folder the pictures are in (PNG, JPEG or WebP; `~` at the
    /// start is the home directory); empty: no slideshow, the picture
    /// named above is shown. Displays with a wallpaper of their own keep
    /// it.
    pub folder: String,
    /// How long each picture stays.
    pub every_minutes: u32,
    /// In an order of chance instead of by name.
    pub shuffle: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for Slideshow {
    fn default() -> Self {
        Self {
            folder: String::new(),
            every_minutes: 30,
            shuffle: false,
            extra: Default::default(),
        }
    }
}

impl Slideshow {
    /// Whether pictures are shown one after another.
    pub fn is_on(&self) -> bool {
        !self.folder.trim().is_empty()
    }

    /// The folder as a path.
    pub fn folder_path(&self) -> std::path::PathBuf {
        home_path(self.folder.trim())
    }

    /// How long a picture stays, in seconds.
    pub fn period_seconds(&self) -> u64 {
        u64::from(self.every_minutes.max(1)) * 60
    }

    /// Which of `count` pictures is shown in the `step`th period (the
    /// periods are counted from a fixed moment, so every display and the
    /// next session agree). By name one after the other; by chance, never
    /// the same twice in a row where there are two or more.
    pub fn slide(&self, count: usize, step: u64) -> Option<usize> {
        if count == 0 {
            return None;
        }
        if !self.shuffle || count <= 2 {
            return Some((step % count as u64) as usize);
        }
        // Round after round every picture once, each round in an order
        // of its own that is the same whenever it is asked for (so that
        // nothing has to be remembered).
        let order = |round: u64| -> Vec<usize> {
            let mut value = round.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut next = || {
                value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut mixed = value;
                mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                mixed ^ (mixed >> 31)
            };
            let mut order: Vec<usize> = (0..count).collect();
            for at in (1..count).rev() {
                order.swap(at, (next() % (at as u64 + 1)) as usize);
            }
            order
        };
        let round = step / count as u64;
        let mut now = order(round);
        // A round does not begin with the picture the one before ended
        // with. (Its end is not what this changes: there are three or
        // more.)
        if round > 0 && order(round - 1).last() == now.first() {
            now.swap(0, 1);
        }
        now.get((step % count as u64) as usize).copied()
    }
}

/// A path of the configuration: `~/` at its start is the home directory.
fn home_path(text: &str) -> std::path::PathBuf {
    // (`~` alone, and `~\` as it is written on Windows, too.)
    let rest = if text == "~" {
        Some("")
    } else {
        text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))
    };
    match rest {
        Some(rest) => {
            dirs::home_dir().map_or_else(|| std::path::PathBuf::from(text), |home| home.join(rest))
        }
        None => std::path::PathBuf::from(text),
    }
}

/// The wallpaper: a picture, or a colour, behind the windows. gevrek draws
/// it itself on Linux. On Windows the system draws it: a picture named
/// here is given to the system, where the system's background is a
/// picture already, and what was there is given back when none is named
/// any more and when gevrek stops; the colour alone changes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WallpaperConfig {
    /// The picture's file (PNG, JPEG or WebP); empty: the colour only.
    pub image: String,
    pub fit: WallpaperFit,
    /// The colour where no picture is.
    pub color: (u8, u8, u8),
    /// Displays with a wallpaper of their own.
    pub outputs: Vec<OutputWallpaper>,
    /// Pictures of a folder, one after another, in place of the one
    /// picture.
    pub slideshow: Slideshow,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            image: String::new(),
            fit: WallpaperFit::Fill,
            // The colour gevrek has always had behind its windows.
            color: (18, 18, 23),
            outputs: Vec::new(),
            slideshow: Default::default(),
            extra: Default::default(),
        }
    }
}

impl WallpaperConfig {
    /// The picture and the fit for the display with this name: its own
    /// entry's, else the common one's. `~` at the start of a path is the
    /// home directory.
    pub fn of(&self, output: &str) -> (Option<std::path::PathBuf>, WallpaperFit) {
        let (image, fit) = self
            .outputs
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(output))
            .map_or((self.image.as_str(), self.fit), |entry| {
                (entry.image.as_str(), entry.fit)
            });
        let image = image.trim();
        if image.is_empty() {
            return (None, fit);
        }
        (Some(home_path(image)), fit)
    }

    /// Whether the display with this name shows the slideshow: it is on,
    /// and the display has no wallpaper of its own.
    pub fn slides_on(&self, output: &str) -> bool {
        self.slideshow.is_on()
            && !self
                .outputs
                .iter()
                .any(|entry| entry.name.eq_ignore_ascii_case(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_has_its_own_wallpaper_or_the_common_one() {
        let config: WallpaperConfig = serde_json::from_str(
            r#"{"image": "/pictures/all.png", "fit": "fit",
                "outputs": [{"name": "DP-2", "image": "/pictures/second.jpg", "fit": "tile"}, {"name": "HDMI-A-1"}]}"#,
        )
        .expect("a wallpaper");
        assert_eq!(
            config.of("DP-1"),
            (Some("/pictures/all.png".into()), WallpaperFit::Fit)
        );
        assert_eq!(
            config.of("dp-2"),
            (Some("/pictures/second.jpg".into()), WallpaperFit::Tile)
        );
        // An entry without a picture: that display has the colour only.
        assert_eq!(config.of("HDMI-A-1"), (None, WallpaperFit::Fill));
        // A configuration from before there was a wallpaper.
        let old = WallpaperConfig::default();
        assert_eq!(old.of("DP-1"), (None, WallpaperFit::Fill));
        assert_eq!(old.color, (18, 18, 23));
    }

    #[test]
    fn a_slideshow_goes_through_its_pictures() {
        let off = WallpaperConfig::default();
        assert!(!off.slideshow.is_on() && !off.slides_on("DP-1"));
        let config: WallpaperConfig = serde_json::from_str(
            r#"{"image": "/pictures/all.png", "slideshow": {"folder": "/pictures/many"},
                "outputs": [{"name": "DP-2", "image": "/pictures/second.jpg"}]}"#,
        )
        .expect("a wallpaper");
        // A display with its own keeps it.
        assert!(config.slides_on("DP-1") && !config.slides_on("dp-2"));
        let show = &config.slideshow;
        assert_eq!(show.period_seconds(), 30 * 60);
        // By name: one after the other, round and round.
        let order: Vec<usize> = (0..7).filter_map(|step| show.slide(3, step)).collect();
        assert_eq!(order, [0, 1, 2, 0, 1, 2, 0]);
        assert_eq!(show.slide(0, 5), None);
        // By chance: the same for the same step, every picture in time,
        // never one twice in a row.
        let chance = Slideshow {
            shuffle: true,
            ..show.clone()
        };
        let order: Vec<usize> = (0..200).filter_map(|step| chance.slide(5, step)).collect();
        assert_eq!(order.len(), 200);
        assert!(order.windows(2).all(|pair| pair[0] != pair[1]), "{order:?}");
        assert!((0..5).all(|picture| order.contains(&picture)));
        assert_eq!(chance.slide(5, 77), chance.slide(5, 77));
        assert_ne!(order[..20], [0, 1, 2, 3, 4].repeat(4)[..]);
        assert_eq!(chance.slide(1, 9), Some(0));
    }
}
