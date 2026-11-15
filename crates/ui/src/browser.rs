//! A browser for files and folders, drawn by gevrek itself: to choose a
//! file or a folder where a path is asked for.
//!
//! The system's own dialog is another program's window with another look,
//! and on a Wayland desktop there may be none to ask; this one is a part
//! of the window it is opened in. It has what such a dialog has:
//!
//! - the places a person goes to (the home folder and what lies in it,
//!   the drives or the mounted volumes) at the left;
//! - the way to where it is as steps that can be pressed, or as a path to
//!   type; back, forward, up, and reading the folder again;
//! - the folder's entries with their size and when they were changed, in
//!   the order of whichever of the three is pressed; the folders first;
//! - only the kinds of file that are asked for (pictures, programs), or
//!   all; hidden entries when asked for; a few letters to find an entry;
//! - a new folder; a name to type; the keyboard (the arrows, Enter,
//!   Backspace for up, Escape).
//!
//! What it lists is read when a folder is entered and when it is asked to
//! read again, not with every frame.

use crate::theme::{self, icons};
use crate::widgets::{self, PathKind};
use egui::RichText;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One entry of a folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
    pub size: u64,
    pub changed: Option<SystemTime>,
    pub hidden: bool,
}

/// What the entries are ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Name,
    Size,
    Changed,
}

/// How the browser ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Chosen(PathBuf),
    Cancelled,
}

/// A place a person goes to, at the browser's left.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub sign: &'static str,
    pub name: String,
    pub path: PathBuf,
}

/// The home folder, by the environment.
pub fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|folder| folder.is_dir())
}

/// The places of this machine: the home folder, the folders in it that
/// people keep things in (those that are there), and the drives or the
/// volumes.
pub fn places() -> Vec<Place> {
    let mut found = Vec::new();
    if let Some(home) = home() {
        found.push(Place {
            sign: icons::HOUSE,
            name: "Home".to_string(),
            path: home.clone(),
        });
        for (sign, name) in [
            (icons::DESKTOP, "Desktop"),
            (icons::FILE_TEXT, "Documents"),
            (icons::DOWNLOAD_SIMPLE, "Downloads"),
            (icons::IMAGES, "Pictures"),
            (icons::MUSIC_NOTES, "Music"),
            (icons::FILM_STRIP, "Videos"),
        ] {
            let folder = home.join(name);
            if folder.is_dir() {
                found.push(Place {
                    sign,
                    name: name.to_string(),
                    path: folder,
                });
            }
        }
    }
    if cfg!(windows) {
        for letter in b'A'..=b'Z' {
            let drive = PathBuf::from(format!("{}:\\", letter as char));
            if drive.is_dir() {
                found.push(Place {
                    sign: icons::HARD_DRIVES,
                    name: format!("{}:", letter as char),
                    path: drive,
                });
            }
        }
    } else {
        found.push(Place {
            sign: icons::HARD_DRIVES,
            name: "This machine".to_string(),
            path: PathBuf::from("/"),
        });
        let user = std::env::var("USER").unwrap_or_default();
        for under in [
            PathBuf::from("/mnt"),
            PathBuf::from("/media"),
            PathBuf::from("/run/media").join(&user),
        ] {
            for volume in std::fs::read_dir(&under).into_iter().flatten().flatten() {
                let path = volume.path();
                if path.is_dir() {
                    found.push(Place {
                        sign: icons::HARD_DRIVES,
                        name: volume.file_name().to_string_lossy().to_string(),
                        path,
                    });
                }
            }
        }
    }
    found
}

/// The entries of a folder, as they are on the disk.
pub fn listing(folder: &Path) -> Result<Vec<Item>, String> {
    let read = std::fs::read_dir(folder).map_err(|error| error.to_string())?;
    Ok(read
        .flatten()
        .map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // (What a link points to counts: a link to a folder is one.)
            let about = std::fs::metadata(&path).ok();
            let hidden = name.starts_with('.') || hidden_by_the_system(&path);
            Item {
                is_folder: about.as_ref().is_some_and(std::fs::Metadata::is_dir),
                size: about.as_ref().map_or(0, std::fs::Metadata::len),
                changed: about.and_then(|about| about.modified().ok()),
                hidden,
                name,
                path,
            }
        })
        .collect())
}

#[cfg(windows)]
fn hidden_by_the_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    // (FILE_ATTRIBUTE_HIDDEN.)
    std::fs::symlink_metadata(path).is_ok_and(|about| about.file_attributes() & 0x2 != 0)
}

#[cfg(not(windows))]
fn hidden_by_the_system(_path: &Path) -> bool {
    false
}

/// Whether a file is of one of the kinds asked for (by the end of its
/// name; no kinds asked for: every file is).
pub fn is_of(name: &str, kinds: &[String]) -> bool {
    kinds.is_empty()
        || Path::new(name)
            .extension()
            .and_then(|end| end.to_str())
            .is_some_and(|end| kinds.iter().any(|kind| kind.eq_ignore_ascii_case(end)))
}

/// The entries to show: the folders first, then the files, each group in
/// the order asked for; without the hidden ones unless they are asked
/// for, without the files of other kinds than are asked for, and only
/// those whose names have the letters typed to find one.
pub fn shown(
    items: &[Item],
    order: (Order, bool),
    hidden: bool,
    kinds: &[String],
    find: &str,
) -> Vec<Item> {
    let needle = find.trim().to_lowercase();
    let mut kept: Vec<Item> = items
        .iter()
        .filter(|item| hidden || !item.hidden)
        .filter(|item| item.is_folder || is_of(&item.name, kinds))
        .filter(|item| needle.is_empty() || item.name.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    let (by, rising) = order;
    kept.sort_by(|one, other| {
        let within = match by {
            Order::Name => std::cmp::Ordering::Equal,
            Order::Size => one.size.cmp(&other.size),
            Order::Changed => one.changed.cmp(&other.changed),
        }
        .then_with(|| one.name.to_lowercase().cmp(&other.name.to_lowercase()));
        // (Folders stay first whichever way the order runs.)
        other
            .is_folder
            .cmp(&one.is_folder)
            .then(if rising { within } else { within.reverse() })
    });
    kept
}

/// A size as a person reads it.
pub fn size_said(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if size < 10.0 {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

/// A moment as a day and a time of day (`2026-11-15 21:04`), in universal
/// time: enough to tell entries apart, with nothing but the clock's count.
pub fn moment_said(moment: SystemTime) -> String {
    let Ok(since) = moment.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::new();
    };
    let seconds = since.as_secs();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // (Days to a date of the calendar: the usual reckoning from the
    // first of March of a year that can be divided by 400.)
    let shifted = days as i64 + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 { month_part + 3 } else { month_part - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// The steps of the way to a folder, each with the path it leads to:
/// `C:\` `Users` `ada`, or `/` `home` `ada`.
pub fn steps(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut so_far = PathBuf::new();
    for part in folder.components() {
        so_far.push(part.as_os_str());
        let name = match part {
            std::path::Component::Prefix(prefix) => {
                // (The drive and its root are one step.)
                prefix.as_os_str().to_string_lossy().to_string()
            }
            std::path::Component::RootDir => {
                if let Some((last, path)) = found.last_mut() {
                    let last: &mut String = last;
                    last.push(std::path::MAIN_SEPARATOR);
                    *path = so_far.clone();
                    continue;
                }
                std::path::MAIN_SEPARATOR.to_string()
            }
            other => other.as_os_str().to_string_lossy().to_string(),
        };
        found.push((name, so_far.clone()));
    }
    found
}

/// Where a browser begins for what is typed in a path's field: the folder
/// that is named, or the folder of the file that is named, or the nearest
/// folder above that is there; the home folder where nothing is typed.
pub fn start_for(typed: &str, home: Option<&Path>) -> PathBuf {
    let fallback = || {
        home.map(Path::to_path_buf)
            .or_else(self::home)
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
    };
    if typed.trim().is_empty() {
        return fallback();
    }
    let whole = widgets::whole_path(typed, home);
    let mut at: Option<&Path> = Some(whole.as_path());
    while let Some(folder) = at {
        if folder.is_dir() {
            return folder.to_path_buf();
        }
        at = folder.parent();
    }
    fallback()
}

/// The picture of the chosen file, small, for the browser's side.
enum Preview {
    Reading(std::sync::mpsc::Receiver<Result<(egui::ColorImage, [u32; 2]), String>>),
    /// What is drawn, and how large the file's own picture is.
    Ready(egui::TextureHandle, [u32; 2]),
    Failed(String),
}

/// The browser, while it is open.
pub struct Browser {
    /// The picture that is chosen, read beside the drawing.
    preview: Option<(PathBuf, Preview)>,
    kind: PathKind,
    /// The kinds of file asked for, by the ends of their names, and the
    /// word for them ("Pictures"); none: every file.
    kinds: Vec<String>,
    kinds_name: String,
    /// Every file is shown, whatever kinds are asked for.
    all_kinds: bool,
    folder: PathBuf,
    items: Vec<Item>,
    /// Why the folder could not be read.
    refused: Option<String>,
    chosen: Option<PathBuf>,
    /// The name typed at the foot.
    name: String,
    /// The path typed in place of the steps, while it is typed.
    typed_path: Option<String>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    order: (Order, bool),
    hidden: bool,
    find: String,
    /// The name of a folder that is being made.
    new_folder: Option<String>,
    places: Vec<Place>,
    /// The chosen entry is to be brought into view.
    bring: bool,
}

impl Browser {
    /// A browser for a file or a folder that begins where `typed` leads.
    /// `kinds`: the ends of the names of the files that are asked for,
    /// with the word for them.
    pub fn new(kind: PathKind, typed: &str, home: Option<&Path>, kinds_name: &str, kinds: &[&str]) -> Self {
        let whole = widgets::whole_path(typed, home);
        let mut browser = Self {
            preview: None,
            kind,
            kinds: kinds.iter().map(|kind| (*kind).to_string()).collect(),
            kinds_name: kinds_name.to_string(),
            all_kinds: false,
            folder: PathBuf::new(),
            items: Vec::new(),
            refused: None,
            chosen: None,
            name: String::new(),
            typed_path: None,
            back: Vec::new(),
            forward: Vec::new(),
            order: (Order::Name, true),
            hidden: false,
            find: String::new(),
            new_folder: None,
            places: places(),
            bring: true,
        };
        browser.enter(start_for(typed, home), false);
        // The file that is named is the one chosen at first.
        if kind != PathKind::Folder && whole.is_file() {
            browser.name = whole
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            browser.chosen = Some(whole);
        }
        browser
    }

    /// The folder it shows.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn read(&mut self) {
        match listing(&self.folder) {
            Ok(items) => {
                self.items = items;
                self.refused = None;
            }
            Err(why) => {
                self.items.clear();
                self.refused = Some(why);
            }
        }
    }

    /// Go into a folder. `remember`: the one it leaves is where Back goes.
    fn enter(&mut self, folder: PathBuf, remember: bool) {
        if remember && folder != self.folder && !self.folder.as_os_str().is_empty() {
            self.back.push(self.folder.clone());
            self.forward.clear();
        }
        self.folder = folder;
        self.chosen = None;
        self.find.clear();
        self.typed_path = None;
        self.new_folder = None;
        self.bring = true;
        self.read();
    }

    fn kinds_now(&self) -> &[String] {
        if self.all_kinds || self.kind == PathKind::Folder {
            &[]
        } else {
            &self.kinds
        }
    }

    /// What Enter, or the button, would choose now: the chosen entry (a
    /// folder only where a folder is asked for), else what is typed as a
    /// name, else (for a folder) the folder that is shown.
    fn answer(&self) -> Option<PathBuf> {
        let typed = self.name.trim();
        match self.kind {
            PathKind::Folder => Some(
                self.chosen
                    .clone()
                    .filter(|path| path.is_dir())
                    .unwrap_or_else(|| self.folder.clone()),
            ),
            PathKind::File | PathKind::Program => {
                if let Some(file) = self.chosen.clone().filter(|path| path.is_file()) {
                    return Some(file);
                }
                (!typed.is_empty())
                    .then(|| self.folder.join(typed))
                    .filter(|path| path.is_file())
            }
        }
    }

    /// The picture of the chosen file, where that is a picture: read
    /// away from the drawing the first time it is chosen, and made small.
    fn look_at_the_chosen(&mut self, context: &egui::Context) {
        let wanted = self
            .chosen
            .clone()
            .filter(|path| path.is_file() && is_of(&path.to_string_lossy(), &PICTURES.map(str::to_string)));
        let Some(wanted) = wanted else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_none_or(|(path, _)| *path != wanted) {
            let (tell, told) = std::sync::mpsc::channel();
            let (path, wake) = (wanted.clone(), context.clone());
            let started = std::thread::Builder::new()
                .name("gevrek-browser-preview".to_string())
                .spawn(move || {
                    let read = image::open(&path)
                        .map(|picture| {
                            let size = [picture.width(), picture.height()];
                            let small = picture.thumbnail(440, 440).to_rgba8();
                            (
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [small.width() as usize, small.height() as usize],
                                    small.as_raw(),
                                ),
                                size,
                            )
                        })
                        .map_err(|error| error.to_string());
                    let _ = tell.send(read);
                    wake.request_repaint();
                });
            self.preview = Some((
                wanted.clone(),
                match started {
                    Ok(_) => Preview::Reading(told),
                    Err(error) => Preview::Failed(error.to_string()),
                },
            ));
        }
        let arrived = match &self.preview {
            Some((_, Preview::Reading(told))) => told.try_recv().ok(),
            _ => None,
        };
        if let Some(read) = arrived {
            let now = match read {
                Ok((picture, size)) => Preview::Ready(
                    context.load_texture(
                        format!("browser-preview-{}", wanted.display()),
                        picture,
                        egui::TextureOptions::LINEAR,
                    ),
                    size,
                ),
                Err(why) => Preview::Failed(why),
            };
            self.preview = Some((wanted, now));
        }
    }

    /// Draw the browser over the window it is opened in. What came of
    /// it, once it is over.
    pub fn show(&mut self, context: &egui::Context) -> Option<Done> {
        let palette = theme::pal(context);
        self.look_at_the_chosen(context);
        let mut done = None;
        let mut go: Option<(PathBuf, bool)> = None;
        let shown = shown(&self.items, self.order, self.hidden, self.kinds_now(), &self.find);
        // The keys, where no field is typed in.
        let typing = context.memory(|memory| memory.focused().is_some());
        let key = |key: egui::Key| context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        if key(egui::Key::Escape) {
            if self.typed_path.is_some() || self.new_folder.is_some() {
                self.typed_path = None;
                self.new_folder = None;
            } else {
                done = Some(Done::Cancelled);
            }
        }
        if !typing {
            let at = self
                .chosen
                .as_ref()
                .and_then(|chosen| shown.iter().position(|item| item.path == *chosen));
            let step = i32::from(key(egui::Key::ArrowDown)) - i32::from(key(egui::Key::ArrowUp));
            if step != 0 && !shown.is_empty() {
                let next = match at {
                    Some(at) => (at as i32 + step).clamp(0, shown.len() as i32 - 1) as usize,
                    None => 0,
                };
                self.choose(&shown[next]);
                self.bring = true;
            }
            if key(egui::Key::Backspace) {
                if let Some(up) = self.folder.parent() {
                    go = Some((up.to_path_buf(), true));
                }
            }
            if key(egui::Key::Enter) {
                match self.chosen.clone().filter(|path| path.is_dir()) {
                    // (Enter goes into a folder; a folder is chosen by
                    // the button.)
                    Some(folder) => go = Some((folder, true)),
                    None => {
                        if let Some(answer) = self.answer() {
                            done = Some(Done::Chosen(answer));
                        }
                    }
                }
            }
        }
        let alt = |key: egui::Key| context.input_mut(|input| input.consume_key(egui::Modifiers::ALT, key));
        if alt(egui::Key::ArrowLeft) {
            self.go_back();
        }
        if alt(egui::Key::ArrowRight) {
            self.go_forward();
        }
        if alt(egui::Key::ArrowUp) {
            if let Some(up) = self.folder.parent() {
                go = Some((up.to_path_buf(), true));
            }
        }

        let title = match self.kind {
            PathKind::Folder => "Choose a folder",
            PathKind::File => "Choose a file",
            PathKind::Program => "Choose a program",
        };
        let screen = context.content_rect();
        let size = egui::vec2(
            (screen.width() - 80.0).clamp(520.0, 980.0),
            (screen.height() - 80.0).clamp(360.0, 640.0),
        );
        let modal = egui::Modal::new(egui::Id::unique("gevrek file browser")).show(context, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.spacing_mut().item_spacing.y = 6.0;
            // Its name, and the way out.
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong().size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, icons::X, "Leave it as it is (Escape)").clicked() {
                        done = Some(Done::Cancelled);
                    }
                });
            });
            // Where it is, and the ways from there.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui
                    .add_enabled(
                        !self.back.is_empty(),
                        egui::Button::new(icons::ARROW_LEFT).frame_when_inactive(false),
                    )
                    .on_hover_text("Back (Alt+Left)")
                    .clicked()
                {
                    self.go_back();
                }
                if ui
                    .add_enabled(
                        !self.forward.is_empty(),
                        egui::Button::new(icons::ARROW_RIGHT).frame_when_inactive(false),
                    )
                    .on_hover_text("Forward (Alt+Right)")
                    .clicked()
                {
                    self.go_forward();
                }
                if ui
                    .add_enabled(
                        self.folder.parent().is_some(),
                        egui::Button::new(icons::ARROW_UP).frame_when_inactive(false),
                    )
                    .on_hover_text("The folder above (Backspace)")
                    .clicked()
                {
                    if let Some(up) = self.folder.parent() {
                        go = Some((up.to_path_buf(), true));
                    }
                }
                if widgets::icon_button(ui, icons::ARROW_CLOCKWISE, "Read the folder again").clicked() {
                    self.read();
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::icon_button(ui, icons::FOLDER_PLUS, "A new folder here").clicked() {
                        self.new_folder = Some(String::new());
                    }
                    if widgets::icon_button(
                        ui,
                        if self.hidden { icons::EYE } else { icons::EYE_SLASH },
                        if self.hidden {
                            "Hidden entries are shown: a press hides them"
                        } else {
                            "Hidden entries are not shown: a press shows them"
                        },
                    )
                    .clicked()
                    {
                        self.hidden = !self.hidden;
                    }
                    if widgets::icon_button(ui, icons::PENCIL_SIMPLE, "Type the path").clicked() {
                        self.typed_path = match self.typed_path {
                            Some(_) => None,
                            None => Some(self.folder.display().to_string()),
                        };
                    }
                    // The way to here: typed, or as steps.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        match self.typed_path.as_mut() {
                            Some(typed) => {
                                let field = ui.add(
                                    egui::TextEdit::singleline(typed)
                                        .desired_width(ui.available_width())
                                        .font(egui::TextStyle::Monospace),
                                );
                                if !field.has_focus() && !field.lost_focus() {
                                    field.request_focus();
                                }
                                if field.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                {
                                    let wanted = widgets::whole_path(typed, home().as_deref());
                                    if wanted.is_dir() {
                                        go = Some((wanted, true));
                                    } else if wanted.is_file() && self.kind != PathKind::Folder {
                                        done = Some(Done::Chosen(wanted));
                                    } else {
                                        self.refused = Some(format!("{} is not there", wanted.display()));
                                    }
                                }
                            }
                            None => {
                                egui::ScrollArea::horizontal()
                                    .id_salt("the way")
                                    .stick_to_right(true)
                                    .show(ui, |ui| {
                                        let all = steps(&self.folder);
                                        let last = all.len().saturating_sub(1);
                                        for (index, (name, path)) in all.into_iter().enumerate() {
                                            if index > 0 {
                                                ui.label(
                                                    RichText::new(icons::CARET_RIGHT)
                                                        .size(11.0)
                                                        .color(palette.faint),
                                                );
                                            }
                                            let words = if index == last {
                                                RichText::new(name).strong()
                                            } else {
                                                RichText::new(name).color(palette.muted)
                                            };
                                            if ui
                                                .add(egui::Button::new(words).frame_when_inactive(false))
                                                .clicked()
                                                && index != last
                                            {
                                                go = Some((path, true));
                                            }
                                        }
                                    });
                            }
                        }
                    });
                });
            });
            ui.separator();
            // The foot is kept room for; the places and the entries have
            // the rest.
            let foot = 78.0 + if self.new_folder.is_some() { 34.0 } else { 0.0 };
            let room = (ui.available_height() - foot).max(120.0);
            ui.horizontal(|ui| {
                ui.set_height(room);
                // The places.
                ui.vertical(|ui| {
                    ui.set_width(168.0);
                    egui::ScrollArea::vertical()
                        .id_salt("the places")
                        .max_height(room)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for place in &self.places {
                                let here = self.folder == place.path;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 28.0),
                                    egui::Sense::click(),
                                );
                                widgets::list_row_bg(ui, &response, rect, here);
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(8.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    place.sign,
                                    egui::FontId::proportional(15.0),
                                    if here { palette.accent_fg() } else { palette.muted },
                                );
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(32.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    &place.name,
                                    egui::FontId::proportional(13.5),
                                    palette.text,
                                );
                                if response.on_hover_text(place.path.display().to_string()).clicked() {
                                    go = Some((place.path.clone(), true));
                                }
                            }
                        });
                });
                ui.separator();
                // The entries. (Beside them, the chosen picture: they
                // leave it its room.)
                let beside = if self.preview.is_some() { 244.0 } else { 0.0 };
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - beside).max(220.0));
                    let wide = ui.available_width();
                    let (size_at, changed_at) = (wide - 250.0, wide - 150.0);
                    // What they are ordered by: a press on a column's name.
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let left = ui.cursor().left();
                        for (by, name, at, width) in [
                            (Order::Name, "Name", 0.0, size_at),
                            (Order::Size, "Size", size_at, 100.0),
                            (Order::Changed, "Changed", changed_at, 150.0),
                        ] {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width.max(60.0), 22.0),
                                egui::Sense::click(),
                            );
                            let _ = (left, at);
                            let mine = self.order.0 == by;
                            let words = if mine {
                                format!(
                                    "{name}  {}",
                                    if self.order.1 { icons::CARET_UP } else { icons::CARET_DOWN }
                                )
                            } else {
                                name.to_string()
                            };
                            ui.painter().text(
                                rect.left_center() + egui::vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                words,
                                egui::FontId::proportional(12.0),
                                if mine || response.hovered() { palette.text } else { palette.faint },
                            );
                            if response.clicked() {
                                self.order = if mine { (by, !self.order.1) } else { (by, true) };
                            }
                        }
                    });
                    if let Some(why) = &self.refused {
                        ui.label(
                            RichText::new(format!("{}  {why}", icons::WARNING)).color(palette.warn),
                        );
                    }
                    let row = 26.0;
                    let mut pressed: Option<(Item, bool)> = None;
                    let chosen_at = self
                        .chosen
                        .as_ref()
                        .and_then(|chosen| shown.iter().position(|item| item.path == *chosen));
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("the entries")
                        .max_height(room - 30.0)
                        .auto_shrink([false, false]);
                    if let (true, Some(at)) = (self.bring, chosen_at) {
                        // (Brought into view where the keys moved the choice.)
                        let top = at as f32 * (row + ui.spacing().item_spacing.y);
                        let view = room - 30.0;
                        let now = ui
                            .data(|data| data.get_temp::<f32>(egui::Id::unique("gevrek browser offset")))
                            .unwrap_or(0.0);
                        if top < now {
                            area = area.vertical_scroll_offset(top);
                        } else if top + row > now + view {
                            area = area.vertical_scroll_offset(top + row - view);
                        }
                    }
                    self.bring = false;
                    let out = area.show_rows(ui, row, shown.len(), |ui, rows| {
                        for item in &shown[rows] {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row),
                                egui::Sense::click(),
                            );
                            let mine = self.chosen.as_ref() == Some(&item.path);
                            // (Where a folder is asked for, a file is only
                            // there to be seen.)
                            let faint = self.kind == PathKind::Folder && !item.is_folder;
                            widgets::list_row_bg(ui, &response, rect, mine);
                            let sign = if item.is_folder {
                                icons::FOLDER
                            } else if is_of(&item.name, &PICTURES.map(str::to_string)) {
                                icons::FILE_IMAGE
                            } else {
                                icons::FILE
                            };
                            let colour = if faint || item.hidden { palette.faint } else { palette.text };
                            let painter = ui.painter().with_clip_rect(rect);
                            painter.text(
                                rect.left_center() + egui::vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                sign,
                                egui::FontId::proportional(15.0),
                                if item.is_folder { palette.accent_fg() } else { palette.muted },
                            );
                            let name_room = egui::Rect::from_min_max(
                                rect.min + egui::vec2(32.0, 0.0),
                                egui::pos2(rect.left() + size_at - 8.0, rect.bottom()),
                            );
                            ui.painter().with_clip_rect(name_room).text(
                                name_room.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &item.name,
                                egui::FontId::proportional(13.5),
                                colour,
                            );
                            if !item.is_folder {
                                painter.text(
                                    egui::pos2(rect.left() + size_at + 8.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    size_said(item.size),
                                    egui::FontId::proportional(12.0),
                                    palette.muted,
                                );
                            }
                            if let Some(changed) = item.changed {
                                painter.text(
                                    egui::pos2(rect.left() + changed_at + 8.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    moment_said(changed),
                                    egui::FontId::proportional(12.0),
                                    palette.muted,
                                );
                            }
                            if response.double_clicked() {
                                pressed = Some((item.clone(), true));
                            } else if response.clicked() {
                                pressed = Some((item.clone(), false));
                            }
                        }
                    });
                    ui.data_mut(|data| {
                        data.insert_temp(egui::Id::unique("gevrek browser offset"), out.state.clamped_offset().y);
                    });
                    if shown.is_empty() && self.refused.is_none() {
                        ui.label(
                            RichText::new(if self.items.is_empty() {
                                "This folder is empty."
                            } else {
                                "Nothing here is of the kind that is asked for."
                            })
                            .color(palette.muted),
                        );
                    }
                    match pressed {
                        Some((item, true)) if item.is_folder => go = Some((item.path, true)),
                        Some((item, true)) if self.kind != PathKind::Folder => {
                            done = Some(Done::Chosen(item.path));
                        }
                        Some((item, _)) => self.choose(&item),
                        None => {}
                    }
                });
                // The chosen picture, small, with what is known of it.
                if let Some((path, preview)) = &self.preview {
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(220.0);
                        let (frame, _) = ui.allocate_exact_size(
                            egui::vec2(220.0, 220.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(frame, 8.0, palette.control);
                        let mut about = String::new();
                        match preview {
                            Preview::Ready(texture, size) => {
                                // (Whole, in the middle of its frame.)
                                let own = texture.size_vec2();
                                let scale = (frame.width() / own.x).min(frame.height() / own.y).min(1.0);
                                let place = egui::Rect::from_center_size(frame.center(), own * scale);
                                ui.painter().image(
                                    texture.id(),
                                    place,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                                about = format!("{} × {} px", size[0], size[1]);
                            }
                            Preview::Reading(_) => {
                                ui.painter().text(
                                    frame.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Reading…",
                                    egui::FontId::proportional(12.5),
                                    palette.faint,
                                );
                            }
                            Preview::Failed(why) => {
                                ui.painter().text(
                                    frame.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icons::WARNING,
                                    egui::FontId::proportional(22.0),
                                    palette.warn,
                                );
                                about = format!("It cannot be shown: {why}");
                            }
                        }
                        ui.painter().rect_stroke(
                            frame,
                            8.0,
                            egui::Stroke::new(1.0, palette.border),
                            egui::StrokeKind::Inside,
                        );
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.add(egui::Label::new(RichText::new(name).size(13.0)).truncate());
                        if let Ok(bytes) = std::fs::metadata(path).map(|file| file.len()) {
                            about = if about.is_empty() {
                                size_said(bytes)
                            } else {
                                format!("{about}  ·  {}", size_said(bytes))
                            };
                        }
                        ui.add(
                            egui::Label::new(RichText::new(about).size(11.5).color(palette.faint)).wrap(),
                        );
                    });
                }
            });
            ui.separator();
            // A folder that is being made.
            if let Some(typed) = self.new_folder.as_mut() {
                let mut make = false;
                let mut leave = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  New folder", icons::FOLDER_PLUS)).color(palette.muted));
                    let field = ui.add(
                        egui::TextEdit::singleline(typed)
                            .hint_text("its name")
                            .desired_width(260.0),
                    );
                    if !field.has_focus() && !field.lost_focus() {
                        field.request_focus();
                    }
                    make = (field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        || widgets::secondary_button(ui, "Make it").clicked();
                    leave = widgets::text_button(ui, "", "Leave it").clicked();
                });
                if make && !typed.trim().is_empty() {
                    let folder = self.folder.join(typed.trim());
                    match std::fs::create_dir(&folder) {
                        Ok(()) => go = Some((folder, true)),
                        Err(error) => self.refused = Some(format!("The folder was not made: {error}")),
                    }
                } else if leave {
                    self.new_folder = None;
                }
            }
            // The foot: what to find, which kinds, the name, and the end.
            ui.horizontal(|ui| {
                widgets::search_field(ui, &mut self.find, "Find in this folder", 200.0);
                if self.kind != PathKind::Folder && !self.kinds.is_empty() {
                    let said = if self.all_kinds {
                        "All files".to_string()
                    } else {
                        self.kinds_name.clone()
                    };
                    egui::ComboBox::from_id_salt("the kinds")
                        .selected_text(said)
                        .show_ui(ui, |ui| {
                            let ends = self
                                .kinds
                                .iter()
                                .map(|kind| format!("*.{kind}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            ui.selectable_value(
                                &mut self.all_kinds,
                                false,
                                format!("{} ({ends})", self.kinds_name),
                            );
                            ui.selectable_value(&mut self.all_kinds, true, "All files");
                        });
                }
                ui.label(
                    RichText::new(format!(
                        "{} of {}",
                        shown.len(),
                        self.items.len()
                    ))
                    .size(12.0)
                    .color(palette.faint),
                );
            });
            ui.horizontal(|ui| {
                let answer = self.answer();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let word = match (self.kind, &self.chosen) {
                        (PathKind::Folder, Some(chosen)) if chosen.is_dir() => "Choose the folder",
                        (PathKind::Folder, _) => "Choose this folder",
                        _ => "Choose",
                    };
                    if widgets::primary_button(ui, answer.is_some(), word).clicked() {
                        if let Some(answer) = answer.clone() {
                            done = Some(Done::Chosen(answer));
                        }
                    }
                    if widgets::secondary_button(ui, "Cancel").clicked() {
                        done = Some(Done::Cancelled);
                    }
                    // What would be chosen, or a name to type.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        match self.kind {
                            PathKind::Folder => {
                                let said = answer
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_default();
                                ui.label(RichText::new(said).monospace().size(12.5).color(palette.muted));
                            }
                            PathKind::File | PathKind::Program => {
                                ui.label(RichText::new("Name").color(palette.muted));
                                let field = ui.add(
                                    egui::TextEdit::singleline(&mut self.name)
                                        .desired_width(ui.available_width() - 8.0),
                                );
                                if field.changed() {
                                    // (Typed: no entry is the chosen one
                                    // unless it has that name.)
                                    let wanted = self.folder.join(self.name.trim());
                                    self.chosen = wanted.is_file().then_some(wanted);
                                }
                                if field.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                {
                                    let typed = widgets::whole_path(self.name.trim(), home().as_deref());
                                    let wanted = if typed.is_absolute() {
                                        typed
                                    } else {
                                        self.folder.join(self.name.trim())
                                    };
                                    if wanted.is_dir() {
                                        go = Some((wanted, true));
                                    } else if wanted.is_file() {
                                        done = Some(Done::Chosen(wanted));
                                    }
                                }
                            }
                        }
                    });
                });
            });
        });
        // A press beside it leaves it as it is.
        if modal.should_close() && done.is_none() {
            done = Some(Done::Cancelled);
        }
        if let Some((folder, remember)) = go {
            self.enter(folder, remember);
        }
        done
    }

    fn choose(&mut self, item: &Item) {
        self.chosen = Some(item.path.clone());
        if !item.is_folder {
            self.name.clone_from(&item.name);
        }
    }

    fn go_back(&mut self) {
        if let Some(folder) = self.back.pop() {
            self.forward.push(self.folder.clone());
            self.enter(folder, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(folder) = self.forward.pop() {
            self.back.push(self.folder.clone());
            self.enter(folder, false);
        }
    }
}

/// The ends of the names of pictures.
pub const PICTURES: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "webp", "gif"];

/// The ends of the names of what can be started, on this system (none
/// where any file may be a program).
pub const PROGRAMS: &[&str] = if cfg!(windows) {
    &["exe", "bat", "cmd", "com", "lnk", "ps1"]
} else {
    &[]
};

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-browser-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        std::fs::create_dir_all(folder.join("Pictures")).unwrap();
        std::fs::create_dir_all(folder.join(".cache")).unwrap();
        for (file, bytes) in [("b.png", 30), ("A.jpg", 2_000), ("notes.txt", 5), (".hidden.png", 1)] {
            std::fs::write(folder.join(file), vec![0u8; bytes]).unwrap();
        }
        folder
    }

    #[test]
    fn a_folder_is_listed_folders_first_in_the_order_asked_for() {
        let folder = folder("listed");
        let items = listing(&folder).expect("a folder");
        assert_eq!(items.len(), 6);
        let names = |shown: Vec<Item>| shown.into_iter().map(|item| item.name).collect::<Vec<_>>();
        // By name, without the hidden: the folder, then the files, the
        // letters' case aside.
        assert_eq!(
            names(shown(&items, (Order::Name, true), false, &[], "")),
            ["Pictures", "A.jpg", "b.png", "notes.txt"]
        );
        // The other way round: the folder still first.
        assert_eq!(
            names(shown(&items, (Order::Name, false), false, &[], "")),
            ["Pictures", "notes.txt", "b.png", "A.jpg"]
        );
        // By size.
        assert_eq!(
            names(shown(&items, (Order::Size, false), false, &[], "")),
            ["Pictures", "A.jpg", "b.png", "notes.txt"]
        );
        // With the hidden ones.
        assert_eq!(shown(&items, (Order::Name, true), true, &[], "").len(), 6);
        // Only pictures: the folders stay, to go into.
        let pictures: Vec<String> = PICTURES.map(str::to_string).to_vec();
        assert_eq!(
            names(shown(&items, (Order::Name, true), false, &pictures, "")),
            ["Pictures", "A.jpg", "b.png"]
        );
        // A few letters find an entry.
        assert_eq!(names(shown(&items, (Order::Name, true), false, &[], "NOT")), ["notes.txt"]);
        // A folder that is not there says why.
        assert!(listing(&folder.join("no such")).is_err());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn sizes_moments_and_kinds_are_said_as_people_read_them() {
        assert_eq!(size_said(0), "0 B");
        assert_eq!(size_said(1_023), "1023 B");
        assert_eq!(size_said(1_536), "1.5 KB");
        assert_eq!(size_said(20 * 1024 * 1024), "20 MB");
        assert_eq!(size_said(3 * 1024 * 1024 * 1024), "3.0 GB");
        let at = |seconds: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        assert_eq!(moment_said(at(0)), "1970-01-01 00:00");
        assert_eq!(moment_said(at(951_782_400)), "2000-02-29 00:00");
        assert_eq!(moment_said(at(1_794_776_640)), "2026-11-15 21:04");
        let kinds = vec!["png".to_string(), "jpg".to_string()];
        assert!(is_of("a.PNG", &kinds) && is_of("b.jpg", &kinds));
        assert!(!is_of("c.txt", &kinds) && !is_of("png", &kinds));
        assert!(is_of("anything", &[]));
    }

    #[test]
    fn the_way_to_a_folder_is_its_steps() {
        #[cfg(windows)]
        {
            let found = steps(Path::new(r"C:\Users\ada"));
            let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(names, [r"C:\", "Users", "ada"]);
            assert_eq!(found[0].1, PathBuf::from(r"C:\"));
            assert_eq!(found[1].1, PathBuf::from(r"C:\Users"));
        }
        #[cfg(not(windows))]
        {
            let found = steps(Path::new("/home/ada"));
            let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(names, ["/", "home", "ada"]);
            assert_eq!(found[1].1, PathBuf::from("/home"));
        }
    }

    #[test]
    fn a_browser_begins_where_what_is_typed_leads_and_goes_about() {
        let folder = folder("begins");
        let typed = |text: &str| folder.join(text).display().to_string();
        // A file that is there: its folder, and it is the chosen one.
        let browser = Browser::new(PathKind::File, &typed("b.png"), None, "Pictures", &PICTURES);
        assert_eq!(browser.folder(), folder);
        assert_eq!(browser.answer(), Some(folder.join("b.png")));
        // One that is not there: the nearest folder above that is.
        assert_eq!(start_for(&typed("Pictures/no/such/file.png"), None), folder.join("Pictures"));
        // A folder is asked for: the folder shown is the answer until one
        // in it is chosen.
        let mut browser = Browser::new(PathKind::Folder, &typed(""), None, "", &[]);
        assert_eq!(browser.answer(), Some(folder.clone()));
        let pictures = shown(&browser.items, browser.order, false, &[], "")[0].clone();
        browser.choose(&pictures);
        assert_eq!(browser.answer(), Some(folder.join("Pictures")));
        // Into it, back, and forward again.
        browser.enter(folder.join("Pictures"), true);
        assert_eq!(browser.folder(), folder.join("Pictures"));
        browser.go_back();
        assert_eq!(browser.folder(), folder);
        browser.go_forward();
        assert_eq!(browser.folder(), folder.join("Pictures"));
        // A file is asked for and none is chosen: nothing to answer; a
        // name typed that is a file there is the answer.
        let mut browser = Browser::new(PathKind::File, &typed(""), None, "", &[]);
        assert_eq!(browser.answer(), None);
        browser.name = "notes.txt".to_string();
        assert_eq!(browser.answer(), Some(folder.join("notes.txt")));
        browser.name = "nothing.txt".to_string();
        assert_eq!(browser.answer(), None);
        let _ = std::fs::remove_dir_all(folder);
    }
}
