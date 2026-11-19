//! The icons of applications.
//!
//! An application names its icon (`firefox`, `org.gnome.Nautilus`), or
//! gives a file. A name is looked for in the icon themes of the data
//! directories as the icon-theme specification lays them out, in the
//! theme the desktop uses and then in the ones every theme falls back to;
//! a picture near the wanted size is preferred, a drawing (SVG) fits any.
//!
//! On Windows an application's icon is whatever the shell draws for it:
//! the application names its shortcut, or its place in the shell's folder
//! of applications, and the shell is asked for the picture.
//!
//! [`picture`] is the one question for every system, and [`Cache`] asks
//! it away from the thread that draws and remembers the answers.

use std::path::{Path, PathBuf};

/// The themes looked in after the user's own, in this order: the ones
/// the common desktops ship, then the one every theme inherits.
// (The dark breeze first: its signs are light, and gevrek's panel is
// dark unless it is made otherwise; its applications' pictures are the
// same as the light one's.)
const FALLBACK_THEMES: [&str; 6] = [
    "breeze-dark",
    "breeze",
    "Adwaita",
    "Papirus",
    "gnome",
    "hicolor",
];

/// The directories icon themes are in, the user's first.
pub fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".icons"));
    }
    for data in crate::apps::data_dirs() {
        dirs.push(data.join("icons"));
    }
    dirs
}

/// The file of the icon `name` at about `size` pixels, in `theme` (when
/// one is named) and the fallback themes under `dirs`, then among the
/// loose pictures (`pixmaps`). A name that is a path is that file.
pub fn find_in(
    dirs: &[PathBuf],
    pixmaps: &[PathBuf],
    theme: Option<&str>,
    name: &str,
    size: u32,
) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    // Some entries name the file with its ending.
    let name = name
        .strip_suffix(".png")
        .or_else(|| name.strip_suffix(".svg"))
        .unwrap_or(name);
    let themes = theme.into_iter().chain(FALLBACK_THEMES);
    for theme in themes {
        let mut best: Option<(u32, PathBuf)> = None;
        for dir in dirs {
            let root = dir.join(theme);
            let Ok(sizes) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in sizes.flatten() {
                let folder = entry.file_name().to_string_lossy().into_owned();
                if !entry.path().is_dir() {
                    continue;
                }
                // A theme keeps its pictures two folders down, one for the
                // size and one for the kind, in either order: `48x48/apps`,
                // `scalable/apps`, `symbolic/status`, or (breeze)
                // `apps/48`, `status/16`. Every kind is looked in: an
                // application's own picture is under `apps`, but what a
                // program shows in the tray may be of any kind (Solaar
                // names a battery, which is a `status`).
                let outer_px = folder_size(&folder);
                let mut places: Vec<(Option<u32>, bool, PathBuf)> =
                    vec![(outer_px, folder == "apps", entry.path())];
                for inner in std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if !inner.path().is_dir() {
                        continue;
                    }
                    let inner_name = inner.file_name().to_string_lossy().into_owned();
                    places.push((
                        outer_px.or_else(|| folder_size(&inner_name)),
                        folder == "apps" || inner_name == "apps",
                        inner.path(),
                    ));
                }
                for (px, of_applications, place) in places {
                    for (ending, drawing) in [("svg", true), ("png", false)] {
                        let file = place.join(format!("{name}.{ending}"));
                        if !file.is_file() {
                            continue;
                        }
                        // How far from what is wanted: a drawing fits
                        // any size; a picture is better the nearer it
                        // is, and too large is better than too small.
                        // Of two that are as good, an application's own
                        // picture comes first.
                        let off = match (drawing, px) {
                            (true, _) => 1,
                            (false, Some(px)) if px >= size => px - size + 2,
                            (false, Some(px)) => (size - px) * 4,
                            (false, None) => 1000,
                        } * 2
                            + u32::from(!of_applications);
                        if best.as_ref().is_none_or(|(known, _)| off < *known) {
                            best = Some((off, file));
                        }
                    }
                }
            }
        }
        if let Some((_, file)) = best {
            return Some(file);
        }
    }
    for dir in pixmaps {
        for ending in ["png", "svg", "xpm"] {
            let file = dir.join(format!("{name}.{ending}"));
            if file.is_file() && ending != "xpm" {
                return Some(file);
            }
        }
    }
    None
}

/// The pixels a folder of a theme is for: `48x48` and `48x48@2` are 48
/// and 96, `64` is 64; `scalable` and the like are none.
fn folder_size(folder: &str) -> Option<u32> {
    let (size, times) = match folder.split_once('@') {
        Some((size, times)) => (size, times.trim_end_matches('x').parse::<u32>().ok()?),
        None => (folder, 1),
    };
    let first = size.split('x').next()?;
    first.parse::<u32>().ok().map(|px| px * times)
}

/// The icon theme the desktop uses, where it says (GTK's settings file).
pub fn desktop_theme() -> Option<String> {
    let config = dirs::config_dir()?;
    for file in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        let Ok(text) = std::fs::read_to_string(config.join(file)) else {
            continue;
        };
        if let Some(theme) = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("gtk-icon-theme-name"))
        {
            let theme = theme.trim_start_matches([' ', '=']).trim();
            if !theme.is_empty() {
                return Some(theme.to_string());
            }
        }
    }
    None
}

/// The file of an application's icon on this system.
pub fn find(name: &str, size: u32) -> Option<PathBuf> {
    let theme = desktop_theme();
    let pixmaps: Vec<PathBuf> = crate::apps::data_dirs()
        .into_iter()
        .map(|data| data.join("pixmaps"))
        .collect();
    find_in(&theme_dirs(), &pixmaps, theme.as_deref(), name, size)
}

/// A picture ready to be shown: its size and its pixels, four bytes each
/// (red, green, blue, how solid), not multiplied.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Read an icon's file as a picture of `size` by `size` pixels.
pub fn load(path: &Path, size: u32) -> Option<Picture> {
    let size = size.clamp(8, 512);
    let is_drawing = path
        .extension()
        .and_then(|ending| ending.to_str())
        .is_some_and(|ending| ending.eq_ignore_ascii_case("svg"));
    if is_drawing {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
        let drawn = tree.size();
        let scale = (size as f32 / drawn.width()).min(size as f32 / drawn.height());
        // In the middle, when it is not square.
        let shift_x = (size as f32 - drawn.width() * scale) / 2.0;
        let shift_y = (size as f32 - drawn.height() * scale) / 2.0;
        let transform =
            resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(shift_x, shift_y);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        // The drawing library multiplies colour by solidness; undo that.
        let pixels = pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let colour = pixel.demultiply();
                [colour.red(), colour.green(), colour.blue(), colour.alpha()]
            })
            .collect();
        return Some(Picture {
            width: size,
            height: size,
            pixels,
        });
    }
    let image = image::open(path)
        .ok()?
        .resize(size, size, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(Picture {
        width: image.width(),
        height: image.height(),
        pixels: image.into_raw(),
    })
}

/// The icon an application names, as a picture of about `size` pixels.
/// It reads files, and on Windows asks the shell: not for the thread
/// that draws (see [`Cache`]).
pub fn picture(name: &str, size: u32) -> Option<Picture> {
    if name.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        shell::picture(name, size)
    }
    #[cfg(not(windows))]
    {
        load(&find(name, size)?, size)
    }
}

/// The installed application a window of this name belongs to (what the
/// program calls itself, a Wayland application id): the one whose entry
/// is named so, or ends in it (`org.mozilla.firefox` for `firefox`), or
/// starts a program of that name.
pub fn app_of<'a>(apps: &'a [crate::apps::App], app_id: &str) -> Option<&'a crate::apps::App> {
    if app_id.is_empty() {
        return None;
    }
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    apps.iter()
        .find(|app| same(&app.id, app_id))
        .or_else(|| {
            apps.iter().find(|app| {
                app.id
                    .rsplit('.')
                    .next()
                    .is_some_and(|last| same(last, app_id))
            })
        })
        .or_else(|| {
            apps.iter().find(|app| {
                app.command
                    .first()
                    .and_then(|program| std::path::Path::new(program).file_name())
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| same(name, app_id))
            })
        })
}

/// The picture of the program a window belongs to, by what the program
/// calls itself: an icon of that name, else the icon its application's
/// entry names (which is often another: the studio's is `gevrek`). It
/// reads files: not for the thread that draws.
#[cfg(not(windows))]
pub fn picture_of_app(app_id: &str, size: u32) -> Option<Picture> {
    picture(app_id, size).or_else(|| {
        let apps = crate::apps::installed();
        let icon = &app_of(&apps, app_id)?.icon;
        if icon.starts_with('/') {
            load(std::path::Path::new(icon), size)
        } else {
            picture(icon, size)
        }
    })
}

/// Pixels as Windows keeps them (blue, green, red, how solid, the colours
/// multiplied by the solidness) as the pixels of a [`Picture`]. An old
/// icon says nothing of solidness anywhere: it is solid everywhere.
pub fn from_windows_pixels(pixels: &[u8]) -> Vec<u8> {
    let (pixels, _) = pixels.as_chunks::<4>();
    let flat = pixels.iter().all(|pixel| pixel[3] == 0);
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for &[blue, green, red, solid] in pixels {
        if flat {
            out.extend([red, green, blue, 255]);
        } else if solid == 0 {
            out.extend([0, 0, 0, 0]);
        } else {
            let plain = |colour: u8| {
                ((u32::from(colour) * 255 + u32::from(solid) / 2) / u32::from(solid)).min(255) as u8
            };
            out.extend([plain(red), plain(green), plain(blue), solid]);
        }
    }
    out
}

#[cfg(windows)]
pub(crate) mod shell {
    use super::{from_windows_pixels, Picture};
    use crate::com::Com;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_ICONONLY,
        SIIGBF_THUMBNAILONLY,
    };

    /// The icon the shell draws for what `name` names to it: a file (a
    /// shortcut among them), or a place such as `shell:AppsFolder\…`.
    pub fn picture(name: &str, size: u32) -> Option<Picture> {
        picture_as(name, size, false)
    }

    /// As `picture`; or, where `thumbnail`, the small picture of what a
    /// file holds (a photograph, a page), as large as fits into `size`
    /// each way. There is none for most files, and then the answer is
    /// none: the shell is not let fall back to the icon, so that who asks
    /// knows which of the two it got.
    pub fn picture_as(name: &str, size: u32, thumbnail: bool) -> Option<Picture> {
        let what = if thumbnail {
            SIIGBF_THUMBNAILONLY
        } else {
            SIIGBF_ICONONLY
        };
        let _com = Com::new();
        // SAFETY: the shell's objects are used on this thread alone,
        // which may use them while `_com` lives.
        unsafe {
            let item: IShellItemImageFactory =
                SHCreateItemFromParsingName(&HSTRING::from(name), None).ok()?;
            picture_from(&item, size, what)
        }
    }

    /// The picture one of the shell's items gives of itself, asked for in
    /// the way `what` says (the icon only, the small picture only, only
    /// what is in Windows' own store of small pictures).
    ///
    /// # Safety
    /// Called on a thread that is set up for the shell's objects, with an
    /// item that was made on it.
    pub unsafe fn picture_from(item: &IShellItemImageFactory, size: u32, what: SIIGBF) -> Option<Picture> {
        let size = size.clamp(8, 512) as i32;
        {
            let bitmap = item
                .GetImage(SIZE { cx: size, cy: size }, what)
                .ok()?;
            let mut read = None;
            let mut what = BITMAP::default();
            let told = GetObjectW(
                bitmap,
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut what as *mut BITMAP as *mut _),
            );
            if told != 0 && what.bmWidth > 0 && what.bmHeight > 0 {
                let (width, height) = (what.bmWidth, what.bmHeight);
                let mut asked = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width,
                        // The first row first.
                        biHeight: -height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut pixels = vec![0u8; width as usize * height as usize * 4];
                let device = CreateCompatibleDC(None);
                let rows = GetDIBits(
                    device,
                    bitmap,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr() as *mut _),
                    &mut asked,
                    DIB_RGB_COLORS,
                );
                let _ = DeleteDC(device);
                if rows == height {
                    read = Some(Picture {
                        width: width as u32,
                        height: height as u32,
                        pixels: from_windows_pixels(&pixels),
                    });
                }
            }
            let _ = DeleteObject(bitmap);
            read
        }
    }
}

/// What a [`Cache`] asks for: an icon's name and its size.
type Wanted = (String, u32);

/// The icons that were asked for, and those being looked for.
///
/// Looking for an icon reads files or asks the shell, which can take
/// long; a surface is drawn many times a second. So an icon is looked
/// for once, on a thread of the cache's own, and until it is there the
/// one who draws is told there is none yet and draws something else.
pub struct Cache {
    known: std::collections::HashMap<Wanted, Option<Picture>>,
    asked: std::collections::HashSet<Wanted>,
    ask: std::sync::mpsc::Sender<Wanted>,
    answers: std::sync::mpsc::Receiver<(Wanted, Option<Picture>)>,
}

impl Cache {
    /// `wake` is called, on the cache's thread, each time an icon was
    /// looked for: the moment to draw again.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        Self::with(picture, wake)
    }

    /// A cache that looks for its icons with `look`.
    pub fn with(
        look: impl Fn(&str, u32) -> Option<Picture> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (ask, wanted) = std::sync::mpsc::channel::<Wanted>();
        let (answer, answers) = std::sync::mpsc::channel();
        // The thread ends with the cache: nobody asks any more.
        let started = std::thread::Builder::new()
            .name("gevrek-icons".to_string())
            .spawn(move || {
                for (name, size) in wanted {
                    let found = look(&name, size);
                    if answer.send(((name, size), found)).is_err() {
                        break;
                    }
                    wake();
                }
            });
        if let Err(error) = started {
            log::warn!("no icons: the thread that looks for them did not start: {error}");
        }
        Self {
            known: Default::default(),
            asked: Default::default(),
            ask,
            answers,
        }
    }

    /// The icon `name` at `size`, when it was found. `None` also means
    /// that it is being looked for: asked again after the cache woke its
    /// user, the answer is there.
    pub fn get(&mut self, name: &str, size: u32) -> Option<&Picture> {
        for (wanted, found) in self.answers.try_iter() {
            self.known.insert(wanted, found);
        }
        let wanted = (name.to_string(), size);
        if !name.is_empty()
            && !self.known.contains_key(&wanted)
            && self.asked.insert(wanted.clone())
        {
            let _ = self.ask.send(wanted.clone());
        }
        self.known.get(&wanted).and_then(Option::as_ref)
    }

    /// As `get`, for one who must tell "not looked for yet" (`None`)
    /// from "looked for, and there is none" (`Some(None)`).
    pub fn answer(&mut self, name: &str, size: u32) -> Option<Option<&Picture>> {
        let _ = self.get(name, size);
        self.known
            .get(&(name.to_string(), size))
            .map(Option::as_ref)
    }

    /// Let go of the icon `name` at every size: what it is an icon of
    /// has changed, or nobody shows it any more. Asked for again, it is
    /// looked for again.
    pub fn forget(&mut self, name: &str) {
        self.known.retain(|(known, _), _| known != name);
        self.asked.retain(|(asked, _)| asked != name);
    }

    /// Let go of the icon `name` at one size only: its pixels are not
    /// kept any longer (there is a bound on how much is). Asked for
    /// again, it is looked for again.
    pub fn forget_at(&mut self, name: &str, size: u32) {
        let wanted = (name.to_string(), size);
        self.known.remove(&wanted);
        self.asked.remove(&wanted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_program_is_found_among_the_applications() {
        let app = |id: &str, program: &str| crate::apps::App {
            id: id.to_string(),
            name: id.to_string(),
            generic_name: String::new(),
            comment: String::new(),
            keywords: Vec::new(),
            icon: format!("icon-of-{id}"),
            command: vec![program.to_string()],
            terminal: false,
            dbus_activatable: false,
            directory: None,
            entry: std::path::PathBuf::new(),
            actions: Vec::new(),
        };
        let apps = [
            app("gevrek-studio", "gevrek-studio"),
            app("org.mozilla.firefox", "/usr/bin/firefox-bin"),
            app("terminal-of-the-desktop", "/usr/bin/foot"),
        ];
        let icon = |app_id: &str| app_of(&apps, app_id).map(|app| app.icon.as_str());
        // By its entry's name, by the end of it, by its program.
        assert_eq!(icon("gevrek-studio"), Some("icon-of-gevrek-studio"));
        assert_eq!(icon("Firefox"), Some("icon-of-org.mozilla.firefox"));
        assert_eq!(icon("foot"), Some("icon-of-terminal-of-the-desktop"));
        assert_eq!(icon("unknown"), None);
        assert_eq!(icon(""), None);
    }

    #[test]
    fn windows_pixels_become_plain_ones() {
        // Half solid red, as Windows keeps it: the colour multiplied.
        let half_red = [0u8, 0, 128, 128];
        let clear = [0u8, 0, 0, 0];
        let white = [255u8, 255, 255, 255];
        let pixels = [half_red, clear, white].concat();
        assert_eq!(
            from_windows_pixels(&pixels),
            [255, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]
        );
        // An icon without solidness is solid: blue, green, red in, red,
        // green, blue out.
        assert_eq!(
            from_windows_pixels(&[10, 20, 30, 0, 40, 50, 60, 0]),
            [30, 20, 10, 255, 60, 50, 40, 255]
        );
    }

    #[test]
    fn an_icon_of_any_kind_is_found_and_an_applications_own_comes_first() {
        let root = std::env::temp_dir().join(format!("gevrek-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let make = |path: &str| {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("folders");
            std::fs::write(&file, b"<svg/>").expect("a file");
            file
        };
        // What Solaar names in the tray once it knows a device's battery:
        // a sign of the kind `status`, in breeze's way of folders.
        let battery = make("breeze/status/16/battery-060.svg");
        // The other way round, as hicolor has it.
        let bell = make("hicolor/24x24/status/bell.svg");
        // One name in two kinds: the application's own is taken.
        make("hicolor/scalable/mimetypes/editor.svg");
        let editor = make("hicolor/scalable/apps/editor.svg");
        let dirs = [root.clone()];
        assert_eq!(find_in(&dirs, &[], None, "battery-060", 20), Some(battery));
        assert_eq!(find_in(&dirs, &[], None, "bell", 20), Some(bell));
        assert_eq!(find_in(&dirs, &[], None, "editor", 20), Some(editor));
        assert_eq!(find_in(&dirs, &[], None, "nothing-of-that-name", 20), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_icon_is_looked_for_once_and_not_by_the_one_who_asks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let looked = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&looked);
        let asker = std::thread::current().id();
        let (woken, wakes) = std::sync::mpsc::channel();
        let mut cache = Cache::with(
            move |name, size| {
                assert_ne!(std::thread::current().id(), asker);
                counted.fetch_add(1, Ordering::SeqCst);
                (name == "there").then(|| Picture {
                    width: size,
                    height: size,
                    pixels: vec![7; (size * size * 4) as usize],
                })
            },
            move || {
                let _ = woken.send(());
            },
        );
        let wait = || {
            wakes
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("woken")
        };
        // Asked for however often, it is looked for once.
        let _ = cache.get("there", 2);
        let _ = cache.get("there", 2);
        let _ = cache.get("missing", 2);
        // No name: nothing to look for.
        assert!(cache.get("", 2).is_none());
        wait();
        wait();
        let found = cache.get("there", 2).expect("found by now").clone();
        assert_eq!((found.width, found.pixels.len()), (2, 16));
        assert!(cache.get("missing", 2).is_none());
        assert!(cache.get("missing", 2).is_none());
        // Another size is another picture.
        assert!(cache.get("there", 4).is_none());
        wait();
        assert_eq!(cache.get("there", 4).expect("found").width, 4);
        assert_eq!(looked.load(Ordering::SeqCst), 3);
        // One that was looked for and is none is told from one not yet
        // looked for; forgotten, an icon is looked for again.
        assert!(matches!(cache.answer("missing", 2), Some(None)));
        assert!(matches!(cache.answer("there", 2), Some(Some(_))));
        cache.forget("there");
        assert!(cache.answer("there", 2).is_none());
        wait();
        assert!(matches!(cache.answer("there", 2), Some(Some(_))));
        assert_eq!(looked.load(Ordering::SeqCst), 4);
    }

    fn tree() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "gevrek-icons-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("made");
        std::fs::write(path, b"x").expect("written");
    }

    #[test]
    fn an_icon_is_found_at_the_nearest_size_and_a_drawing_fits_any() {
        let base = tree();
        let icons = base.join("icons");
        touch(&icons.join("hicolor/32x32/apps/editor.png"));
        touch(&icons.join("hicolor/64x64/apps/editor.png"));
        touch(&icons.join("hicolor/256x256/apps/editor.png"));
        touch(&icons.join("hicolor/scalable/apps/drawn.svg"));
        touch(&icons.join("hicolor/16x16/apps/drawn.png"));
        let dirs = vec![icons.clone()];
        let found = |name: &str, size: u32| find_in(&dirs, &[], None, name, size);
        assert_eq!(
            found("editor", 48),
            Some(icons.join("hicolor/64x64/apps/editor.png"))
        );
        assert_eq!(
            found("editor", 200),
            Some(icons.join("hicolor/256x256/apps/editor.png"))
        );
        assert_eq!(
            found("editor", 24),
            Some(icons.join("hicolor/32x32/apps/editor.png"))
        );
        assert_eq!(
            found("drawn", 48),
            Some(icons.join("hicolor/scalable/apps/drawn.svg"))
        );
        assert_eq!(
            found("editor.png", 48),
            Some(icons.join("hicolor/64x64/apps/editor.png"))
        );
        assert_eq!(found("missing", 48), None);
        assert_eq!(found("", 48), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_desktops_theme_comes_before_the_fallbacks_and_loose_pictures_last() {
        let base = tree();
        let icons = base.join("icons");
        touch(&icons.join("hicolor/48x48/apps/mail.png"));
        touch(&icons.join("Fancy/48x48/apps/mail.png"));
        // The layout some themes use: the kind first, then the size.
        touch(&icons.join("breeze/apps/48/files.svg"));
        let pixmaps = base.join("pixmaps");
        touch(&pixmaps.join("oldtool.png"));
        let dirs = vec![icons.clone()];
        let loose = vec![pixmaps.clone()];
        assert_eq!(
            find_in(&dirs, &loose, Some("Fancy"), "mail", 48),
            Some(icons.join("Fancy/48x48/apps/mail.png"))
        );
        assert_eq!(
            find_in(&dirs, &loose, None, "mail", 48),
            Some(icons.join("hicolor/48x48/apps/mail.png"))
        );
        assert_eq!(
            find_in(&dirs, &loose, None, "files", 48),
            Some(icons.join("breeze/apps/48/files.svg"))
        );
        assert_eq!(
            find_in(&dirs, &loose, None, "oldtool", 48),
            Some(pixmaps.join("oldtool.png"))
        );
        // A path is that file, when it is there.
        let direct = pixmaps.join("oldtool.png");
        assert_eq!(
            find_in(&dirs, &loose, None, &direct.to_string_lossy(), 48),
            Some(direct)
        );
        assert_eq!(find_in(&dirs, &loose, None, "/no/such/icon.png", 48), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folders_name_says_its_size() {
        assert_eq!(folder_size("48x48"), Some(48));
        assert_eq!(folder_size("48x48@2"), Some(96));
        assert_eq!(folder_size("64"), Some(64));
        assert_eq!(folder_size("scalable"), None);
    }

    #[test]
    fn a_drawing_and_a_picture_are_read_at_the_size_asked() {
        let base = tree();
        std::fs::create_dir_all(&base).expect("made");
        let drawing = base.join("dot.svg");
        std::fs::write(&drawing, r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#ff0000"/></svg>"##)
            .expect("written");
        let picture = load(&drawing, 32).expect("a picture");
        assert_eq!((picture.width, picture.height), (32, 32));
        assert_eq!(&picture.pixels[..4], &[255, 0, 0, 255]);
        let png = base.join("dot.png");
        image::save_buffer(
            &png,
            &[0u8, 255, 0, 255].repeat(64 * 64),
            64,
            64,
            image::ExtendedColorType::Rgba8,
        )
        .expect("saved");
        let picture = load(&png, 24).expect("a picture");
        assert_eq!((picture.width, picture.height), (24, 24));
        assert_eq!(&picture.pixels[..4], &[0, 255, 0, 255]);
        assert!(load(&base.join("none.png"), 24).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
