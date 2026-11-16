//! The applications of Windows: what the Start menu lists.
//!
//! Two sources, as the Start menu has them. The shortcuts (`*.lnk`) under
//! the `Programs` folder of the user and of all users, each resolved by
//! the shell to what it starts; and the packaged applications (from the
//! Store, MSIX), which have no shortcut and are listed by the shell's
//! folder of applications (`shell:AppsFolder`) under the identifier they
//! are started by.
//!
//! What a launcher has no use for is left out: uninstallers, shortcuts to
//! a manual or a web page or a folder, shortcuts whose program is gone,
//! and a second shortcut to the same program with the same arguments.
//!
//! An application is started by handing the shell its shortcut, or its
//! place in the folder of applications, to open ([`opened_by`]): what the
//! Start menu does. So a shortcut that says "as administrator", "in this
//! folder" or "minimised" is honoured, and a packaged application is
//! started by the system and not as a child of ours.
//!
//! What is decided here (which shortcut is an application, which is a
//! duplicate, how an identifier reads) asks nothing of Windows and is
//! tested on every system; only the reading and the starting are
//! Windows' own.

use crate::apps::App;
use std::collections::HashSet;
use std::path::PathBuf;

/// What the shell's folder of applications is called when something in
/// it is named: `shell:AppsFolder\<identifier>`.
pub const APPS_FOLDER: &str = "shell:AppsFolder\\";

/// A shortcut of the Start menu, as the shell reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// The shortcut's file.
    pub file: PathBuf,
    /// Its place under `Programs`, folders included
    /// (`Accessories\Paint.lnk`).
    pub relative: String,
    /// The name the shell shows for it (in the user's language where the
    /// folder says one).
    pub name: String,
    /// The program it starts; empty where it points at something that is
    /// no file (a place in the shell, an installer's own kind of link).
    pub target: String,
    pub arguments: String,
    pub directory: String,
    pub description: String,
    /// Where its icon is, when the shortcut names one, and which of the
    /// icons there.
    pub icon: String,
    pub icon_index: i32,
}

/// A packaged application: its identifier (the application user model
/// id, `Family_publisher!App`) and the name the shell shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packaged {
    pub id: String,
    pub name: String,
}

/// What a shortcut's target is on the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Missing,
    File,
    Folder,
}

/// The name an application from a shortcut is known by: its place under
/// `Programs` without the ending, in small letters, with `/` between the
/// folders. It is the same whatever language the names are shown in.
pub fn id_of(relative: &str) -> String {
    let lower = relative.to_lowercase().replace('\\', "/");
    lower
        .strip_suffix(".lnk")
        .map(str::to_string)
        .unwrap_or(lower)
}

/// The last part of a path without its ending, in small letters
/// (`C:\Windows\System32\charmap.exe` is `charmap`).
fn stem_of(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file,
    };
    stem.to_lowercase()
}

/// The ending of a path's last part, in small letters; empty: none.
fn ending_of(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    file.rsplit_once('.')
        .map(|(_, ending)| ending.to_lowercase())
        .unwrap_or_default()
}

/// Whether a shortcut removes a program rather than starting one. A
/// program whose business is removing others ("… Uninstaller") is not
/// meant: only the word itself, the names installers give their
/// removers, and the system's installer asked to remove.
pub fn is_uninstaller(name: &str, target: &str, arguments: &str) -> bool {
    let name = name.to_lowercase();
    if name
        .split(|letter: char| !letter.is_alphanumeric())
        .any(|word| word == "uninstall" || word == "kaldır")
    {
        return true;
    }
    let stem = stem_of(target);
    if stem == "uninstall" || stem == "uninst" || stem.starts_with("unins0") {
        return true;
    }
    stem == "msiexec"
        && arguments
            .to_lowercase()
            .split_whitespace()
            .any(|word| word.starts_with("/x") || word.starts_with("-x"))
}

/// Whether a shortcut's target is something to read rather than a
/// program: a manual, a licence, a web page.
pub fn is_document(target: &str) -> bool {
    matches!(
        ending_of(target).as_str(),
        "txt"
            | "chm"
            | "hlp"
            | "htm"
            | "html"
            | "url"
            | "website"
            | "pdf"
            | "rtf"
            | "md"
            | "log"
            | "ini"
            | "xml"
            | "doc"
            | "docx"
    )
}

/// Whether an identifier of the folder of applications is a packaged
/// application's: a package family (a name, `_`, thirteen letters and
/// digits for the publisher), `!`, the application in the package. The
/// folder lists the programs with shortcuts too, under other names (a
/// path, or a name the program gave itself); those come from their
/// shortcuts.
pub fn is_packaged_id(id: &str) -> bool {
    let Some((family, app)) = id.split_once('!') else {
        return false;
    };
    let Some((name, publisher)) = family.rsplit_once('_') else {
        return false;
    };
    !app.is_empty()
        && !app.contains('!')
        && !name.is_empty()
        && publisher.len() == 13
        && publisher
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric())
}

/// The arguments of a command line, as Windows programs take them
/// apart: blanks separate, `"` holds blanks together, and a `\` only
/// means something in front of a `"` (two are one, an odd one makes the
/// `"` a plain letter).
pub fn split_arguments(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut has_word = false;
    let mut quoted = false;
    let mut slashes = 0usize;
    for letter in line.chars() {
        if letter == '\\' {
            slashes += 1;
            continue;
        }
        if letter == '"' {
            word.push_str(&"\\".repeat(slashes / 2));
            if slashes % 2 == 1 {
                word.push('"');
            } else {
                quoted = !quoted;
            }
            has_word = true;
            slashes = 0;
            continue;
        }
        if slashes > 0 {
            word.push_str(&"\\".repeat(slashes));
            has_word = true;
            slashes = 0;
        }
        if (letter == ' ' || letter == '\t') && !quoted {
            if has_word {
                words.push(std::mem::take(&mut word));
                has_word = false;
            }
            continue;
        }
        word.push(letter);
        has_word = true;
    }
    if slashes > 0 {
        word.push_str(&"\\".repeat(slashes));
        has_word = true;
    }
    if has_word {
        words.push(word);
    }
    words
}

/// The application a shortcut stands for; `None` where it is none to
/// list (see the top of this file). `found` says what a path is on the
/// disk.
pub fn app_of_shortcut(shortcut: &Shortcut, found: &dyn Fn(&str) -> Found) -> Option<App> {
    let name = shortcut.name.trim();
    if name.is_empty() || is_uninstaller(name, &shortcut.target, &shortcut.arguments) {
        return None;
    }
    let target = shortcut.target.trim();
    if !target.is_empty() {
        if is_document(target) {
            return None;
        }
        // A program that is gone, or a folder: nothing to start. (A name
        // without a folder is looked for by Windows when it is started.)
        let whole = target.contains(':') || target.starts_with('\\');
        if whole && found(target) != Found::File {
            return None;
        }
    }
    let file = shortcut.file.to_string_lossy().into_owned();
    let command = if target.is_empty() {
        // No program to name: the shell opens the shortcut.
        vec!["explorer.exe".to_string(), file.clone()]
    } else {
        let mut command = vec![target.to_string()];
        command.extend(split_arguments(&shortcut.arguments));
        command
    };
    // The program's own name finds it too ("winword" for Word).
    let stem = stem_of(target);
    let keywords = if stem.is_empty() || stem == name.to_lowercase() {
        Vec::new()
    } else {
        vec![stem]
    };
    let description = shortcut.description.trim();
    // A description that only names a text in a library was not found by
    // the shell; and one that repeats the name says nothing.
    let comment = if description.starts_with('@') || description.eq_ignore_ascii_case(name) {
        String::new()
    } else {
        description.to_string()
    };
    Some(App {
        actions: Vec::new(),
        id: id_of(&shortcut.relative),
        name: name.to_string(),
        generic_name: String::new(),
        comment,
        keywords,
        // The shell draws a shortcut's icon from the shortcut.
        icon: file,
        command,
        terminal: false,
        dbus_activatable: false,
        directory: Some(shortcut.directory.trim().to_string()).filter(|path| !path.is_empty()),
        entry: shortcut.file.clone(),
    })
}

/// The application a packaged entry stands for.
pub fn app_of_packaged(packaged: &Packaged) -> Option<App> {
    let name = packaged.name.trim();
    // A name the shell could not look up is left as the place it would
    // have been looked up at.
    if name.is_empty() || name.starts_with("ms-resource:") || !is_packaged_id(&packaged.id) {
        return None;
    }
    let place = format!("{APPS_FOLDER}{}", packaged.id);
    Some(App {
        actions: Vec::new(),
        id: packaged.id.clone(),
        name: name.to_string(),
        generic_name: String::new(),
        comment: String::new(),
        keywords: Vec::new(),
        icon: place.clone(),
        command: vec!["explorer.exe".to_string(), place],
        terminal: false,
        dbus_activatable: false,
        directory: None,
        entry: PathBuf::new(),
    })
}

/// The applications of what was read, by name. `shortcuts` are in the
/// order they count: the user's before those of all users. Of two
/// shortcuts at the same place under `Programs` the first stands; so it
/// does of two that start the same program with the same arguments, and
/// of two with the same name that name no program.
pub fn gather(
    shortcuts: &[Shortcut],
    packaged: &[Packaged],
    found: &dyn Fn(&str) -> Found,
) -> Vec<App> {
    let mut ids = HashSet::new();
    let mut starts = HashSet::new();
    let mut apps = Vec::new();
    for shortcut in shortcuts {
        // Also when the user's shortcut is none to list: the one of all
        // users at the same place does not come in its stead.
        if !ids.insert(id_of(&shortcut.relative)) {
            continue;
        }
        let Some(app) = app_of_shortcut(shortcut, found) else {
            continue;
        };
        let target = shortcut.target.trim().to_lowercase();
        let same = if target.is_empty() {
            (String::new(), app.name.to_lowercase())
        } else {
            (target, shortcut.arguments.trim().to_lowercase())
        };
        if !starts.insert(same) {
            continue;
        }
        apps.push(app);
    }
    for one in packaged {
        if !ids.insert(one.id.to_lowercase()) {
            continue;
        }
        if let Some(app) = app_of_packaged(one) {
            apps.push(app);
        }
    }
    apps.sort_by_key(|one| one.name.to_lowercase());
    apps
}

/// Which kind of program a file is, read from its first bytes: 2 for one
/// with windows of its own, 3 for one that runs in a terminal. `None` for
/// what is no program of Windows.
pub fn subsystem_of(header: &[u8]) -> Option<u16> {
    if header.get(..2)? != b"MZ" {
        return None;
    }
    let at = u32::from_le_bytes(header.get(0x3C..0x40)?.try_into().ok()?) as usize;
    if header.get(at..at + 4)? != b"PE\0\0" {
        return None;
    }
    // (After the mark and the file's header: the word for the subsystem
    // is at the same place in programs of 32 and of 64 bits.)
    let word = header.get(at + 0x5C..at + 0x5E)?;
    Some(u16::from_le_bytes([word[0], word[1]]))
}

/// The name a program of the person's own folder of programs is listed
/// by, where it is one to list: a program with windows of its own (one
/// for a terminal is started from a terminal), not a copy kept from before
/// an update (`name.old.exe`), and of gevrek's own programs only the
/// settings.
pub fn own_program_name(file_name: &str, subsystem: Option<u16>) -> Option<String> {
    let lower = file_name.to_lowercase();
    let stem = lower.strip_suffix(".exe")?;
    if stem == "gevrek-studio" {
        return Some("gevrek studio".to_string());
    }
    if stem.starts_with("gevrek") || stem.ends_with(".old") || stem.ends_with(".bak") {
        return None;
    }
    (subsystem == Some(2)).then(|| file_name[..stem.len()].to_string())
}

/// The application a program of the person's own folder stands for.
pub fn app_of_program(file: &std::path::Path, name: &str) -> App {
    let path = file.to_string_lossy().into_owned();
    // The file's own name finds it too, where it is listed by another
    // ("gevrek-studio" for gevrek studio, which "settings" finds as well).
    let stem = stem_of(&path);
    let mut keywords = Vec::new();
    if !stem.is_empty() && stem != name.to_lowercase() {
        keywords.push(stem.clone());
    }
    let settings = stem == "gevrek-studio";
    if settings {
        keywords.push("settings".to_string());
    }
    App {
        actions: Vec::new(),
        id: format!("program:{}", name.to_lowercase()),
        name: name.to_string(),
        generic_name: if settings {
            "Settings".to_string()
        } else {
            String::new()
        },
        comment: if settings {
            "The settings of gevrek".to_string()
        } else {
            String::new()
        },
        keywords,
        // The shell draws a program's icon from the program.
        icon: path.clone(),
        command: vec![path],
        terminal: false,
        dbus_activatable: false,
        directory: None,
        // Started as a shortcut is: the shell opens the file.
        entry: file.to_path_buf(),
    }
}

/// The applications with the person's own programs among them. A program
/// the Start menu already has (by its name, by the name of the program a
/// shortcut starts, or by the very file) is not listed a second time.
pub fn with_own_programs(mut apps: Vec<App>, programs: Vec<App>) -> Vec<App> {
    for program in programs {
        let name = program.name.to_lowercase();
        let known = apps.iter().any(|app| {
            app.name.to_lowercase() == name
                || app.keywords.contains(&name)
                || app
                    .command
                    .first()
                    .is_some_and(|start| start.eq_ignore_ascii_case(&program.command[0]))
        });
        if !known {
            apps.push(program);
        }
    }
    apps.sort_by_key(|one| one.name.to_lowercase());
    apps
}

/// What the shell is given to open, to start an application: its
/// shortcut, or its place in the folder of applications.
pub fn opened_by(app: &App) -> String {
    if app.entry.as_os_str().is_empty() {
        format!("{APPS_FOLDER}{}", app.id)
    } else {
        app.entry.to_string_lossy().into_owned()
    }
}

#[cfg(windows)]
pub use shell::{installed, programs_folders, read_packaged, read_shortcuts, start, start_as};

#[cfg(windows)]
mod shell {
    use super::{
        app_of_program, gather, opened_by, own_program_name, subsystem_of, with_own_programs, App,
        Found, Packaged, Shortcut,
    };
    use crate::com::Com;
    use std::path::{Path, PathBuf};
    use windows::core::{Interface, GUID, HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
    };
    use windows::Win32::System::Threading::GetProcessId;
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonPrograms, FOLDERID_Programs,
        IEnumShellItems, IShellItem, IShellLinkW, SHGetFileInfoW, SHGetKnownFolderItem,
        SHGetKnownFolderPath, ShellExecuteExW, ShellLink, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI,
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHFILEINFOW,
        SHGFI_DISPLAYNAME, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// A text Windows wrote into a buffer, up to its zero.
    fn text_of(buffer: &[u16]) -> String {
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..length])
    }

    /// A text Windows made room for itself; the room is given back.
    unsafe fn taken(text: PWSTR) -> String {
        if text.is_null() {
            return String::new();
        }
        let read = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        read
    }

    fn folder(id: &GUID) -> Option<PathBuf> {
        let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
        let path = unsafe { taken(path) };
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    /// The `Programs` folders of the Start menu: the user's, then the one
    /// of all users.
    pub fn programs_folders() -> Vec<PathBuf> {
        [&FOLDERID_Programs, &FOLDERID_CommonPrograms]
            .into_iter()
            .filter_map(folder)
            .collect()
    }

    /// The shortcuts below `folder`, by their place under `root`.
    fn collect(root: &Path, folder: &Path, found: &mut Vec<(String, PathBuf)>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 6 {
                    collect(root, &path, found, depth + 1);
                }
                continue;
            }
            let is_shortcut = path
                .extension()
                .and_then(|ending| ending.to_str())
                .is_some_and(|ending| ending.eq_ignore_ascii_case("lnk"));
            if !is_shortcut {
                continue;
            }
            if let Ok(relative) = path.strip_prefix(root) {
                found.push((relative.to_string_lossy().into_owned(), path));
            }
        }
    }

    /// The name the shell shows for a file: without the ending, and in
    /// the user's language where the folder gives one.
    fn shown_name(file: &HSTRING) -> String {
        let mut info = SHFILEINFOW::default();
        let answered = unsafe {
            SHGetFileInfoW(
                PCWSTR(file.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_DISPLAYNAME,
            )
        };
        if answered == 0 {
            return String::new();
        }
        text_of(&info.szDisplayName)
    }

    /// One shortcut, as the shell reads it. It is read as it is written:
    /// the shell is not asked to go looking for a target that moved,
    /// which can take long and can ask the user.
    fn read_shortcut(relative: &str, file: &Path) -> Option<Shortcut> {
        let wide = HSTRING::from(file.as_os_str());
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let stored: IPersistFile = link.cast().ok()?;
            stored.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
            let mut target = [0u16; 1024];
            let _ = link.GetPath(&mut target, std::ptr::null_mut(), 0);
            let mut arguments = [0u16; 4096];
            let _ = link.GetArguments(&mut arguments);
            let mut directory = [0u16; 1024];
            let _ = link.GetWorkingDirectory(&mut directory);
            let mut description = [0u16; 2048];
            let _ = link.GetDescription(&mut description);
            let mut icon = [0u16; 1024];
            let mut icon_index = 0i32;
            let _ = link.GetIconLocation(&mut icon, &mut icon_index);
            let mut name = shown_name(&wide);
            if name.is_empty() {
                name = file
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            Some(Shortcut {
                file: file.to_path_buf(),
                relative: relative.to_string(),
                name,
                target: text_of(&target),
                arguments: text_of(&arguments),
                directory: text_of(&directory),
                description: text_of(&description),
                icon: text_of(&icon),
                icon_index,
            })
        }
    }

    /// Every shortcut of the Start menu, the user's first.
    pub fn read_shortcuts() -> Vec<Shortcut> {
        let _com = Com::new();
        let mut shortcuts = Vec::new();
        for root in programs_folders() {
            let mut files = Vec::new();
            collect(&root, &root, &mut files, 0);
            files.sort();
            shortcuts.extend(
                files
                    .iter()
                    .filter_map(|(relative, file)| read_shortcut(relative, file)),
            );
        }
        shortcuts
    }

    fn name_of(item: &IShellItem, which: SIGDN) -> String {
        unsafe { item.GetDisplayName(which).map(|text| taken(text)) }.unwrap_or_default()
    }

    /// Everything in the shell's folder of applications, by the
    /// identifier it is started by. The programs with shortcuts are in
    /// it too; [`gather`] keeps the packaged ones.
    pub fn read_packaged() -> Vec<Packaged> {
        let _com = Com::new();
        let mut found = Vec::new();
        unsafe {
            let Ok(folder) =
                SHGetKnownFolderItem::<_, IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
            else {
                return found;
            };
            let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems)
            else {
                return found;
            };
            loop {
                let mut one = [None];
                let mut fetched = 0u32;
                if items.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(item) = one[0].take() else {
                    break;
                };
                found.push(Packaged {
                    id: name_of(&item, SIGDN_PARENTRELATIVEPARSING),
                    name: name_of(&item, SIGDN_NORMALDISPLAY),
                });
            }
        }
        found
    }

    /// The applications of this machine's Start menu.
    pub fn installed() -> Vec<App> {
        let found = |path: &str| match std::fs::metadata(path) {
            Ok(what) if what.is_dir() => Found::Folder,
            Ok(_) => Found::File,
            Err(_) => Found::Missing,
        };
        with_own_programs(
            gather(&read_shortcuts(), &read_packaged(), &found),
            own_programs(),
        )
    }

    /// The programs of the person's own folder of programs
    /// (`%USERPROFILE%\.local\bin`): what they built or put there
    /// themselves, which has no shortcut in the Start menu and would be
    /// found by nobody.
    fn own_programs() -> Vec<App> {
        use std::io::Read;
        let Some(folder) = dirs::home_dir().map(|home| home.join(".local").join("bin")) else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let file = entry.path();
                let file_name = file.file_name()?.to_str()?.to_string();
                if !file_name.to_lowercase().ends_with(".exe") {
                    return None;
                }
                // The first bytes say what kind of program it is.
                let mut header = vec![0u8; 1024];
                let read = std::fs::File::open(&file).ok()?.read(&mut header).ok()?;
                header.truncate(read);
                let name = own_program_name(&file_name, subsystem_of(&header))?;
                Some(app_of_program(&file, &name))
            })
            .collect()
    }

    /// Start an application as the Start menu does: the shell opens its
    /// shortcut, or its place in the folder of applications. The answer
    /// is the number of the process that was started, where the shell
    /// says (it does not for a packaged application, which the system
    /// starts, nor when a running program took the request).
    ///
    /// The program is no child that ends with this one, and it has the
    /// rights of this program: one that is run as administrator starts
    /// its programs so too. What goes wrong is answered, not shown: who
    /// asked may be a surface above every window, where a dialog of the
    /// shell would lie hidden.
    pub fn start(app: &App) -> std::io::Result<Option<u32>> {
        start_as(app, false)
    }

    /// As `start`, with an administrator's rights where `elevated`:
    /// Windows asks the person first (its own question, on its own
    /// screen), and the program starts when they agree. A packaged
    /// application cannot be started so, and one whose question was
    /// answered with no is not started: both are an error to who asked.
    pub fn start_as(app: &App, elevated: bool) -> std::io::Result<Option<u32>> {
        let _com = Com::new();
        let file = HSTRING::from(opened_by(app));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            // Done when the call returns: the thread that asks may have
            // no loop of messages to finish it in.
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: if elevated {
                windows::core::w!("runas")
            } else {
                PCWSTR::null()
            },
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe {
            ShellExecuteExW(&mut info).map_err(std::io::Error::other)?;
            if info.hProcess.is_invalid() {
                return Ok(None);
            }
            let process = GetProcessId(info.hProcess);
            let _ = CloseHandle(info.hProcess);
            Ok((process != 0).then_some(process))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first bytes of a program of this kind.
    fn header(subsystem: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x200];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x80 + 0x5C..0x80 + 0x5E].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    #[test]
    fn the_persons_own_programs_with_windows_are_listed() {
        assert_eq!(subsystem_of(&header(2)), Some(2));
        assert_eq!(subsystem_of(&header(3)), Some(3));
        assert_eq!(subsystem_of(b"not a program"), None);
        assert_eq!(subsystem_of(&header(2)[..0x90]), None);
        // One with windows of its own, by its file's name as it is written.
        assert_eq!(own_program_name("Peek.exe", Some(2)), Some("Peek".to_string()));
        // One for a terminal, a copy from before an update, another file.
        assert_eq!(own_program_name("soyuz-tui.exe", Some(3)), None);
        assert_eq!(own_program_name("peek.old.exe", Some(2)), None);
        assert_eq!(own_program_name("notes.txt", Some(2)), None);
        // Of gevrek's own programs only the settings.
        assert_eq!(own_program_name("gevrek-wm.exe", Some(3)), None);
        assert_eq!(
            own_program_name("gevrek-studio.exe", Some(3)),
            Some("gevrek studio".to_string())
        );
    }

    #[test]
    fn a_program_the_start_menu_has_is_not_listed_twice() {
        let found = |_: &str| Found::File;
        let menu = gather(
            &[shortcut("Soyuz.lnk", "Soyuz", "C:\\Programs\\Soyuz\\soyuz.exe", "")],
            &[],
            &found,
        );
        let own = |name: &str| app_of_program(std::path::Path::new(&format!("C:\\bin\\{name}.exe")), name);
        let all = with_own_programs(menu, vec![own("soyuz"), own("peek"), own("walle")]);
        let names: Vec<&str> = all.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["peek", "Soyuz", "walle"]);
        // Started as a shortcut is: the shell opens the program's file.
        let peek = &all[0];
        assert_eq!(opened_by(peek), "C:\\bin\\peek.exe");
        assert_eq!(peek.id, "program:peek");
        // The settings are found by their file's name and by what they are.
        let studio = app_of_program(
            std::path::Path::new("C:\\bin\\gevrek-studio.exe"),
            "gevrek studio",
        );
        for typed in ["gevrek", "studio", "gevrek-studio", "settings"] {
            assert!(crate::apps::score(&studio, typed).is_some(), "{typed}");
        }
    }

    fn shortcut(relative: &str, name: &str, target: &str, arguments: &str) -> Shortcut {
        Shortcut {
            file: PathBuf::from(format!("C:\\Start\\{relative}")),
            relative: relative.to_string(),
            name: name.to_string(),
            target: target.to_string(),
            arguments: arguments.to_string(),
            ..Default::default()
        }
    }

    /// Everything is a file but what is named here.
    fn found(path: &str) -> Found {
        if path.contains("gone") {
            Found::Missing
        } else if path.ends_with("\\Samples") {
            Found::Folder
        } else {
            Found::File
        }
    }

    #[test]
    fn a_shortcut_is_read_as_an_application() {
        let mut paint = shortcut(
            "Accessories\\Paint.lnk",
            "Paint",
            "C:\\Windows\\System32\\mspaint.exe",
            "/new \"C:\\My Pictures\\a b.png\"",
        );
        paint.directory = "C:\\Users\\me".to_string();
        paint.description = "Draw and edit pictures".to_string();
        let app = app_of_shortcut(&paint, &found).expect("an application");
        assert_eq!(app.id, "accessories/paint");
        assert_eq!(app.name, "Paint");
        assert_eq!(app.comment, "Draw and edit pictures");
        assert_eq!(
            app.command,
            [
                "C:\\Windows\\System32\\mspaint.exe",
                "/new",
                "C:\\My Pictures\\a b.png"
            ]
        );
        // The program's own name finds it.
        assert_eq!(app.keywords, ["mspaint"]);
        assert_eq!(app.directory.as_deref(), Some("C:\\Users\\me"));
        assert_eq!(app.icon, "C:\\Start\\Accessories\\Paint.lnk");
        assert!(!app.terminal);
        // The same word twice says nothing.
        let charmap = shortcut("Charmap.lnk", "Charmap", "C:\\Windows\\charmap.exe", "");
        let app = app_of_shortcut(&charmap, &found).expect("an application");
        assert!(app.keywords.is_empty());
        assert_eq!(app.directory, None);
    }

    #[test]
    fn a_description_that_says_nothing_is_left_out() {
        let mut tool = shortcut("Tool.lnk", "Tool", "C:\\tool.exe", "");
        tool.description = "@%SystemRoot%\\system32\\shell32.dll,-22067".to_string();
        assert_eq!(app_of_shortcut(&tool, &found).expect("one").comment, "");
        tool.description = "tool".to_string();
        assert_eq!(app_of_shortcut(&tool, &found).expect("one").comment, "");
    }

    #[test]
    fn an_uninstaller_is_no_application_but_a_program_that_uninstalls_is() {
        assert!(is_uninstaller(
            "Uninstall Thing",
            "C:\\thing\\setup.exe",
            ""
        ));
        assert!(is_uninstaller("Thing'i Kaldır", "C:\\thing\\setup.exe", ""));
        assert!(is_uninstaller("Thing", "C:\\thing\\unins000.exe", ""));
        assert!(is_uninstaller("Thing", "C:\\thing\\Uninstall.exe", ""));
        assert!(is_uninstaller(
            "Remove Thing",
            "C:\\Windows\\System32\\msiexec.exe",
            "/x {1234}"
        ));
        assert!(!is_uninstaller(
            "Thing Setup",
            "C:\\Windows\\System32\\msiexec.exe",
            "/i thing.msi"
        ));
        assert!(!is_uninstaller(
            "Great Uninstaller",
            "C:\\great\\GreatUnin.exe",
            ""
        ));
        assert!(!is_uninstaller("Paint", "C:\\Windows\\mspaint.exe", ""));
        let remover = shortcut("Thing\\Uninstall.lnk", "Uninstall", "C:\\thing\\u.exe", "");
        assert!(app_of_shortcut(&remover, &found).is_none());
    }

    #[test]
    fn what_is_to_read_or_gone_or_a_folder_is_no_application() {
        let none =
            |target: &str| app_of_shortcut(&shortcut("X.lnk", "X", target, ""), &found).is_none();
        assert!(none("C:\\thing\\Manual.PDF"));
        assert!(none("C:\\thing\\readme.txt"));
        assert!(none("C:\\thing\\Website.url"));
        assert!(none("C:\\gone\\thing.exe"));
        assert!(none("C:\\thing\\Samples"));
        assert!(!none("C:\\thing\\thing.exe"));
        // A bare name is Windows' to find when it is started.
        assert!(!none("gone.exe"));
        // No name: nothing to list.
        assert!(app_of_shortcut(&shortcut("X.lnk", " ", "C:\\x.exe", ""), &found).is_none());
    }

    #[test]
    fn a_shortcut_to_no_file_is_opened_by_the_shell() {
        let tools = shortcut("Windows Tools.lnk", "Windows Tools", "", "");
        let app = app_of_shortcut(&tools, &found).expect("an application");
        assert_eq!(
            app.command,
            ["explorer.exe", "C:\\Start\\Windows Tools.lnk"]
        );
        assert!(app.keywords.is_empty());
        assert_eq!(opened_by(&app), "C:\\Start\\Windows Tools.lnk");
    }

    #[test]
    fn the_users_shortcut_stands_and_a_second_to_the_same_program_is_dropped() {
        let shortcuts = vec![
            // The user's.
            shortcut("Editor.lnk", "My Editor", "C:\\mine\\editor.exe", ""),
            shortcut("Hidden\\Uninstall.lnk", "Uninstall", "C:\\u.exe", ""),
            shortcut("Run.lnk", "Run", "", ""),
            // All users'.
            shortcut("Editor.lnk", "Editor", "C:\\all\\editor.exe", ""),
            // At the place of the user's uninstaller: does not come back.
            shortcut("Hidden\\Uninstall.lnk", "Thing", "C:\\thing.exe", ""),
            shortcut(
                "Tools\\Editor again.lnk",
                "Editor again",
                "c:\\MINE\\editor.exe",
                "",
            ),
            shortcut(
                "Tools\\Editor safe.lnk",
                "Editor (safe)",
                "C:\\mine\\editor.exe",
                "/safe",
            ),
            shortcut("Other\\Run.lnk", "run", "", ""),
            shortcut("Browser.lnk", "browser", "C:\\b\\browser.exe", ""),
        ];
        let packaged = vec![
            Packaged {
                id: "Maker.Notes_8wekyb3d8bbwe!App".to_string(),
                name: "Notes".to_string(),
            },
            // Listed twice by the shell.
            Packaged {
                id: "Maker.Notes_8wekyb3d8bbwe!App".to_string(),
                name: "Notes".to_string(),
            },
            // A program with a shortcut, as the folder names it.
            Packaged {
                id: "{6D809377-6AF0-444B-8957-A3773F02200E}\\b\\browser.exe".to_string(),
                name: "browser".to_string(),
            },
            Packaged {
                id: "Maker.Odd_8wekyb3d8bbwe!App".to_string(),
                name: "ms-resource:AppName".to_string(),
            },
        ];
        let apps = gather(&shortcuts, &packaged, &found);
        let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(
            names,
            ["browser", "Editor (safe)", "My Editor", "Notes", "Run"]
        );
        let notes = &apps[3];
        assert_eq!(notes.id, "Maker.Notes_8wekyb3d8bbwe!App");
        assert_eq!(
            notes.icon,
            "shell:AppsFolder\\Maker.Notes_8wekyb3d8bbwe!App"
        );
        assert_eq!(
            notes.command,
            [
                "explorer.exe",
                "shell:AppsFolder\\Maker.Notes_8wekyb3d8bbwe!App"
            ]
        );
    }

    #[test]
    fn an_application_is_started_by_what_the_shell_opens() {
        let paint = app_of_shortcut(
            &shortcut("Accessories\\Paint.lnk", "Paint", "C:\\mspaint.exe", "/new"),
            &found,
        )
        .expect("an application");
        // The shortcut itself, which carries the arguments and the rest.
        assert_eq!(opened_by(&paint), "C:\\Start\\Accessories\\Paint.lnk");
        let notes = app_of_packaged(&Packaged {
            id: "Maker.Notes_8wekyb3d8bbwe!App".to_string(),
            name: "Notes".to_string(),
        })
        .expect("an application");
        assert_eq!(
            opened_by(&notes),
            "shell:AppsFolder\\Maker.Notes_8wekyb3d8bbwe!App"
        );
    }

    #[test]
    fn a_packaged_application_is_known_by_the_shape_of_its_identifier() {
        assert!(is_packaged_id("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"));
        assert!(is_packaged_id("Maker.Thing_1a2b3c4d5e6f7!Maker.Thing.Main"));
        assert!(!is_packaged_id("MSEdge"));
        assert!(!is_packaged_id(
            "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\charmap.exe"
        ));
        assert!(!is_packaged_id("Maker.Thing_short!App"));
        assert!(!is_packaged_id("Maker.Thing_8wekyb3d8bbwe!"));
        assert!(!is_packaged_id("NoPublisher!App"));
    }

    #[test]
    fn arguments_are_taken_apart_as_windows_programs_do() {
        assert_eq!(split_arguments("/a  /b\t/c"), ["/a", "/b", "/c"]);
        assert_eq!(
            split_arguments("--open \"C:\\Program Files\\x y.txt\" z"),
            ["--open", "C:\\Program Files\\x y.txt", "z"]
        );
        // A `\` is itself but in front of a `"`.
        assert_eq!(split_arguments("C:\\a\\b"), ["C:\\a\\b"]);
        assert_eq!(split_arguments("\"C:\\dir\\\\\" x"), ["C:\\dir\\", "x"]);
        assert_eq!(split_arguments("say \\\"hi\\\""), ["say", "\"hi\""]);
        assert_eq!(split_arguments("\"\""), [""]);
        assert!(split_arguments("   ").is_empty());
    }

    #[test]
    fn a_shortcuts_place_is_its_name_whatever_the_letters() {
        assert_eq!(id_of("Accessories\\Paint.lnk"), "accessories/paint");
        assert_eq!(id_of("Paint.LNK"), "paint");
        assert_eq!(id_of("Odd"), "odd");
    }
}
