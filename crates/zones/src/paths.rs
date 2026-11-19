//! Paths as text. The core never touches the file system, so a path is
//! only ever taken apart and put together, and it may be one of the other
//! system (a state file carried over, a test): both separators are read.

fn is_separator(character: char) -> bool {
    character == '/' || character == '\\'
}

/// The last part of a path.
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(is_separator);
    trimmed.rsplit(is_separator).next().unwrap_or(trimmed)
}

/// The folder a path lies in; nothing for a path of one part.
pub fn parent(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(is_separator);
    let cut = trimmed.rfind(is_separator)?;
    // (The root keeps its separator: the parent of `/a` is `/`.)
    Some(if cut == 0 { &trimmed[..1] } else { &trimmed[..cut] })
}

/// `name` in `folder`, with the separator the folder is written with.
pub fn join(folder: &str, name: &str) -> String {
    let separator = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    if folder.ends_with(is_separator) {
        format!("{folder}{name}")
    } else {
        format!("{folder}{separator}{name}")
    }
}

/// Whether two folders are the same one, however they end and whichever
/// separator they are written with. Letters are compared as they are: the
/// backend knows whether its file system tells them apart.
pub fn same_folder(one: &str, other: &str) -> bool {
    let parts = |path: &str| {
        path.split(is_separator)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    parts(one) == parts(other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_of_both_systems_are_taken_apart() {
        for (path, name, folder) in [
            ("/home/ada/Desktop/a.txt", "a.txt", Some("/home/ada/Desktop")),
            ("C:\\Users\\ada\\a.txt", "a.txt", Some("C:\\Users\\ada")),
            ("/home/ada/Desktop/", "Desktop", Some("/home/ada")),
            ("/a", "a", Some("/")),
            ("a.txt", "a.txt", None),
        ] {
            assert_eq!(file_name(path), name, "{path}");
            assert_eq!(parent(path), folder, "{path}");
        }
    }

    #[test]
    fn a_name_is_joined_with_the_folders_own_separator() {
        assert_eq!(join("/home/ada", "a.txt"), "/home/ada/a.txt");
        assert_eq!(join("/home/ada/", "a.txt"), "/home/ada/a.txt");
        assert_eq!(join("C:\\Users\\ada", "a.txt"), "C:\\Users\\ada\\a.txt");
        assert!(same_folder("/home/ada/", "/home/ada"));
        assert!(same_folder("C:\\Users\\ada", "C:/Users/ada/"));
        assert!(!same_folder("/home/ada", "/home/bob"));
    }
}
