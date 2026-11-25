//! What lies on the desktop, as the core knows it: a name, a kind and the
//! facts rules ask about. No picture and no handle of the system's: those
//! stay with the backend.

use crate::ids::ItemId;
use serde::{Deserialize, Serialize};

/// What sort of thing an item is.
pub use gevrek_model::zones::ItemKind;

/// What rules and sorting ask of an item. A fact the backend could not
/// find is absent, and a rule that asks for it does not match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemFacts {
    /// Without the dot, in small letters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// In bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Times are seconds since 1970.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessed: Option<u64>,
    /// Where a shortcut leads, or what a launcher starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut_target: Option<String>,
}

impl ItemFacts {
    /// When the item was last touched in any way that is known.
    pub fn last_touched(&self) -> Option<u64> {
        [self.created, self.modified, self.accessed]
            .into_iter()
            .flatten()
            .max()
    }
}

/// One item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: ItemId,
    pub kind: ItemKind,
    /// The name as it is shown.
    pub name: String,
    /// Where the item is in the file system, for the items that are
    /// there. A rule that moves files leaves the others alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub facts: ItemFacts,
}

impl Item {
    /// A file at `path`, named by the last part of the path, with the
    /// extension read off the name. The path is its id as well.
    pub fn file(path: &str) -> Self {
        let name = crate::paths::file_name(path).to_string();
        Self {
            id: ItemId::new(path),
            kind: ItemKind::File,
            facts: ItemFacts {
                extension: extension_of(&name),
                ..ItemFacts::default()
            },
            name,
            path: Some(path.to_string()),
        }
    }

    /// A folder at `path`.
    pub fn folder(path: &str) -> Self {
        Self {
            id: ItemId::new(path),
            kind: ItemKind::Folder,
            name: crate::paths::file_name(path).to_string(),
            path: Some(path.to_string()),
            facts: ItemFacts::default(),
        }
    }
}

/// The extension of a file name: what follows the last dot, in small
/// letters. A name that only begins with a dot (`.profile`) has none.
pub fn extension_of(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extension_is_what_follows_the_last_dot() {
        for (name, extension) in [
            ("report.PDF", Some("pdf")),
            ("archive.tar.gz", Some("gz")),
            (".profile", None),
            ("Makefile", None),
            ("ends.", None),
        ] {
            assert_eq!(extension_of(name).as_deref(), extension, "{name}");
        }
    }

    #[test]
    fn a_file_is_named_by_the_end_of_its_path_on_both_systems() {
        assert_eq!(Item::file("C:\\Users\\ada\\Desktop\\a.txt").name, "a.txt");
        assert_eq!(Item::file("/home/ada/Desktop/a.txt").name, "a.txt");
        assert_eq!(
            Item::file("/home/ada/Desktop/a.txt")
                .facts
                .extension
                .as_deref(),
            Some("txt")
        );
    }
}
