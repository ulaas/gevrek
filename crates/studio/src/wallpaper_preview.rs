//! The wallpaper as it will look: each display drawn small, with the
//! picture the configuration gives it put on it the way its fit says
//! (covering it, whole with the colour beside it, pulled to its shape, at
//! its own size in the middle, or again and again), or the colour alone.
//!
//! A path and the word `fill` say little; the picture on the display says
//! whether its middle is the part that matters, and whether a picture of
//! another shape leaves bars. The picture is read away from the drawing
//! (a large one takes a tenth of a second), once for each file, and made
//! small: the preview needs no more.

use egui::{Color32, Rect, Ui};
use gevrek_model::wallpaper::{WallpaperConfig, WallpaperFit};
use gevrek_ui::theme;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// A picture that was read: what it is drawn from, and how large the
/// file's own picture is (the preview's is smaller).
struct Read {
    texture: egui::TextureHandle,
    size: egui::Vec2,
}

enum Picture {
    Reading(mpsc::Receiver<Result<(egui::ColorImage, egui::Vec2), String>>),
    Ready(Read),
    /// It could not be read: why.
    Failed(String),
}

/// A display as the preview draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub name: String,
    /// Its size in pixels.
    pub size: egui::Vec2,
}

/// The displays of the window manager's state; one of the usual shape
/// where there is no window manager to ask.
pub fn displays(state: Option<&Value>) -> Vec<Display> {
    let found: Vec<Display> = state
        .and_then(|state| state["monitors"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            let side = |name: &str| monitor["size"][name].as_f64().map(|side| side as f32);
            let (width, height) = (side("width")?, side("height")?);
            (width >= 1.0 && height >= 1.0).then(|| Display {
                name: monitor["connector"]
                    .as_str()
                    .filter(|connector| !connector.is_empty())
                    .or(monitor["name"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                size: egui::vec2(width, height),
            })
        })
        .collect();
    if found.is_empty() {
        vec![Display {
            name: String::new(),
            size: egui::vec2(1920.0, 1080.0),
        }]
    } else {
        found
    }
}

/// Where a picture goes on a display for a fit: the pieces to draw, each
/// a place on the display's drawing and the part of the picture (0 to 1
/// each way) that is shown there. `picture`: the picture's own size;
/// `scale`: how much smaller the drawing is than the display.
pub fn pieces(fit: WallpaperFit, picture: egui::Vec2, screen: Rect, scale: f32) -> Vec<(Rect, Rect)> {
    let whole = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if picture.x < 1.0 || picture.y < 1.0 || screen.width() < 1.0 || screen.height() < 1.0 {
        return Vec::new();
    }
    // A picture of a size on the display, cut to the display: its place,
    // and the part of it that is inside.
    let cut = |place: Rect| -> Option<(Rect, Rect)> {
        let inside = place.intersect(screen);
        if inside.width() <= 0.0 || inside.height() <= 0.0 {
            return None;
        }
        let part = Rect::from_min_max(
            egui::pos2(
                (inside.left() - place.left()) / place.width(),
                (inside.top() - place.top()) / place.height(),
            ),
            egui::pos2(
                (inside.right() - place.left()) / place.width(),
                (inside.bottom() - place.top()) / place.height(),
            ),
        );
        Some((inside, part))
    };
    match fit {
        WallpaperFit::Stretch => vec![(screen, whole)],
        WallpaperFit::Fill | WallpaperFit::Fit => {
            let (across, down) = (screen.width() / picture.x, screen.height() / picture.y);
            let by = if fit == WallpaperFit::Fill {
                across.max(down)
            } else {
                across.min(down)
            };
            cut(Rect::from_center_size(screen.center(), picture * by))
                .into_iter()
                .collect()
        }
        WallpaperFit::Centre => cut(Rect::from_center_size(screen.center(), picture * scale))
            .into_iter()
            .collect(),
        WallpaperFit::Tile => {
            let tile = picture * scale;
            // (Never so many that a small picture on a large display
            // costs a frame: beyond that the pattern is plain anyway.)
            let (across, down) = (
                ((screen.width() / tile.x).ceil() as usize).clamp(1, 40),
                ((screen.height() / tile.y).ceil() as usize).clamp(1, 40),
            );
            let mut all = Vec::new();
            for row in 0..down {
                for column in 0..across {
                    let place = Rect::from_min_size(
                        screen.min + egui::vec2(column as f32 * tile.x, row as f32 * tile.y),
                        tile,
                    );
                    all.extend(cut(place));
                }
            }
            all
        }
    }
}

/// The first picture of a slideshow's folder, by name.
fn first_slide(folder: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
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
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// What a display shows by the configuration: a file and its fit, or the
/// colour alone; and whether the file is the first of a slideshow.
pub fn shown_on(config: &WallpaperConfig, display: &str) -> (Option<PathBuf>, WallpaperFit, bool) {
    let (image, fit) = config.of(display);
    if config.slides_on(display) {
        return (first_slide(&config.slideshow.folder_path()), fit, true);
    }
    (image, fit, false)
}

/// What the system itself shows as its background (Windows draws the
/// wallpaper there): its kind and the pictures of its displays.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemShows {
    pub kind: String,
    pub pictures: Vec<String>,
}

/// What `gevrek wallpaper --json` says the system shows: its `system`
/// part, each picture once.
pub fn system_shows_in(told: &Value) -> Option<SystemShows> {
    let system = told.get("system").filter(|system| system.is_object())?;
    let mut pictures: Vec<String> = system["displays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|display| display["picture"].as_str())
        .filter(|picture| !picture.is_empty())
        .map(str::to_string)
        .collect();
    pictures.dedup();
    Some(SystemShows {
        kind: system["kind"].as_str().unwrap_or_default().to_string(),
        pictures,
    })
}

/// The words for what is known of a picture's file: how large the
/// picture is, how large the file, and how it is put on the display.
pub fn about_picture(size: Option<egui::Vec2>, bytes: Option<u64>, fit: WallpaperFit) -> String {
    let mut parts = Vec::new();
    if let Some(size) = size {
        parts.push(format!("{} × {} px", size.x as u32, size.y as u32));
    }
    if let Some(bytes) = bytes {
        parts.push(gevrek_ui::browser::size_said(bytes));
    }
    parts.push(format!("{fit:?}").to_lowercase());
    parts.join("  ·  ")
}

/// The preview, and the pictures it has read.
#[derive(Default)]
pub struct WallpaperPreview {
    pictures: HashMap<PathBuf, Picture>,
    /// What the system shows, as it was last told, when it was asked,
    /// and the answer that is on its way.
    system: Option<SystemShows>,
    system_asked: Option<std::time::Instant>,
    system_told: Option<mpsc::Receiver<Option<SystemShows>>>,
}

impl WallpaperPreview {
    /// The picture of a file, once it has been read; reading begins the
    /// first time it is asked for.
    fn picture(&mut self, context: &egui::Context, file: &Path) -> &Picture {
        if !self.pictures.contains_key(file) {
            let (tell, told) = mpsc::channel();
            let (path, wake) = (file.to_path_buf(), context.clone());
            let started = std::thread::Builder::new()
                .name("gevrek-studio-wallpaper".to_string())
                .spawn(move || {
                    let read = image::open(&path)
                        .map(|picture| {
                            let size = egui::vec2(picture.width() as f32, picture.height() as f32);
                            let small = picture.thumbnail(640, 640).to_rgba8();
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [small.width() as usize, small.height() as usize],
                                small.as_raw(),
                            );
                            (image, size)
                        })
                        .map_err(|error| error.to_string());
                    if let Err(error) = &read {
                        log::debug!("the wallpaper {} is not shown: {error}", path.display());
                    }
                    let _ = tell.send(read);
                    wake.request_repaint();
                });
            self.pictures.insert(
                file.to_path_buf(),
                match started {
                    Ok(_) => Picture::Reading(told),
                    Err(error) => Picture::Failed(error.to_string()),
                },
            );
        }
        let arrived = match self.pictures.get(file) {
            Some(Picture::Reading(told)) => told.try_recv().ok(),
            _ => None,
        };
        if let Some(read) = arrived {
            let now = match read {
                Ok((image, size)) => Picture::Ready(Read {
                    texture: context.load_texture(
                        format!("wallpaper-{}", file.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                    size,
                }),
                Err(error) => Picture::Failed(error),
            };
            self.pictures.insert(file.to_path_buf(), now);
        }
        &self.pictures[file]
    }

    /// What the system shows as its own background, asked of the window
    /// manager through gevrek's command beside this program, away from
    /// the drawing, every few seconds while the page is looked at. Only
    /// where the system draws the wallpaper (Windows).
    fn system_shows(&mut self, context: &egui::Context) -> Option<&SystemShows> {
        if !cfg!(windows) {
            return None;
        }
        if let Some(told) = &self.system_told {
            if let Ok(answer) = told.try_recv() {
                self.system = answer;
                self.system_told = None;
            }
        }
        let due = self
            .system_asked
            .is_none_or(|asked| asked.elapsed() > std::time::Duration::from_secs(5));
        if due && self.system_told.is_none() {
            self.system_asked = Some(std::time::Instant::now());
            let (tell, told) = mpsc::channel();
            let wake = context.clone();
            let started = std::thread::Builder::new()
                .name("gevrek-studio-system-wallpaper".to_string())
                .spawn(move || {
                    let program = std::env::current_exe().ok().and_then(|own| {
                        Some(own.parent()?.join(format!("gevrek{}", std::env::consts::EXE_SUFFIX)))
                    });
                    let answer = program
                        .and_then(|program| {
                            let mut command = std::process::Command::new(program);
                            command.args(["wallpaper", "--json"]);
                            #[cfg(windows)]
                            {
                                use std::os::windows::process::CommandExt;
                                // (No console window comes up for it.)
                                command.creation_flags(0x0800_0000);
                            }
                            command.output().ok()
                        })
                        .filter(|output| output.status.success())
                        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
                        .and_then(|told| system_shows_in(&told));
                    let _ = tell.send(answer);
                    wake.request_repaint();
                });
            if started.is_ok() {
                self.system_told = Some(told);
            }
        }
        self.system.as_ref()
    }

    /// Draw every display with its wallpaper.
    pub fn show(&mut self, ui: &mut Ui, config: &WallpaperConfig, state: Option<&Value>) {
        let palette = theme::pal(ui.ctx());
        let context = ui.ctx().clone();
        let displays = displays(state);
        let (red, green, blue) = config.color;
        let colour = Color32::from_rgb(red, green, blue);
        // As wide as fits beside each other, within reason.
        let wide = ((ui.available_width() - 16.0 * (displays.len() as f32 - 1.0))
            / displays.len() as f32)
            .clamp(160.0, 420.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 10.0);
            for display in &displays {
                let scale = wide / display.size.x;
                let high = display.size.y * scale;
                ui.vertical(|ui| {
                    let (screen, _) =
                        ui.allocate_exact_size(egui::vec2(wide, high), egui::Sense::hover());
                    let painter = ui.painter().with_clip_rect(screen);
                    painter.rect_filled(screen, 6.0, colour);
                    let (file, fit, slides) = shown_on(config, &display.name);
                    let mut said = match (&file, slides) {
                        (None, true) => "A slideshow whose folder has no picture".to_string(),
                        (None, false) => "The colour alone".to_string(),
                        (Some(file), _) => file
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    };
                    // (The picture's own size, once it is read.)
                    let mut own_size = None;
                    if let Some(file) = &file {
                        match self.picture(&context, file) {
                            Picture::Ready(read) => {
                                own_size = Some(read.size);
                                for (place, part) in pieces(fit, read.size, screen, scale) {
                                    painter.image(read.texture.id(), place, part, Color32::WHITE);
                                }
                                if slides {
                                    said = format!("A slideshow; first: {said}");
                                }
                            }
                            Picture::Reading(_) => said = format!("Reading {said}…"),
                            Picture::Failed(why) => {
                                said = format!("{said} cannot be shown: {why}");
                            }
                        }
                    }
                    ui.painter().rect_stroke(
                        screen,
                        6.0,
                        egui::Stroke::new(1.0, palette.border_strong()),
                        egui::StrokeKind::Inside,
                    );
                    if !display.name.is_empty() && displays.len() > 1 {
                        let name = gevrek_ui::controls::one_line(
                            ui,
                            &display.name,
                            egui::FontId::proportional(12.0),
                            palette.muted,
                            wide,
                        );
                        ui.label(egui::WidgetText::from(name));
                    }
                    let words = gevrek_ui::controls::one_line(
                        ui,
                        &said,
                        egui::FontId::proportional(12.0),
                        palette.faint,
                        wide,
                    );
                    ui.label(egui::WidgetText::from(words));
                    // The picture in use: where it is, and what is known
                    // of it. (The whole path comes up over it, and a
                    // press copies it.)
                    if let Some(file) = &file {
                        let whole = file.display().to_string();
                        let path = gevrek_ui::controls::one_line(
                            ui,
                            &whole,
                            egui::FontId::monospace(11.5),
                            palette.muted,
                            wide,
                        );
                        let shown = ui
                            .add(egui::Label::new(egui::WidgetText::from(path)).sense(egui::Sense::click()))
                            .on_hover_text(format!("{whole}\nA press copies the path."));
                        if shown.clicked() {
                            ui.ctx().copy_text(whole.clone());
                        }
                        let bytes = std::fs::metadata(file).ok().map(|about| about.len());
                        let about = if bytes.is_none() {
                            "The file is not there on this machine.".to_string()
                        } else {
                            about_picture(own_size, bytes, fit)
                        };
                        ui.label(
                            egui::RichText::new(about)
                                .size(11.5)
                                .color(if bytes.is_none() { palette.warn } else { palette.faint }),
                        );
                    }
                });
            }
        });
        // What the system itself shows now, where it draws the wallpaper:
        // the person's own, while the configuration names none.
        if let Some(system) = self.system_shows(&context).cloned() {
            ui.add_space(4.0);
            let kind = match system.kind.as_str() {
                "picture" => "a picture",
                "colour" => "a plain colour",
                "slideshow" => "Windows' own slideshow",
                "spotlight" => "Windows Spotlight",
                _ => "not known",
            };
            ui.label(
                egui::RichText::new(format!("Windows' background now: {kind}"))
                    .size(12.0)
                    .color(palette.muted),
            );
            for picture in &system.pictures {
                let shown = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(picture)
                                .monospace()
                                .size(11.5)
                                .color(palette.faint),
                        )
                        .truncate()
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text(format!("{picture}\nA press copies the path."));
                if shown.clicked() {
                    ui.ctx().copy_text(picture.clone());
                }
            }
        }
        if state.is_none() {
            ui.label(
                egui::RichText::new(
                    "No window manager is connected: one display of the usual shape is drawn.",
                )
                .size(12.0)
                .color(palette.faint),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(320.0, 180.0))
    }

    #[test]
    fn a_picture_is_put_on_a_display_as_its_fit_says() {
        let square = egui::vec2(1000.0, 1000.0);
        // Pulled to the display's shape: all of it, over all of the display.
        let stretched = pieces(WallpaperFit::Stretch, square, screen(), 0.25);
        assert_eq!(stretched.len(), 1);
        assert_eq!(stretched[0].0, screen());
        assert_eq!(stretched[0].1.size(), egui::vec2(1.0, 1.0));
        // Covering it: the display is full, and the square's top and
        // bottom are cut off (its middle 56 hundredths are shown).
        let filled = pieces(WallpaperFit::Fill, square, screen(), 0.25);
        assert_eq!(filled[0].0, screen());
        assert_eq!(filled[0].1.width(), 1.0);
        assert!((filled[0].1.height() - 180.0 / 320.0).abs() < 1e-4);
        assert!((filled[0].1.center().y - 0.5).abs() < 1e-4);
        // Whole: as high as the display, in the middle, bars beside it.
        let fitted = pieces(WallpaperFit::Fit, square, screen(), 0.25);
        assert_eq!(fitted[0].0.size(), egui::vec2(180.0, 180.0));
        assert_eq!(fitted[0].0.center(), screen().center());
        assert_eq!(fitted[0].1.size(), egui::vec2(1.0, 1.0));
        // At its own size (a quarter of it in the drawing): 250 by 250
        // in the middle, cut to the display's height.
        let centred = pieces(WallpaperFit::Centre, square, screen(), 0.25);
        assert_eq!(centred[0].0.size(), egui::vec2(250.0, 180.0));
        assert!((centred[0].1.height() - 180.0 / 250.0).abs() < 1e-4);
        // Again and again from the corner: two across (the second cut),
        // one down.
        let tiled = pieces(WallpaperFit::Tile, square, screen(), 0.25);
        assert_eq!(tiled.len(), 2);
        assert_eq!(tiled[0].0.min, screen().min);
        assert!((tiled[1].0.width() - 70.0).abs() < 1e-3);
        // Nothing to draw of a picture without a size.
        assert!(pieces(WallpaperFit::Fill, egui::vec2(0.0, 10.0), screen(), 1.0).is_empty());
    }

    #[test]
    fn what_is_known_of_the_picture_in_use_is_said() {
        assert_eq!(
            about_picture(Some(egui::vec2(3840.0, 2160.0)), Some(2 * 1024 * 1024), WallpaperFit::Fill),
            "3840 × 2160 px  ·  2.0 MB  ·  fill"
        );
        assert_eq!(about_picture(None, Some(100), WallpaperFit::Tile), "100 B  ·  tile");
        // What gevrek's command says the system shows: each picture once.
        let told = serde_json::json!({"system": {"kind": "picture", "displays": [
            {"display": "one", "picture": r"C:\own\a.jpg"},
            {"display": "two", "picture": r"C:\own\a.jpg"},
            {"display": "three", "picture": ""},
        ]}});
        assert_eq!(
            system_shows_in(&told),
            Some(SystemShows {
                kind: "picture".to_string(),
                pictures: vec![r"C:\own\a.jpg".to_string()],
            })
        );
        assert_eq!(system_shows_in(&serde_json::json!({"image": "x"})), None);
        assert_eq!(system_shows_in(&serde_json::json!({"system": null})), None);
    }

    #[test]
    fn the_displays_are_the_window_managers_or_one_of_the_usual_shape() {
        assert_eq!(
            displays(None),
            [Display {
                name: String::new(),
                size: egui::vec2(1920.0, 1080.0)
            }]
        );
        let state = serde_json::json!({"monitors": [
            {"name": "Monitor A", "connector": "DP-1", "size": {"width": 2560, "height": 1440}},
            {"name": "Monitor B", "connector": "", "size": {"width": 1080, "height": 1920}},
            {"name": "broken"},
        ]});
        let found = displays(Some(&state));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "DP-1");
        assert_eq!(found[1].name, "Monitor B");
        assert_eq!(found[1].size, egui::vec2(1080.0, 1920.0));
    }

    #[test]
    fn a_display_shows_its_own_picture_the_common_one_or_the_colour() {
        let config: WallpaperConfig = serde_json::from_str(
            r#"{"image": "/pictures/all.png", "fit": "fit",
                "outputs": [{"name": "DP-2", "image": "/pictures/second.jpg", "fit": "tile"}, {"name": "DP-3"}]}"#,
        )
        .expect("a wallpaper");
        assert_eq!(
            shown_on(&config, "DP-1"),
            (Some("/pictures/all.png".into()), WallpaperFit::Fit, false)
        );
        assert_eq!(
            shown_on(&config, "DP-2"),
            (Some("/pictures/second.jpg".into()), WallpaperFit::Tile, false)
        );
        assert_eq!(shown_on(&config, "DP-3").0, None);
        // A slideshow whose folder is not there: said, and nothing shown.
        let slides: WallpaperConfig =
            serde_json::from_str(r#"{"slideshow": {"folder": "/no/such/folder/anywhere"}}"#)
                .expect("a wallpaper");
        assert_eq!(shown_on(&slides, "DP-1"), (None, WallpaperFit::Fill, true));
    }
}

/// What the window manager says of the wallpaper now (its notices whose
/// names begin with `wallpaper`), each as its title and its words: on
/// Windows, that the picture of the configuration is not shown, and why.
pub fn said_of_it(state: Option<&serde_json::Value>) -> Vec<(String, String)> {
    state
        .and_then(|state| state["notices"].as_array())
        .into_iter()
        .flatten()
        .filter(|notice| {
            notice["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("wallpaper"))
        })
        .map(|notice| {
            (
                notice["title"].as_str().unwrap_or_default().to_string(),
                notice["text"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod said_tests {
    #[test]
    fn only_what_is_said_of_the_wallpaper_is_shown_on_its_page() {
        let state = serde_json::json!({"notices": [
            {"id": "snap-windows", "title": "Shift and drag", "text": "Windows has it too."},
            {"id": "wallpaper-not-changed", "title": "Not shown", "text": "A slideshow."},
        ]});
        assert_eq!(
            super::said_of_it(Some(&state)),
            vec![("Not shown".to_string(), "A slideshow.".to_string())]
        );
        assert!(super::said_of_it(None).is_empty());
        assert!(super::said_of_it(Some(&serde_json::json!({}))).is_empty());
    }
}