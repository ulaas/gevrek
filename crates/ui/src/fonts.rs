//! Which fonts gevrek writes in: one for the interface, one for monospace
//! text, one for the symbols the others lack, and, for each area of the
//! desktop that wants another, a font of its own (see [`Fonts`]).
//!
//! What is chosen is a font file. "Default" keeps gevrek's own choice: the
//! desktop's sans-serif and monospace fonts where the system-font setting
//! asks for them (looked up with fontconfig), egui's built-in fonts
//! otherwise, and the system's symbol font as the last fallback.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use egui::FontFamily;

use crate::theme::Settings;

/// The font chosen for one use.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Choice {
    /// gevrek's own choice (for an area: the interface's font).
    #[default]
    Default,
    /// egui's built-in font (for the symbols: no symbol font at all).
    Builtin,
    /// This font file.
    File(PathBuf),
}

impl Choice {
    /// How it is written in the appearance file.
    fn key(&self) -> String {
        match self {
            Choice::Default => "default".to_owned(),
            Choice::Builtin => "builtin".to_owned(),
            Choice::File(path) => path.display().to_string(),
        }
    }

    fn parse(text: &str) -> Self {
        match text.trim() {
            "" | "default" => Choice::Default,
            "builtin" => Choice::Builtin,
            path => Choice::File(PathBuf::from(path)),
        }
    }
}

/// A part of the desktop that can be written in a font of its own.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Area {
    /// The panel, its taskbar and tray, and their menus and hints.
    Panel,
    Launcher,
    /// Notifications and the notification centre.
    Notifications,
    /// The control centre and the power menu.
    ControlCentre,
    /// The on-screen display (volume, brightness).
    Osd,
    /// The window switcher and the overview.
    Switcher,
    /// The snap zones over the windows.
    Zones,
    /// Questions gevrek asks: sharing the screen, control of the desktop,
    /// a program's shortcuts, a password; the first-run coach.
    Dialogs,
    LockScreen,
    Studio,
}

impl Area {
    pub const ALL: [Area; 10] = [
        Area::Panel,
        Area::Launcher,
        Area::Notifications,
        Area::ControlCentre,
        Area::Osd,
        Area::Switcher,
        Area::Zones,
        Area::Dialogs,
        Area::LockScreen,
        Area::Studio,
    ];

    /// Its name in the appearance file (after `font-`).
    pub fn key(self) -> &'static str {
        match self {
            Area::Panel => "panel",
            Area::Launcher => "launcher",
            Area::Notifications => "notifications",
            Area::ControlCentre => "control-centre",
            Area::Osd => "osd",
            Area::Switcher => "switcher",
            Area::Zones => "zones",
            Area::Dialogs => "dialogs",
            Area::LockScreen => "lock-screen",
            Area::Studio => "studio",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Area::Panel => "Panel and taskbar",
            Area::Launcher => "Launcher",
            Area::Notifications => "Notifications",
            Area::ControlCentre => "Control centre",
            Area::Osd => "On-screen display",
            Area::Switcher => "Switcher and overview",
            Area::Zones => "Snap zones",
            Area::Dialogs => "Questions and the coach",
            Area::LockScreen => "Lock screen",
            Area::Studio => "gevrek Studio",
        }
    }
}

/// The fonts chosen, as the appearance file keeps them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Fonts {
    /// The interface's font, for every area that has none of its own.
    pub text: Choice,
    pub mono: Choice,
    /// The last fallback: draws what no font before it has (⏻, ⌘, ★).
    pub symbols: Choice,
    /// The areas with a font of their own; one that is not here (or is
    /// [`Choice::Default`]) is written in [`Fonts::text`].
    pub areas: BTreeMap<Area, Choice>,
}

impl Fonts {
    /// The font chosen for `area` itself (not what it falls back to).
    pub fn area(&self, area: Area) -> &Choice {
        self.areas.get(&area).unwrap_or(&Choice::Default)
    }

    pub fn set_area(&mut self, area: Area, choice: Choice) {
        if choice == Choice::Default {
            self.areas.remove(&area);
        } else {
            self.areas.insert(area, choice);
        }
    }

    /// Takes one line of the appearance file, if it is about fonts.
    pub(crate) fn read(&mut self, key: &str, value: &str) -> bool {
        match key {
            "font" => self.text = Choice::parse(value),
            "font-mono" => self.mono = Choice::parse(value),
            "font-symbols" => self.symbols = Choice::parse(value),
            other => {
                let Some(area) = other
                    .strip_prefix("font-")
                    .and_then(|key| Area::ALL.into_iter().find(|area| area.key() == key))
                else {
                    return false;
                };
                self.set_area(area, Choice::parse(value));
            }
        }
        true
    }

    /// The lines of the appearance file that keep these fonts.
    pub(crate) fn lines(&self) -> String {
        let mut text = format!(
            "font = {}\nfont-mono = {}\nfont-symbols = {}\n",
            self.text.key(),
            self.mono.key(),
            self.symbols.key()
        );
        for (area, choice) in &self.areas {
            if *choice != Choice::Default {
                text.push_str(&format!("font-{} = {}\n", area.key(), choice.key()));
            }
        }
        text
    }
}

/// Installs the fonts that `settings` chose for `area` (`None`: those of
/// the interface, as a surface of no area has): its own font, or the
/// interface's, egui's built-in fonts as fallback, the Phosphor icon font,
/// and the symbol font last.
pub fn install(ctx: &egui::Context, settings: &Settings, area: Option<Area>) {
    let chosen = &settings.fonts;
    let text = match area.map(|area| chosen.area(area)) {
        Some(Choice::Default) | None => &chosen.text,
        Some(own) => own,
    };
    let mut fonts = egui::FontDefinitions::default();
    for (family, choice, query) in [
        (FontFamily::Proportional, text, "sans-serif:style=Regular"),
        (FontFamily::Monospace, &chosen.mono, "monospace:style=Regular"),
    ] {
        let bytes = match choice {
            Choice::File(path) => file(path).or_else(|| {
                log::warn!("the font {} is not there: gevrek's own is used", path.display());
                system(settings.system_font, query)
            }),
            Choice::Default => system(settings.system_font, query),
            Choice::Builtin => None,
        };
        if let Some(bytes) = bytes {
            let name = format!("chosen-{family:?}");
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_static(bytes).into());
            fonts.families.entry(family).or_default().insert(0, name);
        }
    }
    // The icon font, second among the proportional fonts so that icons can
    // sit inside ordinary text without shadowing the text's own font.
    fonts.font_data.insert(
        "phosphor".to_owned(),
        egui::FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    let at = proportional.len().min(1);
    proportional.insert(at, "phosphor".to_owned());
    // The symbol font, last of all: it only draws what no font before it
    // has (⏻, ⌘, ★ in a window's title), so letters and emoji keep their
    // look. (Whatever the system-font setting: it is no look.)
    let symbols = match &chosen.symbols {
        Choice::Default => symbol_font(),
        Choice::Builtin => None,
        Choice::File(path) => file(path).or_else(symbol_font),
    };
    if let Some(bytes) = symbols {
        fonts.font_data.insert(
            "symbols".to_owned(),
            egui::FontData::from_static(bytes).into(),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("symbols".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

/// The desktop's font for `query`, where the system font is wanted.
fn system(wanted: bool, query: &str) -> Option<&'static [u8]> {
    if !wanted {
        return None;
    }
    let found = fc_match(query).and_then(|path| file(Path::new(&path)));
    if found.is_none() {
        log::debug!("no font of the system's for {query}: egui's own is used");
    }
    found
}

/// The bytes of a font file, read once for the program's life (a program
/// asks for the same few files again with each surface it opens).
fn file(path: &Path) -> Option<&'static [u8]> {
    type Cache = Mutex<HashMap<PathBuf, Option<&'static [u8]>>>;
    static READ: OnceLock<Cache> = OnceLock::new();
    let mut read = READ.get_or_init(Default::default).lock().ok()?;
    *read.entry(path.to_owned()).or_insert_with(|| {
        std::fs::read(path)
            .map_err(|error| log::debug!("the font {} was not read: {error}", path.display()))
            .ok()
            .map(|bytes| &*Vec::leak(bytes))
    })
}

/// The font file fontconfig picks for `pattern`, if it's a TrueType/OpenType file.
fn fc_match(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .map_err(|error| log::debug!("fc-match did not start: {error}"))
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    let lower = path.to_lowercase();
    (lower.ends_with(".ttf") || lower.ends_with(".otf")).then_some(path)
}

/// Signs that the fonts egui brings lack, and a symbol font has: the
/// power signs, the command key, a star, a check.
const SYMBOLS: &str = "23fb 23fc 23fd 2b58 2318 2605 2714";

/// The system's font for symbols: Segoe UI Symbol on Windows, Apple
/// Symbols on macOS, and on Linux the first font fontconfig knows that has
/// [`SYMBOLS`].
pub(crate) fn symbol_font() -> Option<&'static [u8]> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    let path = PATH.get_or_init(|| {
        if cfg!(windows) {
            let windows = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
            Some(Path::new(&windows).join("Fonts").join("seguisym.ttf"))
        } else if cfg!(target_os = "macos") {
            Some("/System/Library/Fonts/Apple Symbols.ttf".into())
        } else {
            fc_list_first(&format!(":charset={SYMBOLS}")).map(Into::into)
        }
    });
    let bytes = path.as_deref().and_then(file);
    if bytes.is_none() {
        log::debug!("no symbol font of the system's: signs egui lacks stay boxes");
    }
    bytes
}

/// The first TrueType/OpenType file fontconfig lists for `pattern`.
fn fc_list_first(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-list")
        .args(["-f", "%{file}\n", pattern])
        .output()
        .map_err(|error| log::debug!("fc-list did not start: {error}"))
        .ok()?;
    let mut files: Vec<String> = String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .filter(|path| {
            let lower = path.to_lowercase();
            lower.ends_with(".ttf") || lower.ends_with(".otf")
        })
        .map(str::to_owned)
        .collect();
    // (fc-list's order is no order: the same font each run, by its name.)
    files.sort();
    files.into_iter().next()
}

// ---- the fonts the system has ----

/// A font file of the system's.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Installed {
    pub family: String,
    /// "Regular", "Bold Italic", …
    pub style: String,
    pub path: PathBuf,
}

/// The fonts the system has, by family and then style, looked for once:
/// fontconfig's list on Linux, the font folders' files elsewhere (read
/// only as far as their names).
pub fn installed() -> &'static [Installed] {
    static LIST: OnceLock<Vec<Installed>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list = fc_list_all().unwrap_or_else(|| {
            let mut files = Vec::new();
            for folder in font_folders() {
                walk(&folder, &mut files, 3);
            }
            files.iter().filter_map(|path| read_names(path)).collect()
        });
        list.sort_by(|a, b| {
            (a.family.to_lowercase(), style_rank(&a.style), &a.style)
                .cmp(&(b.family.to_lowercase(), style_rank(&b.style), &b.style))
        });
        list.dedup_by(|a, b| a.family == b.family && a.style == b.style);
        list
    })
}

/// The families of [`installed`], each once, with the file of its most
/// ordinary style (Regular where it has one).
pub fn families() -> &'static [Installed] {
    static LIST: OnceLock<Vec<Installed>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list: Vec<Installed> = Vec::new();
        // (Sorted so that a family's most ordinary style comes first.)
        for font in installed() {
            if list.last().is_none_or(|last| last.family != font.family) {
                list.push(font.clone());
            }
        }
        list
    })
}

/// What `path` is, as [`installed`] knows it or its own names say.
pub fn describe(path: &Path) -> Option<Installed> {
    installed()
        .iter()
        .find(|font| font.path == path)
        .cloned()
        .or_else(|| read_names(path))
}

/// The family's name of a choice, as it is shown.
pub fn shown_name(choice: &Choice, default: &str) -> String {
    match choice {
        Choice::Default => default.to_owned(),
        Choice::Builtin => "gevrek's built-in".to_owned(),
        Choice::File(path) => match describe(path) {
            Some(font) if is_regular(&font.style) => font.family,
            Some(font) => format!("{} {}", font.family, font.style),
            None => path
                .file_name()
                .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned()),
        },
    }
}

fn is_regular(style: &str) -> bool {
    style_rank(style) == 0
}

/// How ordinary a style is: the regular ones first, then the plain weights,
/// then the italic ones.
fn style_rank(style: &str) -> u8 {
    let lower = style.to_lowercase();
    if ["regular", "book", "normal", "roman", "plain", ""].contains(&lower.as_str()) {
        0
    } else if lower.contains("italic") || lower.contains("oblique") {
        2
    } else {
        1
    }
}

/// Every font of fontconfig's (the first face of each file), where
/// fontconfig is.
fn fc_list_all() -> Option<Vec<Installed>> {
    if cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("fc-list")
        .args(["-f", "%{family[0]}\t%{style[0]}\t%{index}\t%{file}\n"])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let list: Vec<Installed> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let (family, style, index, path) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            (index == "0" && is_font_file(Path::new(path)) && !family.is_empty()).then(|| {
                Installed {
                    family: family.to_owned(),
                    style: style.to_owned(),
                    path: path.into(),
                }
            })
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

fn font_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if cfg!(windows) {
        let windows = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        folders.push(Path::new(&windows).join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            folders.push(Path::new(&local).join("Microsoft\\Windows\\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        folders.push("/System/Library/Fonts".into());
        folders.push("/Library/Fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            folders.push(Path::new(&home).join("Library/Fonts"));
        }
    } else {
        folders.push("/usr/share/fonts".into());
        folders.push("/usr/local/share/fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            folders.push(Path::new(&home).join(".local/share/fonts"));
        }
    }
    folders
}

fn walk(folder: &Path, files: &mut Vec<PathBuf>, depth: u8) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                walk(&path, files, depth - 1);
            }
        } else if is_font_file(&path) {
            files.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"].contains(&extension.to_lowercase().as_str())
        })
}

/// The family and style a font file names itself by (the first font of a
/// collection), reading no more of it than its table of names.
pub fn read_names(path: &Path) -> Option<Installed> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut read_at = |at: u64, len: usize| -> Option<Vec<u8>> {
        let mut bytes = vec![0; len];
        file.seek(SeekFrom::Start(at)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    };
    let u16_at = |bytes: &[u8], at: usize| -> Option<u16> {
        Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
    };
    let u32_at = |bytes: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let head = read_at(0, 12)?;
    let start = if &head[..4] == b"ttcf" {
        u32_at(&read_at(12, 4)?, 0)? as u64
    } else {
        0
    };
    let offsets = read_at(start, 12)?;
    let tables = u16_at(&offsets, 4)? as usize;
    let directory = read_at(start + 12, tables * 16)?;
    let (at, len) = (0..tables).find_map(|table| {
        let entry = &directory[table * 16..table * 16 + 16];
        (&entry[..4] == b"name").then(|| Some((u32_at(entry, 8)?, u32_at(entry, 12)?)))?
    })?;
    if len > 1 << 20 {
        return None;
    }
    let names = read_at(at as u64, len as usize)?;
    let count = u16_at(&names, 2)? as usize;
    let strings = u16_at(&names, 4)? as usize;
    // The best name of each kind: Windows' in US English, then Windows' in
    // any language, then the Mac's (in Roman letters).
    let mut best: HashMap<u16, (u8, String)> = HashMap::new();
    for record in 0..count {
        let at = 6 + record * 12;
        let (platform, encoding, language, id, len, offset) = (
            u16_at(&names, at)?,
            u16_at(&names, at + 2)?,
            u16_at(&names, at + 4)?,
            u16_at(&names, at + 6)?,
            u16_at(&names, at + 8)? as usize,
            u16_at(&names, at + 10)? as usize,
        );
        if ![1, 2, 16, 17].contains(&id) {
            continue;
        }
        let Some(bytes) = names.get(strings + offset..strings + offset + len) else {
            continue;
        };
        let (rank, text) = match (platform, encoding) {
            (3, 0 | 1) | (0, _) => {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_be_bytes(*pair))
                    .collect();
                let rank = if language == 0x409 || platform == 0 { 3 } else { 2 };
                (rank, String::from_utf16_lossy(&units))
            }
            (1, 0) => (1, bytes.iter().map(|&byte| byte as char).collect()),
            _ => continue,
        };
        if best.get(&id).is_none_or(|(had, _)| *had < rank) {
            best.insert(id, (rank, text));
        }
    }
    let mut name = |ids: [u16; 2]| {
        ids.iter()
            .find_map(|id| best.remove(id).map(|(_, text)| text.trim().to_owned()))
            .filter(|text| !text.is_empty())
    };
    let family = name([16, 1])?;
    let style = name([17, 2]).unwrap_or_else(|| "Regular".to_owned());
    Some(Installed {
        family,
        style,
        path: path.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lays `text` out in the proportional font of a context whose fonts
    /// are those of `area`, and says which glyphs it came to.
    fn widths(settings: &Settings, area: Option<Area>, text: &str) -> f32 {
        let context = egui::Context::default();
        install(&context, settings, area);
        let mut output = context.run_ui(Default::default(), |_| {});
        let wide = context.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    text.to_owned(),
                    egui::FontId::proportional(20.0),
                    egui::Color32::WHITE,
                )
                .size()
                .x
        });
        // (No renderer takes the font texture in this test.)
        output.textures_delta.clear();
        wide
    }

    #[test]
    fn an_area_with_a_font_of_its_own_is_written_in_it_and_the_others_are_not() {
        // A font of the system's other than the built-in one.
        let Some(other) = families()
            .iter()
            .find(|font| !font.path.to_string_lossy().contains("emoji"))
        else {
            return;
        };
        let mut settings = Settings {
            system_font: false,
            ..Settings::default()
        };
        settings.fonts.set_area(Area::Panel, Choice::File(other.path.clone()));
        let words = "The quick brown fox jumps";
        let own = widths(&settings, Some(Area::Panel), words);
        let others = widths(&settings, Some(Area::Launcher), words);
        let interface = widths(&settings, None, words);
        assert_eq!(others, interface);
        assert_ne!(own, interface, "{} looks as the built-in font", other.family);
    }

    #[test]
    fn the_fonts_of_the_system_are_listed_by_their_names() {
        let list = installed();
        if list.is_empty() {
            return;
        }
        assert!(list.iter().all(|font| !font.family.is_empty()));
        if cfg!(windows) {
            let segoe = families()
                .iter()
                .find(|font| font.family == "Segoe UI")
                .expect("Segoe UI is listed");
            assert_eq!(segoe.style, "Regular");
            assert_eq!(shown_name(&Choice::File(segoe.path.clone()), ""), "Segoe UI");
        }
    }

    #[test]
    fn fonts_survive_the_file_and_an_area_left_default_is_not_written() {
        let mut fonts = Fonts::default();
        fonts.set_area(Area::Osd, Choice::Builtin);
        fonts.set_area(Area::Zones, Choice::Default);
        let lines = fonts.lines();
        assert!(!lines.contains("font-zones"));
        let mut read = Fonts::default();
        for line in lines.lines() {
            let (key, value) = line.split_once('=').unwrap();
            assert!(read.read(key.trim(), value.trim()));
        }
        assert_eq!(read, fonts);
        assert!(!read.read("radius", "3"));
    }
}
