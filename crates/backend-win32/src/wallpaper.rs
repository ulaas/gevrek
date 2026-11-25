//! The wallpaper on Windows: the system draws it, so what `desktop.wallpaper`
//! names is given to the system (`IDesktopWallpaper`), display by display,
//! and what the person had is given back.
//!
//! The rules, each from something that went wrong or could:
//!
//! - A configuration names a wallpaper when it names a picture (the common
//!   one, a display's own, or a slideshow's folder). Its colour alone does
//!   not: every configuration has one. A configuration that names none is
//!   never a reason to touch the system's background.
//! - A background that is a picture, a plain colour or the system's own
//!   slideshow of one folder is changed, and given back as what it was:
//!   the colour by switching the wallpaper off (an empty picture brings a
//!   default picture, not the colour), the slideshow by its folder, its
//!   options and its time (both tried on build 26300, 2026-11-15).
//!   Spotlight, and a slideshow of several places, cannot be put back
//!   through this interface: they are left as they are and a notice says
//!   so.
//! - What kind the background is, is what the interface says of it
//!   (switched off, a slideshow, a picture), not what Settings wrote
//!   down: that is not changed when a program sets a picture. Only
//!   Spotlight is known by Settings' word alone.
//! - A colour or a slideshow given back is held for a few seconds: the
//!   system finishes a picture that was set shortly before after the
//!   giving back, and shows it again (seen on build 26300). While a
//!   display shows a picture gevrek had given, it is given back again.
//! - A kind the person chooses while gevrek's pictures are shown is
//!   theirs: it is left, without a word, until it is another.
//! - What was there is read before the first change (each display's
//!   picture, the position, the colour) and written down with the
//!   instance's state, so that it is given back also after a window
//!   manager that was ended from outside: when the configuration names no
//!   wallpaper any more, and when the window manager stops.
//! - A picture somebody else sets afterwards (the person in Settings,
//!   another program) is theirs: it stays until the configuration asks for
//!   another picture than it did, and it is what is given back.
//! - A display whose entry names no picture keeps what it has: the system
//!   has one colour for all displays, not one a display.
//! - The system has one position for all displays: the common entry's fit,
//!   or the first display's own where there is no common picture.
//! - A file that is not there, or of a kind the system does not show
//!   (WebP), changes nothing and is said once.

use gevrek_engine::overlay_data::Notice;
use gevrek_model::wallpaper::{WallpaperConfig, WallpaperFit};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What kind of background the system has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Picture,
    Plain,
    Slideshow,
    Spotlight,
}

/// The system's desktop background, as far as gevrek asks and sets it.
pub trait Background {
    fn kind(&self) -> Kind;
    /// The displays that are part of the desktop, by the system's lasting
    /// names for them (device paths).
    fn displays(&self) -> Vec<String>;
    /// The picture a display shows; empty: none.
    fn picture(&self, display: &str) -> Option<String>;
    fn set_picture(&mut self, display: &str, file: &str) -> bool;
    /// How pictures are put on the displays, as the system numbers it.
    fn position(&self) -> Option<i32>;
    fn set_position(&mut self, position: i32) -> bool;
    /// The colour where no picture is (0x00BBGGRR).
    fn colour(&self) -> Option<u32>;
    fn set_colour(&mut self, colour: u32) -> bool;
    /// The system's own slideshow, where it shows one that can be given
    /// back (of one place).
    fn slideshow(&self) -> Option<Slides> {
        None
    }
    fn set_slideshow(&mut self, _slides: &Slides) -> bool {
        false
    }
    /// No picture on any display: the plain colour.
    fn set_plain(&mut self) -> bool {
        false
    }
}

/// The system's own slideshow: the folder it shows, its options as the
/// system numbers them, and how long a picture stays, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Slides {
    pub place: String,
    pub options: i32,
    pub tick: u32,
}

/// The system's number for a fit (`DESKTOP_WALLPAPER_POSITION`).
pub fn position_of(fit: WallpaperFit) -> i32 {
    match fit {
        WallpaperFit::Centre => 0,
        WallpaperFit::Tile => 1,
        WallpaperFit::Stretch => 2,
        WallpaperFit::Fit => 3,
        WallpaperFit::Fill => 4,
    }
}

/// Whether the configuration names a wallpaper: a picture somewhere.
pub fn names_one(config: &WallpaperConfig) -> bool {
    !config.image.trim().is_empty()
        || config.slideshow.is_on()
        || config
            .outputs
            .iter()
            .any(|entry| !entry.image.trim().is_empty())
}

/// The kinds of picture the system shows as a wallpaper.
const KINDS: [&str; 4] = ["png", "jpg", "jpeg", "bmp"];

fn shown_by_the_system(file: &Path) -> bool {
    file.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| KINDS.iter().any(|kind| extension.eq_ignore_ascii_case(kind)))
}

/// The pictures of a slideshow's folder the system can show, by name.
fn pictures_in(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| shown_by_the_system(path) && path.is_file())
        .collect();
    found.sort();
    found
}

/// A file's name as the system is told it: whole, with its own strokes.
fn told(file: &Path) -> String {
    std::path::absolute(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
}

fn same_file(a: &str, b: &str) -> bool {
    a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"))
}

/// What the person had before gevrek changed anything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Own {
    /// Each display's picture, by the system's name for the display.
    pictures: Vec<(String, String)>,
    position: Option<i32>,
    colour: Option<u32>,
    /// It was the plain colour, no picture.
    #[serde(default)]
    plain: bool,
    /// It was the system's own slideshow, this one.
    #[serde(default)]
    slides: Option<Slides>,
}

/// How long a colour or a slideshow that was given back is held.
const HELD_FOR: std::time::Duration = std::time::Duration::from_secs(4);

/// What was given back that is no picture, and the pictures gevrek had
/// given before: should one of them be shown again, it is given back
/// again.
struct Held {
    slides: Option<Slides>,
    given: Vec<String>,
    until: std::time::Instant,
}

/// The wallpaper as gevrek has given it to the system.
pub struct Wallpaper {
    /// Where what the person had is written down; `None`: this one never
    /// touches anything (tests of the runtime).
    file: Option<PathBuf>,
    /// The file was looked for (once, at the first look).
    read: bool,
    own: Option<Own>,
    /// The picture gevrek gave each display.
    given: HashMap<String, String>,
    /// The position and colour gevrek gave, and what they were given for.
    look: Option<(i32, u32)>,
    /// A display somebody else gave a picture after gevrek: the picture
    /// gevrek wanted there then. It is left until that is another.
    theirs: HashMap<String, String>,
    /// The kind of background that is not touched, while that is so.
    refused: Option<Kind>,
    /// The kind the person chose while gevrek's pictures were shown: it
    /// is left while it is that.
    theirs_kind: Option<Kind>,
    /// A colour or a slideshow that was given back and is held for a
    /// while.
    held: Option<Held>,
    /// Files that were said to be missing or of no kind the system shows.
    said: HashSet<String>,
    /// The slideshow's folder, its pictures, and when they were listed.
    slides: Option<(PathBuf, Vec<PathBuf>, std::time::Instant)>,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self::kept_in(Some(
            gevrek_model::paths::state_dir().join("windows-wallpaper.json"),
        ))
    }
}

impl Wallpaper {
    /// One that writes down what the person had in this file.
    pub fn kept_in(file: Option<PathBuf>) -> Self {
        Self {
            file,
            read: false,
            own: None,
            given: HashMap::new(),
            look: None,
            theirs: HashMap::new(),
            refused: None,
            theirs_kind: None,
            held: None,
            said: HashSet::new(),
            slides: None,
        }
    }

    /// One that never touches the system's background.
    pub fn nowhere() -> Self {
        Self::kept_in(None)
    }

    /// What is said while the configuration names a wallpaper and the
    /// system's background is of a kind gevrek does not change.
    pub fn notice(&self) -> Option<Notice> {
        let kind = match self.refused? {
            Kind::Plain => "a plain colour",
            Kind::Slideshow => "a slideshow of Windows' own of more than one folder",
            Kind::Spotlight => "Windows Spotlight",
            Kind::Picture => return None,
        };
        Some(Notice {
            id: "wallpaper-not-changed".to_string(),
            title: "The wallpaper of the configuration is not shown".to_string(),
            text: format!(
                "desktop.wallpaper names a picture, and Windows' background is {kind}, which \
                 gevrek could not give back afterwards: it is left as it is. Choose a picture, a \
                 colour or a slideshow of one folder as the background once (Settings, \
                 Personalisation, Background), and gevrek shows its own and gives yours back \
                 when it stops."
            ),
        })
    }

    /// The slideshow's picture for this moment.
    fn slide(&mut self, config: &WallpaperConfig, seconds: u64) -> Option<PathBuf> {
        let show = &config.slideshow;
        if !show.is_on() {
            self.slides = None;
            return None;
        }
        let folder = show.folder_path();
        let stale = self.slides.as_ref().is_none_or(|(known, _, listed)| {
            *known != folder || listed.elapsed() > std::time::Duration::from_secs(60)
        });
        if stale {
            let pictures = pictures_in(&folder);
            if pictures.is_empty() && self.said.insert(folder.display().to_string()) {
                log::warn!(
                    "The slideshow's folder {} has no pictures Windows shows (PNG, JPEG, BMP)",
                    folder.display()
                );
            }
            self.slides = Some((folder, pictures, std::time::Instant::now()));
        }
        let pictures = &self.slides.as_ref()?.1;
        show.slide(pictures.len(), seconds / show.period_seconds())
            .and_then(|at| pictures.get(at).cloned())
    }

    /// What was written down by a window manager that did not give it
    /// back (it was ended from outside): what the displays show now is
    /// then gevrek's.
    fn read_what_was_kept(&mut self, system: &dyn Background) {
        if std::mem::replace(&mut self.read, true) {
            return;
        }
        let Some(own) = self
            .file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str::<Own>(&text).ok())
        else {
            return;
        };
        log::info!("The wallpaper of before was not given back the last time: it is known still");
        for display in system.displays() {
            if let Some(picture) = system.picture(&display) {
                self.given.insert(display, picture);
            }
        }
        if let (Some(position), Some(colour)) = (system.position(), system.colour()) {
            self.look = Some((position, colour));
        }
        self.own = Some(own);
    }

    fn keep(&self) {
        let (Some(file), Some(own)) = (&self.file, &self.own) else {
            return;
        };
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let written = serde_json::to_string_pretty(own)
            .map_err(std::io::Error::other)
            .and_then(|text| std::fs::write(file, text));
        if let Err(error) = written {
            log::warn!("what the wallpaper was before is not written down: {error}");
        }
    }

    /// Read what the person has, before the first change.
    fn remember(&mut self, system: &dyn Background) {
        if self.own.is_some() {
            return;
        }
        let kind = system.kind();
        self.own = Some(Own {
            plain: kind == Kind::Plain,
            slides: (kind == Kind::Slideshow)
                .then(|| system.slideshow())
                .flatten(),
            pictures: system
                .displays()
                .into_iter()
                .filter_map(|display| {
                    let picture = system.picture(&display)?;
                    Some((display, picture))
                })
                .collect(),
            position: system.position(),
            colour: system.colour(),
        });
        self.keep();
    }

    /// Bring the system's background in line with the configuration.
    /// `names`: what a display (by the system's name for it) is called in
    /// the configuration. `seconds`: the clock, for the slideshow. `true`
    /// when there is a notice that was not there, or one less.
    pub fn follow(
        &mut self,
        config: &WallpaperConfig,
        system: &mut dyn Background,
        names: &dyn Fn(&str) -> Vec<String>,
        seconds: u64,
    ) -> bool {
        if self.file.is_none() {
            return false;
        }
        let refused_before = self.refused;
        self.read_what_was_kept(system);
        if names_one(config) {
            self.held = None;
        } else {
            self.hold(system);
        }
        if !names_one(config) {
            self.refused = None;
            self.theirs_kind = None;
            self.give_back(system);
            return self.refused != refused_before;
        }
        let mut kind = system.kind();
        // (The system says for a moment after a picture was set that it
        // shows none, or still its slideshow: seen on build 26300. While
        // a display shows what gevrek gave it, the background is
        // gevrek's pictures, whatever is said of its kind.)
        if kind != Kind::Picture && self.own.is_some() && self.still_shown(system) {
            kind = Kind::Picture;
        }
        if kind == Kind::Picture {
            self.theirs_kind = None;
        } else {
            // Chosen by the person since, if gevrek had given pictures:
            // theirs now, and nothing of before is to be given back.
            if self.own.is_some() {
                log::info!("Windows' background is no picture any more: gevrek lets go of it");
                self.forget();
                self.theirs_kind = Some(kind);
            }
            if self.theirs_kind == Some(kind) {
                self.refused = None;
                return self.refused != refused_before;
            }
            self.theirs_kind = None;
            // A kind that can be given back as it was is changed like a
            // picture; another is left, and that is said.
            let given_back = match kind {
                Kind::Plain => true,
                Kind::Slideshow => system.slideshow().is_some(),
                Kind::Spotlight | Kind::Picture => false,
            };
            if !given_back {
                if self.refused != Some(kind) {
                    log::warn!(
                        "desktop.wallpaper names a picture and Windows' background is {kind:?}: \
                         it is not changed"
                    );
                }
                self.refused = Some(kind);
                return self.refused != refused_before;
            }
        }
        self.refused = None;

        let slide = self.slide(config, seconds);
        let mut any_given = false;
        for display in system.displays() {
            let Some(shows) = system.picture(&display) else {
                continue;
            };
            // What gevrek gave and somebody changed since is theirs.
            if let Some(given) = self.given.get(&display) {
                if !same_file(given, &shows) {
                    log::info!("The wallpaper of a display was changed by someone else: it stays");
                    let wanted_then = self.given.remove(&display).unwrap_or_default();
                    self.theirs.insert(display.clone(), wanted_then);
                    if let Some(own) = self.own.as_mut() {
                        own.pictures.retain(|(known, _)| *known != display);
                        own.pictures.push((display.clone(), shows.clone()));
                    }
                    self.keep();
                }
            }
            // The display's entry, by any of the names it has.
            let called = names(&display);
            let entry = called
                .iter()
                .find(|name| {
                    config
                        .outputs
                        .iter()
                        .any(|entry| entry.name.eq_ignore_ascii_case(name))
                })
                .map_or("", String::as_str);
            let wanted = if config.slides_on(entry) {
                slide.clone()
            } else {
                config.of(entry).0
            };
            let Some(wanted) = wanted else {
                continue;
            };
            let wanted_text = told(&wanted);
            if !shown_by_the_system(&wanted) || !wanted.is_file() {
                if self.said.insert(wanted_text.clone()) {
                    log::warn!(
                        "The wallpaper {wanted_text} is not there or is no picture Windows shows \
                         (PNG, JPEG, BMP): the display keeps what it has"
                    );
                }
                continue;
            }
            if self
                .theirs
                .get(&display)
                .is_some_and(|then| same_file(then, &wanted_text))
            {
                continue;
            }
            self.theirs.remove(&display);
            if same_file(&shows, &wanted_text) {
                any_given |= self.given.contains_key(&display);
                continue;
            }
            self.remember(system);
            if system.set_picture(&display, &wanted_text) {
                log::info!("The wallpaper of a display is {wanted_text}");
                self.given.insert(display, wanted_text);
                any_given = true;
            } else {
                log::warn!("Windows did not take {wanted_text} as a display's wallpaper");
            }
        }

        // One position and one colour for all displays, where gevrek has
        // given a picture at all.
        if any_given {
            let fit = if !config.image.trim().is_empty() || config.slideshow.is_on() {
                config.fit
            } else {
                config
                    .outputs
                    .iter()
                    .find(|entry| !entry.image.trim().is_empty())
                    .map_or(config.fit, |entry| entry.fit)
            };
            let (red, green, blue) = config.color;
            let wanted = (
                position_of(fit),
                u32::from(red) | u32::from(green) << 8 | u32::from(blue) << 16,
            );
            if self.look != Some(wanted) {
                if system.position() != Some(wanted.0) {
                    system.set_position(wanted.0);
                }
                if system.colour() != Some(wanted.1) {
                    system.set_colour(wanted.1);
                }
                self.look = Some(wanted);
            }
        }
        self.refused != refused_before
    }

    /// Whether any display still shows the picture gevrek gave it.
    fn still_shown(&self, system: &dyn Background) -> bool {
        self.given.iter().any(|(display, given)| {
            system
                .picture(display)
                .is_some_and(|shows| same_file(given, &shows))
        })
    }

    /// Give the person what they had: each display's picture where the
    /// one it shows is still gevrek's, and the position and the colour
    /// where they are.
    pub fn give_back(&mut self, system: &mut dyn Background) {
        let Some(own) = self.own.take() else {
            return;
        };
        let there = system.displays();
        // Whether any display still shows what gevrek gave it.
        let still_ours = self.still_shown(system);
        let no_pictures = own.plain || own.slides.is_some();
        for (display, picture) in &own.pictures {
            // (A colour had none; a slideshow's are its own to choose.)
            if no_pictures {
                break;
            }
            let ours = self.given.get(display).is_some_and(|given| {
                system
                    .picture(display)
                    .is_some_and(|shows| same_file(given, &shows))
            });
            if !ours || picture.is_empty() || !there.contains(display) {
                continue;
            }
            if !system.set_picture(display, picture) {
                log::warn!("Windows did not take a display's wallpaper of before back: {picture}");
            }
        }
        if let Some((position, colour)) = self.look {
            if let Some(was) = own.position {
                if system.position() == Some(position) && was != position {
                    system.set_position(was);
                }
            }
            if let Some(was) = own.colour {
                if system.colour() == Some(colour) && was != colour {
                    system.set_colour(was);
                }
            }
        }
        // What was no picture, as what it was: where gevrek's pictures
        // are still shown.
        let mut held = None;
        if no_pictures && still_ours {
            let back = match &own.slides {
                Some(slides) => system.set_slideshow(slides),
                None => system.set_plain(),
            };
            held = Some(Held {
                slides: own.slides.clone(),
                given: self.given.values().cloned().collect(),
                until: std::time::Instant::now() + HELD_FOR,
            });
            if !back {
                log::warn!(
                    "Windows did not take the background of before back ({})",
                    if own.slides.is_some() {
                        "its slideshow"
                    } else {
                        "the plain colour"
                    }
                );
            }
        }
        log::info!("The wallpaper is the one of before again");
        self.forget();
        self.held = held;
    }

    /// Whether a colour or a slideshow that was given back is still held
    /// (`hold` is to be called meanwhile).
    pub fn holds(&self) -> bool {
        self.held.is_some()
    }

    /// Look whether what was given back is still there, and give it back
    /// again where a picture gevrek had given is shown once more.
    pub fn hold(&mut self, system: &mut dyn Background) {
        let Some(held) = &self.held else {
            return;
        };
        if std::time::Instant::now() >= held.until {
            self.held = None;
            return;
        }
        let again = system.displays().iter().any(|display| {
            system.picture(display).is_some_and(|shows| {
                !shows.is_empty() && held.given.iter().any(|given| same_file(given, &shows))
            })
        });
        if again {
            log::info!("A picture gevrek had given is shown again: the background of before is given back once more");
            match &held.slides {
                Some(slides) => system.set_slideshow(slides),
                None => system.set_plain(),
            };
        }
    }

    /// Nothing is gevrek's any more, and nothing is to be given back.
    fn forget(&mut self) {
        self.own = None;
        self.given.clear();
        self.theirs.clear();
        self.look = None;
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
    }

    /// Whether anything of the system's background is gevrek's now.
    pub fn has_given(&self) -> bool {
        self.own.is_some()
    }

    /// Whether the system has to be asked at all: the configuration names
    /// a wallpaper, or something is to be given back (also what a window
    /// manager before this one left written down).
    pub fn has_work(&self, config: &WallpaperConfig) -> bool {
        let Some(file) = &self.file else {
            return false;
        };
        names_one(config)
            || self.own.is_some()
            || self.held.is_some()
            || self.refused.is_some()
            || (!self.read && file.exists())
    }
}

/// The names a display is known by in a configuration: the one `gevrek
/// state` shows (also without the number that tells two of a model apart),
/// its device path, and Windows' short name (`DISPLAY1`).
pub fn names_of(display: &str, known: &[(String, String, String)]) -> Vec<String> {
    let mut names = vec![display.to_string()];
    for (device, name, path) in known {
        if path.eq_ignore_ascii_case(display) || device.eq_ignore_ascii_case(display) {
            names.push(name.clone());
            names.push(gevrek_engine::outputs::name_without_number(name).to_string());
            names.push(device.clone());
            names.push(device.trim_start_matches(r"\\.\").to_string());
        }
    }
    names
}

/// What the system's background is now, for whoever asks (`gevrek
/// wallpaper`): its kind, each display's picture, the position as the
/// system numbers it and the colour. Only read.
pub fn shown_now() -> Option<serde_json::Value> {
    let desktop = system::Desktop::open()?;
    let kind = match desktop.kind() {
        Kind::Picture => "picture",
        Kind::Plain => "colour",
        Kind::Slideshow => "slideshow",
        Kind::Spotlight => "spotlight",
    };
    let displays: Vec<serde_json::Value> = desktop
        .displays()
        .into_iter()
        .map(|display| {
            let picture = desktop.picture(&display).unwrap_or_default();
            serde_json::json!({"display": display, "picture": picture})
        })
        .collect();
    let colour = desktop
        .colour()
        .map(|colour| [colour & 0xFF, (colour >> 8) & 0xFF, (colour >> 16) & 0xFF]);
    // (The system's own slideshow, where it is one of one place.)
    let slideshow = desktop.slideshow().map(|slides| {
        serde_json::json!({"place": slides.place, "options": slides.options, "tick": slides.tick})
    });
    Some(serde_json::json!({
        "kind": kind,
        "displays": displays,
        "position": desktop.position(),
        "color": colour,
        "slideshow": slideshow,
    }))
}

/// The system itself.
pub mod system {
    use super::{Background, Kind};
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows::Win32::UI::Shell::{
        DesktopWallpaper, IDesktopWallpaper, IShellItem, IShellItemArray,
        SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem,
        DESKTOP_SLIDESHOW_OPTIONS, DESKTOP_WALLPAPER_POSITION, SIGDN_FILESYSPATH,
    };

    /// Windows' desktop wallpaper, for the thread it was asked for on.
    pub struct Desktop {
        /// (Let go of by hand, before the thread says it is done with
        /// the shell's objects.)
        api: std::mem::ManuallyDrop<IDesktopWallpaper>,
        /// The thread was set up for the shell's objects here.
        set_up: bool,
    }

    impl Desktop {
        pub fn open() -> Option<Self> {
            // SAFETY: the thread is set up for the shell's objects (a
            // thread that was already, in whichever way, stays so) and
            // told once that we are done when that said yes.
            unsafe {
                let set_up =
                    CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
                match CoCreateInstance::<_, IDesktopWallpaper>(&DesktopWallpaper, None, CLSCTX_ALL)
                {
                    Ok(api) => Some(Self {
                        api: std::mem::ManuallyDrop::new(api),
                        set_up,
                    }),
                    Err(error) => {
                        log::warn!("Windows' wallpaper cannot be asked for: {error}");
                        if set_up {
                            CoUninitialize();
                        }
                        None
                    }
                }
            }
        }
    }

    impl Drop for Desktop {
        fn drop(&mut self) {
            // SAFETY: let go of once, here, and not used after. First
            // the object, then the thread's word that it is done: a
            // field is let go of after this function, which was after
            // `CoUninitialize`.
            unsafe { std::mem::ManuallyDrop::drop(&mut self.api) };
            if self.set_up {
                // SAFETY: the one telling for the one setting up above.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Text the system handed over, read and given back to it.
    unsafe fn taken(text: PWSTR) -> String {
        if text.is_null() {
            return String::new();
        }
        let read = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.as_ptr().cast()));
        read
    }

    /// What Settings says the background is (0 a picture, 1 a colour, 2 a
    /// slideshow, 3 Spotlight), where it says.
    fn chosen_kind() -> Option<u32> {
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the value is a number of the size the call is told.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers"),
                &HSTRING::from("BackgroundType"),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut value as *mut u32).cast()),
                Some(&mut size),
            )
        };
        read.is_ok().then_some(value)
    }

    impl Background for Desktop {
        fn kind(&self) -> Kind {
            // (Spotlight is known by Settings' word alone. The other
            // kinds by what the interface says: Settings' word stays as
            // it was when a program sets a picture.)
            if chosen_kind() == Some(3) {
                return Kind::Spotlight;
            }
            // SAFETY: a plain query.
            match unsafe { self.api.GetStatus() }.map(|status| status.0) {
                // Switched off: the colour.
                Ok(status) if status & 1 == 0 => return Kind::Plain,
                Ok(status) if status & 2 != 0 => return Kind::Slideshow,
                _ => {}
            }
            // A picture: if a display shows one.
            let any = self
                .displays()
                .iter()
                .any(|display| self.picture(display).is_some_and(|file| !file.is_empty()));
            if any {
                Kind::Picture
            } else {
                Kind::Plain
            }
        }

        fn displays(&self) -> Vec<String> {
            // SAFETY: plain calls; text handed over is given back.
            unsafe {
                let count = self.api.GetMonitorDevicePathCount().unwrap_or(0);
                (0..count)
                    .filter_map(|index| {
                        let path = taken(self.api.GetMonitorDevicePathAt(index).ok()?);
                        // (One that is not part of the desktop now has no place.)
                        self.api.GetMonitorRECT(&HSTRING::from(&path)).ok()?;
                        Some(path)
                    })
                    .collect()
            }
        }

        fn picture(&self, display: &str) -> Option<String> {
            // SAFETY: as above.
            unsafe {
                self.api
                    .GetWallpaper(&HSTRING::from(display))
                    .ok()
                    .map(|text| taken(text))
            }
        }

        fn set_picture(&mut self, display: &str, file: &str) -> bool {
            // SAFETY: both texts live until the call returns.
            unsafe {
                self.api
                    .SetWallpaper(&HSTRING::from(display), &HSTRING::from(file))
                    .is_ok()
            }
        }

        fn position(&self) -> Option<i32> {
            // SAFETY: a plain query.
            unsafe { self.api.GetPosition().ok().map(|position| position.0) }
        }

        fn set_position(&mut self, position: i32) -> bool {
            // SAFETY: a plain call.
            unsafe {
                self.api
                    .SetPosition(DESKTOP_WALLPAPER_POSITION(position))
                    .is_ok()
            }
        }

        fn colour(&self) -> Option<u32> {
            // SAFETY: a plain query.
            unsafe { self.api.GetBackgroundColor().ok().map(|colour| colour.0) }
        }

        fn set_colour(&mut self, colour: u32) -> bool {
            // SAFETY: a plain call.
            unsafe { self.api.SetBackgroundColor(COLORREF(colour)).is_ok() }
        }

        fn slideshow(&self) -> Option<super::Slides> {
            // SAFETY: plain queries; text handed over is given back.
            unsafe {
                let items = self.api.GetSlideshow().ok()?;
                // (Of one place only: that is what can be given back.)
                if items.GetCount().ok()? != 1 {
                    return None;
                }
                let place = taken(items.GetItemAt(0).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?);
                let mut options = DESKTOP_SLIDESHOW_OPTIONS(0);
                let mut tick = 0u32;
                self.api.GetSlideshowOptions(&mut options, &mut tick).ok()?;
                (!place.is_empty()).then_some(super::Slides {
                    place,
                    options: options.0,
                    tick,
                })
            }
        }

        fn set_slideshow(&mut self, slides: &super::Slides) -> bool {
            // SAFETY: the texts and the objects live until the calls return.
            unsafe {
                let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    &HSTRING::from(slides.place.as_str()),
                    None,
                ) else {
                    return false;
                };
                let Ok(items) = SHCreateShellItemArrayFromShellItem::<_, IShellItemArray>(&item)
                else {
                    return false;
                };
                self.api.SetSlideshow(&items).is_ok()
                    && self
                        .api
                        .SetSlideshowOptions(DESKTOP_SLIDESHOW_OPTIONS(slides.options), slides.tick)
                        .is_ok()
            }
        }

        fn set_plain(&mut self) -> bool {
            // SAFETY: a plain call.
            unsafe { self.api.Enable(false).is_ok() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop of two displays that remembers what it is told.
    struct Fake {
        kind: Kind,
        pictures: Vec<(String, String)>,
        position: i32,
        colour: u32,
        set: usize,
        slides: Option<Slides>,
    }

    impl Fake {
        fn with(kind: Kind, first: &str, second: &str) -> Self {
            Self {
                kind,
                pictures: vec![
                    ("one".to_string(), first.to_string()),
                    ("two".to_string(), second.to_string()),
                ],
                position: 5,
                colour: 0x00112233,
                set: 0,
                slides: None,
            }
        }

        fn shows(&self, display: &str) -> String {
            self.picture(display).unwrap_or_default()
        }
    }

    impl Background for Fake {
        fn kind(&self) -> Kind {
            self.kind
        }
        fn displays(&self) -> Vec<String> {
            self.pictures.iter().map(|(display, _)| display.clone()).collect()
        }
        fn picture(&self, display: &str) -> Option<String> {
            self.pictures
                .iter()
                .find(|(known, _)| known == display)
                .map(|(_, picture)| picture.clone())
        }
        fn set_picture(&mut self, display: &str, file: &str) -> bool {
            self.set += 1;
            // (As the system: a picture set ends a slideshow and
            // switches the wallpaper on.)
            self.kind = Kind::Picture;
            match self.pictures.iter_mut().find(|(known, _)| known == display) {
                Some((_, picture)) => {
                    *picture = file.to_string();
                    true
                }
                None => false,
            }
        }
        fn slideshow(&self) -> Option<Slides> {
            (self.kind == Kind::Slideshow)
                .then(|| self.slides.clone())
                .flatten()
        }
        fn set_slideshow(&mut self, slides: &Slides) -> bool {
            self.kind = Kind::Slideshow;
            self.slides = Some(slides.clone());
            for (_, picture) in &mut self.pictures {
                *picture = format!(r"{}\slide.jpg", slides.place);
            }
            true
        }
        fn set_plain(&mut self) -> bool {
            self.kind = Kind::Plain;
            for (_, picture) in &mut self.pictures {
                picture.clear();
            }
            true
        }
        fn position(&self) -> Option<i32> {
            Some(self.position)
        }
        fn set_position(&mut self, position: i32) -> bool {
            self.position = position;
            true
        }
        fn colour(&self) -> Option<u32> {
            Some(self.colour)
        }
        fn set_colour(&mut self, colour: u32) -> bool {
            self.colour = colour;
            true
        }
    }

    /// A folder of the test's own with pictures in it, and a place for
    /// what is written down.
    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-wallpaper-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        std::fs::create_dir_all(&folder).unwrap();
        for picture in ["a.png", "b.jpg", "c.png", "d.webp"] {
            std::fs::write(folder.join(picture), b"a picture").unwrap();
        }
        folder
    }

    fn names(display: &str) -> Vec<String> {
        vec![
            display.to_string(),
            if display == "one" { "DISPLAY1" } else { "DISPLAY2" }.to_string(),
        ]
    }

    fn config(image: &Path) -> WallpaperConfig {
        WallpaperConfig {
            image: image.display().to_string(),
            fit: WallpaperFit::Fit,
            color: (1, 2, 3),
            ..Default::default()
        }
    }

    #[test]
    fn a_configuration_names_a_wallpaper_only_by_a_picture() {
        let mut config = WallpaperConfig::default();
        assert!(!names_one(&config));
        config.color = (200, 10, 10);
        assert!(!names_one(&config));
        config.outputs.push(Default::default());
        assert!(!names_one(&config));
        config.outputs[0].image = "x.png".to_string();
        assert!(names_one(&config));
        let mut slides = WallpaperConfig::default();
        slides.slideshow.folder = "pictures".to_string();
        assert!(names_one(&slides));
        // The model's fits are the system's positions, one to one.
        let positions: Vec<i32> = [
            WallpaperFit::Centre,
            WallpaperFit::Tile,
            WallpaperFit::Stretch,
            WallpaperFit::Fit,
            WallpaperFit::Fill,
        ]
        .map(position_of)
        .to_vec();
        assert_eq!(positions, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn the_pictures_are_given_and_what_was_there_is_given_back() {
        let folder = folder("given");
        let kept = folder.join("kept.json");
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let mut wallpaper = Wallpaper::kept_in(Some(kept.clone()));
        let wanted = config(&folder.join("a.png"));

        assert!(!wallpaper.follow(&wanted, &mut desktop, &names, 0));
        let given = told(&folder.join("a.png"));
        assert_eq!((desktop.shows("one"), desktop.shows("two")), (given.clone(), given.clone()));
        assert_eq!((desktop.position, desktop.colour), (3, 0x00030201));
        assert!(wallpaper.has_given() && kept.exists());
        // Asked again, nothing is set again.
        let set = desktop.set;
        wallpaper.follow(&wanted, &mut desktop, &names, 5);
        assert_eq!(desktop.set, set);

        // The configuration names none any more: all of it is as it was,
        // and nothing is written down.
        wallpaper.follow(&WallpaperConfig::default(), &mut desktop, &names, 9);
        assert_eq!(
            (desktop.shows("one"), desktop.shows("two")),
            (r"C:\own\first.jpg".to_string(), r"C:\own\second.jpg".to_string())
        );
        assert_eq!((desktop.position, desktop.colour), (5, 0x00112233));
        assert!(!wallpaper.has_given() && !kept.exists());
        // And a configuration that never named one touches nothing.
        let set = desktop.set;
        let mut idle = Wallpaper::kept_in(Some(folder.join("idle.json")));
        assert!(!idle.has_work(&WallpaperConfig::default()));
        idle.follow(&WallpaperConfig::default(), &mut desktop, &names, 0);
        assert_eq!(desktop.set, set);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_background_that_is_no_picture_is_left_and_said() {
        let folder = folder("refused");
        let wanted = config(&folder.join("a.png"));
        // (Spotlight, and a slideshow that is not of one place.)
        for kind in [Kind::Slideshow, Kind::Spotlight] {
            let mut desktop = Fake::with(kind, "", "");
            let mut wallpaper = Wallpaper::kept_in(Some(folder.join("kept.json")));
            assert!(wallpaper.follow(&wanted, &mut desktop, &names, 0));
            assert_eq!(desktop.set, 0);
            assert_eq!((desktop.position, desktop.colour), (5, 0x00112233));
            assert_eq!(
                wallpaper.notice().map(|notice| notice.id),
                Some("wallpaper-not-changed".to_string())
            );
            assert!(!wallpaper.has_given());
            // Said once; and gone when the configuration names none.
            assert!(!wallpaper.follow(&wanted, &mut desktop, &names, 1));
            assert!(wallpaper.follow(&WallpaperConfig::default(), &mut desktop, &names, 2));
            assert!(wallpaper.notice().is_none());
        }
        // A picture as the background: nothing to say.
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("kept.json")));
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        assert!(wallpaper.notice().is_none());
        // The person chooses a colour in Settings afterwards: theirs, and
        // nothing of before is given back over it.
        desktop.set_plain();
        let set = desktop.set;
        assert!(!wallpaper.follow(&wanted, &mut desktop, &names, 3));
        assert!(wallpaper.notice().is_none());
        wallpaper.give_back(&mut desktop);
        // (And it is not taken from them at the next look either.)
        wallpaper.follow(&wanted, &mut desktop, &names, 4);
        assert_eq!((desktop.set, desktop.kind), (set, Kind::Plain));
        assert!(!wallpaper.has_given());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_colour_and_a_slideshow_of_one_folder_are_given_back_as_they_were() {
        let folder = folder("kinds");
        let wanted = config(&folder.join("a.png"));
        let given = told(&folder.join("a.png"));

        // A plain colour: gevrek's picture over it, and the colour again.
        let kept = folder.join("plain.json");
        let mut desktop = Fake::with(Kind::Plain, "", "");
        let mut wallpaper = Wallpaper::kept_in(Some(kept.clone()));
        assert!(!wallpaper.follow(&wanted, &mut desktop, &names, 0));
        assert!(wallpaper.notice().is_none());
        assert_eq!((desktop.shows("one"), desktop.kind), (given.clone(), Kind::Picture));
        // (The system says for a moment that it still shows no picture:
        // gevrek does not take that for the person's choice.)
        desktop.kind = Kind::Plain;
        wallpaper.follow(&wanted, &mut desktop, &names, 1);
        assert!(wallpaper.has_given());
        desktop.kind = Kind::Picture;
        wallpaper.give_back(&mut desktop);
        assert_eq!((desktop.shows("one"), desktop.kind), (String::new(), Kind::Plain));
        assert_eq!((desktop.position, desktop.colour), (5, 0x00112233));
        assert!(!kept.exists());

        // The system's slideshow of one folder: the same, also by a
        // window manager that comes after one that was ended.
        let kept = folder.join("slides.json");
        let slides = Slides {
            place: r"C:\own\pictures".to_string(),
            options: 1,
            tick: 1_800_000,
        };
        let mut desktop = Fake::with(Kind::Slideshow, r"C:\own\pictures\x.jpg", r"C:\own\pictures\y.jpg");
        desktop.slides = Some(slides.clone());
        let mut first = Wallpaper::kept_in(Some(kept.clone()));
        first.follow(&wanted, &mut desktop, &names, 0);
        assert_eq!((desktop.shows("two"), desktop.kind), (given.clone(), Kind::Picture));
        drop(first);
        let mut second = Wallpaper::kept_in(Some(kept.clone()));
        second.follow(&WallpaperConfig::default(), &mut desktop, &names, 1);
        assert_eq!(desktop.kind, Kind::Slideshow);
        assert_eq!(desktop.slides, Some(slides));
        assert_eq!(desktop.shows("one"), r"C:\own\pictures\slide.jpg");

        // The system shows gevrek's picture once more after the colour
        // was given back (it finished setting it late): given back again.
        let mut desktop = Fake::with(Kind::Plain, "", "");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("late.json")));
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        wallpaper.give_back(&mut desktop);
        assert!(wallpaper.holds() && wallpaper.has_work(&WallpaperConfig::default()));
        desktop.set_picture("one", &given);
        wallpaper.follow(&WallpaperConfig::default(), &mut desktop, &names, 1);
        assert_eq!((desktop.shows("one"), desktop.kind), (String::new(), Kind::Plain));
        // (Not a picture that is somebody else's.)
        desktop.set_picture("one", r"C:\other\new.jpg");
        wallpaper.hold(&mut desktop);
        assert_eq!(desktop.kind, Kind::Picture);

        // A picture somebody else set over gevrek's meanwhile stays: the
        // colour of before is not put over it.
        let mut desktop = Fake::with(Kind::Plain, "", "");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("other.json")));
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        desktop.set_picture("one", r"C:\other\new.jpg");
        desktop.set_picture("two", r"C:\other\new.jpg");
        wallpaper.give_back(&mut desktop);
        assert_eq!((desktop.shows("one"), desktop.kind), (r"C:\other\new.jpg".to_string(), Kind::Picture));
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_picture_somebody_else_sets_afterwards_is_theirs() {
        let folder = folder("theirs");
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("kept.json")));
        let wanted = config(&folder.join("a.png"));
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        // Another program gives the second display a picture.
        desktop.set_picture("two", r"C:\other\new.jpg");
        wallpaper.follow(&wanted, &mut desktop, &names, 1);
        wallpaper.follow(&wanted, &mut desktop, &names, 2);
        assert_eq!(desktop.shows("two"), r"C:\other\new.jpg");
        // Until the configuration asks for another picture.
        let other = config(&folder.join("c.png"));
        wallpaper.follow(&other, &mut desktop, &names, 3);
        assert_eq!(desktop.shows("two"), told(&folder.join("c.png")));
        // Given back: the first display's of before, and the other
        // program's on the second.
        wallpaper.give_back(&mut desktop);
        assert_eq!(
            (desktop.shows("one"), desktop.shows("two")),
            (r"C:\own\first.jpg".to_string(), r"C:\other\new.jpg".to_string())
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_display_has_its_own_picture_and_a_slideshow_goes_on() {
        let folder = folder("each");
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("kept.json")));
        let mut wanted = config(&folder.join("a.png"));
        wanted.outputs.push(gevrek_model::wallpaper::OutputWallpaper {
            name: "display2".to_string(),
            image: folder.join("b.jpg").display().to_string(),
            fit: WallpaperFit::Tile,
            ..Default::default()
        });
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        assert_eq!(desktop.shows("one"), told(&folder.join("a.png")));
        assert_eq!(desktop.shows("two"), told(&folder.join("b.jpg")));
        // (One position for all: the common picture's.)
        assert_eq!(desktop.position, 3);

        // A slideshow: the folder's pictures the system shows, by name,
        // one each period; the display with its own keeps it.
        wanted.slideshow.folder = folder.display().to_string();
        wanted.slideshow.every_minutes = 1;
        let mut seen = Vec::new();
        for minute in 0..4 {
            wallpaper.follow(&wanted, &mut desktop, &names, minute * 60);
            seen.push(desktop.shows("one"));
            assert_eq!(desktop.shows("two"), told(&folder.join("b.jpg")));
        }
        let expected: Vec<String> = ["a.png", "b.jpg", "c.png", "a.png"]
            .iter()
            .map(|name| told(&folder.join(name)))
            .collect();
        assert_eq!(seen, expected);

        // A display whose entry names no picture keeps what it has, and
        // so does one whose file is not there or is of no kind for it.
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let mut wallpaper = Wallpaper::kept_in(Some(folder.join("other.json")));
        let mut wanted = config(&folder.join("missing.png"));
        wanted.outputs.push(gevrek_model::wallpaper::OutputWallpaper {
            name: "DISPLAY2".to_string(),
            ..Default::default()
        });
        wallpaper.follow(&wanted, &mut desktop, &names, 0);
        wallpaper.follow(&config(&folder.join("d.webp")), &mut desktop, &names, 1);
        assert_eq!(desktop.set, 0);
        assert!(!wallpaper.has_given());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn what_a_window_manager_left_behind_is_given_back_by_the_next() {
        let folder = folder("left");
        let kept = folder.join("kept.json");
        let mut desktop = Fake::with(Kind::Picture, r"C:\own\first.jpg", r"C:\own\second.jpg");
        let wanted = config(&folder.join("a.png"));
        // One gives the pictures and is ended without giving back.
        let mut first = Wallpaper::kept_in(Some(kept.clone()));
        first.follow(&wanted, &mut desktop, &names, 0);
        drop(first);
        assert!(kept.exists());
        // The next has a configuration without a wallpaper.
        let mut second = Wallpaper::kept_in(Some(kept.clone()));
        assert!(second.has_work(&WallpaperConfig::default()));
        second.follow(&WallpaperConfig::default(), &mut desktop, &names, 1);
        assert_eq!(
            (desktop.shows("one"), desktop.shows("two")),
            (r"C:\own\first.jpg".to_string(), r"C:\own\second.jpg".to_string())
        );
        assert_eq!((desktop.position, desktop.colour), (5, 0x00112233));
        assert!(!kept.exists());
        // And one with the same wallpaper goes on, and gives back the
        // person's own in the end, not gevrek's.
        let mut third = Wallpaper::kept_in(Some(kept.clone()));
        third.follow(&wanted, &mut desktop, &names, 2);
        drop(third);
        let mut fourth = Wallpaper::kept_in(Some(kept.clone()));
        fourth.follow(&wanted, &mut desktop, &names, 3);
        fourth.give_back(&mut desktop);
        assert_eq!(desktop.shows("one"), r"C:\own\first.jpg");
        // One that touches nothing touches nothing.
        let set = desktop.set;
        Wallpaper::nowhere().follow(&wanted, &mut desktop, &names, 4);
        assert_eq!(desktop.set, set);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_system_says_what_its_background_is() {
        // Whatever this machine has: asking changes nothing, every
        // display of the desktop is answered for, and asked twice it is
        // the same. (Where there is no desktop to ask, nothing is.)
        let Some(desktop) = system::Desktop::open() else {
            return;
        };
        let displays = desktop.displays();
        let now: Vec<Option<String>> = displays
            .iter()
            .map(|display| desktop.picture(display))
            .collect();
        println!(
            "the background is {:?}; displays {displays:?} show {now:?}; position {:?}, colour {:?}",
            desktop.kind(),
            desktop.position(),
            desktop.colour()
        );
        assert!(now.iter().all(Option::is_some));
        let again: Vec<Option<String>> = displays
            .iter()
            .map(|display| desktop.picture(display))
            .collect();
        assert_eq!(now, again);
        assert_eq!(desktop.kind(), desktop.kind());
        if desktop.kind() == Kind::Picture {
            assert!(now.iter().flatten().any(|file| !file.is_empty()));
        }
    }

    #[test]
    fn a_display_is_called_by_all_its_names() {
        let known = vec![(
            r"\\.\DISPLAY2".to_string(),
            "Monitor ACM2702 (2)".to_string(),
            r"\\?\DISPLAY#ACM2702#1".to_string(),
        )];
        let names = names_of(r"\\?\display#acm2702#1", &known);
        for name in ["Monitor ACM2702 (2)", "Monitor ACM2702", r"\\.\DISPLAY2", "DISPLAY2"] {
            assert!(names.iter().any(|known| known == name), "{name} in {names:?}");
        }
        assert_eq!(names_of("another", &known), ["another"]);
    }
}
