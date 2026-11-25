//! Walking about in a portal. A portal shows a folder; a folder in it can
//! be gone into without leaving the zone, and the way there is kept so
//! that one can go back, forward and up. Where a portal stands is not
//! remembered over a restart and is no part of the configuration: the
//! portal's own folder (its root) is, and nothing above it is ever shown.
//!
//! Whether a folder could be listed is told here too (`Listing`): a share
//! on a slow network is `loading` for a while, one that is gone is
//! `unavailable`, and its zone stays where it is all the same.

use crate::paths;
use serde::{Deserialize, Serialize};

/// How far a listing of a folder (or of what a smart zone looks through)
/// has come.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Listing {
    /// Asked for, and not answered yet.
    #[default]
    Loading,
    /// What is shown is what is there.
    Ready,
    /// It could not be listed (not there, not reachable, not allowed).
    Unavailable { reason: String },
}

impl Listing {
    pub fn is_ready(&self) -> bool {
        *self == Listing::Ready
    }
}

/// One step of the way from a portal's own folder to where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crumb {
    pub name: String,
    pub directory: String,
}

/// The parts of a path, whichever separator it is written with.
fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect()
}

/// Whether `directory` is `root` or lies somewhere below it.
pub fn within(root: &str, directory: &str) -> bool {
    let root = parts(root);
    let directory = parts(directory);
    directory.len() >= root.len() && directory[..root.len()] == root[..]
}

/// Where a portal stands, and how it came there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalView {
    root: String,
    current: String,
    back: Vec<String>,
    forward: Vec<String>,
}

/// How many folders back a portal remembers.
const WAY_BACK: usize = 64;

impl PortalView {
    /// A portal standing in its own folder.
    pub fn new(root: &str) -> Self {
        Self {
            root: root.to_string(),
            current: root.to_string(),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// The folder that is shown.
    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn at_root(&self) -> bool {
        paths::same_folder(&self.root, &self.current)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Go to a folder at or below the portal's own. Says whether the
    /// portal stands elsewhere now; a folder outside is not gone to.
    pub fn go(&mut self, directory: &str) -> bool {
        if !within(&self.root, directory) || paths::same_folder(&self.current, directory) {
            return false;
        }
        // (The portal's own folder is always written the one way it is
        // configured, so that it is one folder to whoever watches it.)
        let directory = if paths::same_folder(&self.root, directory) {
            self.root.clone()
        } else {
            directory.to_string()
        };
        let from = std::mem::replace(&mut self.current, directory);
        self.back.push(from);
        if self.back.len() > WAY_BACK {
            self.back.remove(0);
        }
        self.forward.clear();
        true
    }

    /// Go to the folder this one lies in; not above the portal's own.
    pub fn up(&mut self) -> bool {
        if self.at_root() {
            return false;
        }
        match paths::parent(&self.current).map(str::to_string) {
            Some(parent) => self.go(&parent),
            None => false,
        }
    }

    /// Go to where the portal stood before.
    pub fn back(&mut self) -> bool {
        let Some(before) = self.back.pop() else {
            return false;
        };
        let from = std::mem::replace(&mut self.current, before);
        self.forward.push(from);
        true
    }

    /// Go again to where `back` came from.
    pub fn forward(&mut self) -> bool {
        let Some(again) = self.forward.pop() else {
            return false;
        };
        let from = std::mem::replace(&mut self.current, again);
        self.back.push(from);
        true
    }

    /// The way from the portal's own folder to the one shown, the
    /// portal's own first. Each step can be gone to.
    pub fn crumbs(&self) -> Vec<Crumb> {
        let mut crumbs = vec![Crumb {
            name: paths::file_name(&self.root).to_string(),
            directory: self.root.clone(),
        }];
        let below = parts(&self.current)
            .len()
            .saturating_sub(parts(&self.root).len());
        let names: Vec<&str> = parts(&self.current);
        let mut directory = self.root.clone();
        for name in &names[names.len() - below..] {
            directory = paths::join(&directory, name);
            crumbs.push(Crumb {
                name: name.to_string(),
                directory: directory.clone(),
            });
        }
        crumbs
    }

    /// The portal as it is drawn, with how its folder's listing stands.
    pub fn scene(&self, listing: Listing) -> PortalScene {
        PortalScene {
            root: self.root.clone(),
            directory: self.current.clone(),
            crumbs: self.crumbs(),
            can_go_up: !self.at_root(),
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            listing,
        }
    }
}

/// A portal as it is to be drawn: where it stands and where it can go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalScene {
    /// The portal's own folder, from the configuration.
    pub root: String,
    /// The folder whose items are shown.
    pub directory: String,
    pub crumbs: Vec<Crumb>,
    pub can_go_up: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub listing: Listing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_within_its_root_or_it_is_not() {
        for (root, directory, inside) in [
            ("/home/ada/Downloads", "/home/ada/Downloads", true),
            ("/home/ada/Downloads", "/home/ada/Downloads/", true),
            ("/home/ada/Downloads", "/home/ada/Downloads/a/b", true),
            ("/home/ada/Downloads", "/home/ada", false),
            ("/home/ada/Downloads", "/home/ada/Downloads2", false),
            ("C:\\Users\\ada", "C:/Users/ada/Pictures", true),
            ("C:\\Users\\ada", "D:\\Users\\ada\\Pictures", false),
        ] {
            assert_eq!(within(root, directory), inside, "{directory} in {root}");
        }
    }

    #[test]
    fn a_portal_goes_in_up_back_and_forward_and_never_above_its_own_folder() {
        let mut portal = PortalView::new("/downloads");
        assert!(portal.at_root() && !portal.can_go_back() && !portal.can_go_forward());
        assert!(
            !portal.up(),
            "there is nothing above the portal's own folder"
        );
        assert!(!portal.go("/elsewhere"), "nor beside it");
        assert!(!portal.go("/downloads/"), "and it is there already");

        assert!(portal.go("/downloads/2026"));
        assert!(portal.go("/downloads/2026/november"));
        assert_eq!(portal.current(), "/downloads/2026/november");
        let names: Vec<(String, String)> = portal
            .crumbs()
            .into_iter()
            .map(|crumb| (crumb.name, crumb.directory))
            .collect();
        assert_eq!(
            names,
            [
                ("downloads".to_string(), "/downloads".to_string()),
                ("2026".to_string(), "/downloads/2026".to_string()),
                (
                    "november".to_string(),
                    "/downloads/2026/november".to_string()
                ),
            ]
        );

        assert!(portal.up());
        assert_eq!(portal.current(), "/downloads/2026");
        // Its own folder is spelt as it was given, however it is come to.
        let mut spelt = PortalView::new("/downloads/");
        assert!(spelt.go("/downloads/2026") && spelt.up());
        assert_eq!(spelt.current(), "/downloads/");
        assert!(portal.back());
        assert_eq!(portal.current(), "/downloads/2026/november");
        assert!(portal.back() && portal.back());
        assert_eq!(portal.current(), "/downloads");
        assert!(!portal.back());
        assert!(portal.forward());
        assert_eq!(portal.current(), "/downloads/2026");
        // Going somewhere new forgets what lay ahead.
        assert!(portal.go("/downloads/music"));
        assert!(!portal.can_go_forward());

        let scene = portal.scene(Listing::Loading);
        assert_eq!(scene.root, "/downloads");
        assert_eq!(scene.directory, "/downloads/music");
        assert!(scene.can_go_up && scene.can_go_back && !scene.can_go_forward);
        assert_eq!(scene.listing, Listing::Loading);
    }

    #[test]
    fn crumbs_keep_the_separator_the_folder_is_written_with() {
        let mut portal = PortalView::new("C:\\Users\\ada\\Downloads");
        assert!(portal.go("C:\\Users\\ada\\Downloads\\Work\\Reports"));
        let last = portal.crumbs().pop().unwrap();
        assert_eq!(last.directory, "C:\\Users\\ada\\Downloads\\Work\\Reports");
        assert_eq!(portal.crumbs().len(), 3);
    }

    #[test]
    fn a_listing_is_written_with_its_state() {
        assert_eq!(
            serde_json::to_string(&Listing::Ready).unwrap(),
            r#"{"state":"ready"}"#
        );
        let gone = Listing::Unavailable {
            reason: "the share is not there".to_string(),
        };
        let text = serde_json::to_string(&gone).unwrap();
        assert_eq!(
            text,
            r#"{"state":"unavailable","reason":"the share is not there"}"#
        );
        assert_eq!(serde_json::from_str::<Listing>(&text).unwrap(), gone);
    }
}
