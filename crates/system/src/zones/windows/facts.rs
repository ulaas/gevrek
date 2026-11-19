//! What can be said of an item without asking the system: which kind it
//! is by what the shell says of it, the type a rule may ask for, times as
//! the core counts them. Everything here is plain arithmetic on what the
//! shell half read, so it is tested without a shell.

use gevrek_zones::item::ItemKind;

/// What the shell says an item is (`SFGAO_*`), as far as the kind needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Nature {
    /// It is in the file system (a file or folder with a path).
    pub file_system: bool,
    /// It can be opened as a folder.
    pub folder: bool,
    /// It can be read as one stream of bytes. A compressed folder
    /// (`.zip`) is both a folder and a stream to the shell; to a person
    /// it is a file.
    pub stream: bool,
    /// It leads to something else (`.lnk`, `.url`).
    pub link: bool,
}

/// The kind of an item of that nature.
pub fn kind_of(nature: Nature) -> ItemKind {
    if !nature.file_system {
        ItemKind::Virtual
    } else if nature.link {
        ItemKind::Shortcut
    } else if nature.folder && !nature.stream {
        ItemKind::Folder
    } else {
        ItemKind::File
    }
}

/// The type of a folder, in the words the other systems use for it, so
/// that one rule fits both.
pub const FOLDER_TYPE: &str = "inode/directory";

/// The type a rule may ask an item for. Windows keeps a content type
/// (`text/plain`) for the endings whose programs registered one, and for
/// many more only says what sort of thing a file is (`image`, `audio`,
/// `text`): the "perceived type". The content type is taken where there
/// is one; else the sort becomes the first half of a type made up of it
/// and the ending (`image/x-xcf`), so that a rule for `image/*` holds for
/// every picture Windows knows to be one. Sorts that are no first half of
/// a type (`document`, `compressed`, `system`) come under `application`.
pub fn type_of(extension: &str, content_type: Option<&str>, perceived: Option<&str>) -> Option<String> {
    if let Some(content) = content_type.map(str::trim).filter(|content| content.contains('/')) {
        return Some(content.to_lowercase());
    }
    let perceived = perceived?.trim().to_lowercase();
    if perceived.is_empty() || extension.is_empty() {
        return None;
    }
    let first = match perceived.as_str() {
        "text" | "image" | "audio" | "video" => perceived.as_str(),
        _ => "application",
    };
    Some(format!("{first}/x-{}", extension.to_lowercase()))
}

/// A time of the system (`SystemTime`) as seconds since 1970; nothing for
/// a time before that.
pub fn seconds(time: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    time.ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

/// Whether an id names something in the file system: a drive's path
/// (`C:\Users\ada\a.txt`) or one on another machine (`\\server\share\a`).
/// What is no file is named by the shell with two colons and a class
/// (`::{20D04FE0-...}` is This PC).
pub fn is_path(id: &str) -> bool {
    let bytes = id.as_bytes();
    id.starts_with("\\\\") && !id.starts_with("\\\\?\\") && id.len() > 2
        || bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Whether Explorer shows a file with these attributes of the file
/// system. A hidden file is shown when the person has hidden files
/// shown; one that is hidden and the system's own (`desktop.ini`) never
/// is: Explorer shows those only with a second setting that is not
/// followed here.
pub fn listed(attributes: u32, hidden_shown: bool) -> bool {
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    if attributes & HIDDEN == 0 {
        return true;
    }
    hidden_shown && attributes & SYSTEM == 0
}

/// The Recycle Bin, as the shell names it.
pub const RECYCLE_BIN: &str = "::{645FF040-5081-101B-9F08-00AA002F954E}";

/// Whether the desktop shows a thing of the shell's that is no file.
///
/// The shell's namespace has many such things directly under the desktop
/// (This PC, the person's own folder, Network, Control Panel, Libraries,
/// and more): that is where its tree of folders begins. Which of them lie
/// on the desktop as icons is a setting ("Desktop icon settings"), kept
/// in the registry per class: 1 for hidden, 0 for shown, the person's own
/// value (`user`) before the machine's (`machine`). A thing neither says
/// anything about is not shown, except the Recycle Bin, which is there
/// unless it is switched off.
pub fn shown_on_desktop(id: &str, user: Option<u32>, machine: Option<u32>) -> bool {
    match user.or(machine) {
        Some(hidden) => hidden == 0,
        None => id.eq_ignore_ascii_case(RECYCLE_BIN),
    }
}

/// The class of a thing of the shell's that lies directly under the
/// desktop and is no file (`::{20D04FE0-...}` gives `{20D04FE0-...}`).
/// Nothing for a path, and nothing for what lies deeper.
pub fn class_of(id: &str) -> Option<&str> {
    let class = id.strip_prefix("::")?;
    (class.starts_with('{') && class.ends_with('}') && !class.contains('\\')).then_some(class)
}

/// Two paths that name the same file to Windows: letters of either case,
/// either separator, with or without one at the end.
pub fn same_path(one: &str, other: &str) -> bool {
    let plain = |path: &str| {
        path.trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    plain(one) == plain(other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_follows_what_the_shell_says() {
        let nature = |file_system, folder, stream, link| Nature { file_system, folder, stream, link };
        for (what, kind) in [
            (nature(true, false, true, false), ItemKind::File),
            (nature(true, true, false, false), ItemKind::Folder),
            // A compressed folder is a file to the person.
            (nature(true, true, true, false), ItemKind::File),
            (nature(true, false, true, true), ItemKind::Shortcut),
            // This PC, the Recycle Bin: folders that are no files.
            (nature(false, true, false, false), ItemKind::Virtual),
            (nature(false, false, false, false), ItemKind::Virtual),
        ] {
            assert_eq!(kind_of(what), kind, "{what:?}");
        }
    }

    #[test]
    fn a_type_is_the_registered_one_or_made_of_the_sort() {
        for (extension, content, perceived, wanted) in [
            ("txt", Some("text/plain"), Some("text"), Some("text/plain")),
            ("PNG", Some("Image/PNG"), None, Some("image/png")),
            ("xcf", None, Some("image"), Some("image/x-xcf")),
            ("rs", None, Some("Text"), Some("text/x-rs")),
            ("docx", None, Some("document"), Some("application/x-docx")),
            ("7z", None, Some("compressed"), Some("application/x-7z")),
            // Something written there that is no type is passed over.
            ("abc", Some("nonsense"), Some("video"), Some("video/x-abc")),
            ("abc", None, None, None),
            ("", None, Some("text"), None),
        ] {
            assert_eq!(type_of(extension, content, perceived).as_deref(), wanted, "{extension}");
        }
    }

    #[test]
    fn an_id_is_a_path_or_a_place_that_is_no_file() {
        for (id, path) in [
            ("C:\\Users\\ada\\Desktop\\a.txt", true),
            ("d:/x", true),
            ("\\\\server\\share\\a.txt", true),
            ("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", false),
            ("::{645FF040-5081-101B-9F08-00AA002F954E}", false),
            ("\\\\?\\usb#vid", false),
            ("C:", false),
            ("", false),
        ] {
            assert_eq!(is_path(id), path, "{id}");
        }
        assert!(same_path("C:\\Users\\Ada\\Desktop\\", "c:/users/ada/desktop"));
        assert!(!same_path("C:\\a", "C:\\b"));
    }

    #[test]
    fn what_is_no_file_lies_on_the_desktop_when_the_settings_say_so() {
        let this_pc = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        // The person's value counts before the machine's.
        assert!(shown_on_desktop(this_pc, Some(0), Some(1)));
        assert!(!shown_on_desktop(this_pc, Some(1), Some(0)));
        assert!(!shown_on_desktop(this_pc, None, Some(1)));
        assert!(shown_on_desktop(this_pc, None, Some(0)));
        // Nothing said: only the Recycle Bin is there.
        assert!(!shown_on_desktop(this_pc, None, None));
        assert!(shown_on_desktop(RECYCLE_BIN, None, None));
        assert!(shown_on_desktop(&RECYCLE_BIN.to_lowercase(), None, None));
        assert!(!shown_on_desktop(RECYCLE_BIN, Some(1), None));

        assert_eq!(class_of(this_pc), Some("{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        assert_eq!(class_of("::{A}\\::{B}"), None);
        assert_eq!(class_of("C:\\Users\\ada"), None);
        assert_eq!(class_of("::nothing"), None);
    }

    #[test]
    fn a_hidden_file_is_listed_only_when_the_person_shows_hidden_files() {
        const PLAIN: u32 = 0x20;
        const HIDDEN: u32 = 0x22;
        const OF_THE_SYSTEM: u32 = 0x26;
        const SYSTEM_ONLY: u32 = 0x24;
        for (attributes, hidden_shown, shown) in [
            (PLAIN, false, true),
            (HIDDEN, false, false),
            (HIDDEN, true, true),
            (OF_THE_SYSTEM, false, false),
            (OF_THE_SYSTEM, true, false),
            (SYSTEM_ONLY, false, true),
        ] {
            assert_eq!(listed(attributes, hidden_shown), shown, "{attributes:#x} {hidden_shown}");
        }
    }

    #[test]
    fn times_are_seconds_since_1970() {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(seconds(Ok(time)), Some(1_700_000_000));
        assert_eq!(seconds(Ok(std::time::UNIX_EPOCH - std::time::Duration::from_secs(5))), None);
        assert_eq!(seconds(Err(std::io::Error::other("none"))), None);
    }
}
