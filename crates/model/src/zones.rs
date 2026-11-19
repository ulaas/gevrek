//! Zones: containers on the desktop that hold its icons (`desktop.zones`).
//! What the person sets is here: the zones there are, how they look, and
//! the rules that sort items into them. Everything has a default, a file
//! without a field reads with it, and keys this version does not know are
//! kept and written back as they were.
//!
//! What changes with every drag (which item is in which zone, where a zone
//! is on which displays) is not configuration: `gevrek-zones` keeps it in a
//! state file of its own, and holds the logic. The places windows snap to
//! are another thing (`snap_zones`).

use crate::Extra;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A zone, as the configuration names it.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ZoneId(pub String);

impl ZoneId {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ZoneId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for ZoneId {
    fn from(text: &str) -> Self {
        Self(text.to_string())
    }
}

/// What sort of thing an item of the desktop is.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    #[default]
    File,
    Folder,
    /// A link to something else (`.lnk`, a symbolic link, a `.url`).
    Shortcut,
    /// Something that starts a program (a `.desktop` entry).
    Launcher,
    /// Not a file at all: This PC, the Recycle Bin.
    Virtual,
}

/// The section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ZonesConfig {
    /// Whether zones are shown at all.
    pub enabled: bool,
    pub zones: Vec<ZoneDefinition>,
    /// Tried in order for each item; the first that matches decides.
    pub rules: Vec<Rule>,
    /// What happens to an item no rule matches. Without it such an item
    /// stays where it is.
    pub catch_all: Option<RuleAction>,
    /// An item that comes to the desktop is put into the zone its rule
    /// names, at once. (Only that: a rule that moves the file itself waits
    /// until the person asks for the rules to be applied.)
    pub rules_on_arrival: bool,
    /// Each workspace has an arrangement of its own.
    pub profile_per_workspace: bool,
    /// How large the items outside of every zone are drawn.
    pub loose_icon_size: u32,
    /// The look of every zone that does not set its own.
    pub look: ZoneLook,
    pub snap: SnapSettings,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for ZonesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            zones: Vec::new(),
            rules: Vec::new(),
            catch_all: None,
            rules_on_arrival: true,
            profile_per_workspace: false,
            loose_icon_size: DEFAULT_ICON_SIZE,
            look: ZoneLook::default(),
            snap: SnapSettings::default(),
            extra: Extra::new(),
        }
    }
}

impl ZonesConfig {
    pub fn zone(&self, id: &ZoneId) -> Option<&ZoneDefinition> {
        self.zones.iter().find(|zone| &zone.id == id)
    }

    pub fn zone_mut(&mut self, id: &ZoneId) -> Option<&mut ZoneDefinition> {
        self.zones.iter_mut().find(|zone| &zone.id == id)
    }

    /// The actions of the rules and of the catch-all, each with its place.
    fn actions(&self) -> impl Iterator<Item = (String, &RuleAction)> {
        self.rules
            .iter()
            .enumerate()
            .map(|(index, rule)| (format!("desktop.zones.rules[{index}].action"), &rule.action))
            .chain(self.catch_all.iter().map(|action| ("desktop.zones.catch_all".to_string(), action)))
    }

    /// What cannot be right, as `(path, message)`.
    pub fn problems(&self) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        let mut say = |path: String, message: &str| problems.push((path, message.to_string()));
        let size_is_wrong = |size: u32| !(ICON_SIZES.0..=ICON_SIZES.1).contains(&size);
        const SIZE: &str = "An icon is between 16 and 256 units";
        const OPACITY: &str = "Opacity is in hundredths, from 0 to 100";
        if size_is_wrong(self.loose_icon_size) {
            say("desktop.zones.loose_icon_size".to_string(), SIZE);
        }
        if self.look.opacity.is_some_and(|opacity| opacity > 100) {
            say("desktop.zones.look.opacity".to_string(), OPACITY);
        }
        for (name, value, most) in [
            ("distance", self.snap.distance, 200),
            ("gap", self.snap.gap, 200),
            ("grid", self.snap.grid, 1000),
        ] {
            if !(0..=most).contains(&value) {
                say(
                    format!("desktop.zones.snap.{name}"),
                    &format!("Must be between 0 and {most} units"),
                );
            }
        }
        let mut ids = BTreeSet::new();
        for (index, zone) in self.zones.iter().enumerate() {
            let path = format!("desktop.zones.zones[{index}]");
            if zone.id.as_str().trim().is_empty() {
                say(format!("{path}.id"), "A zone needs an id");
            } else if !ids.insert(&zone.id) {
                say(format!("{path}.id"), &format!("Another zone is called '{}' already", zone.id));
            }
            if size_is_wrong(zone.icon_size) {
                say(format!("{path}.icon_size"), SIZE);
            }
            if zone.look.opacity.is_some_and(|opacity| opacity > 100) {
                say(format!("{path}.look.opacity"), OPACITY);
            }
            if matches!(&zone.kind, ZoneKind::Portal { directory } if directory.trim().is_empty()) {
                say(format!("{path}.kind.directory"), "A portal needs the folder it shows");
            }
            if zone.tab_group.as_ref().is_some_and(|group| group.trim().is_empty()) {
                say(format!("{path}.tab_group"), "A group of tabs needs a name (or leave it out)");
            }
        }
        for (path, action) in self.actions() {
            match action {
                RuleAction::AssignToZone { zone } if zone.as_str().trim().is_empty() => {
                    say(format!("{path}.zone"), "Name the zone the items go to");
                }
                RuleAction::MoveToFolder { folder } if folder.trim().is_empty() => {
                    say(format!("{path}.folder"), "Name the folder the files are moved to");
                }
                _ => {}
            }
        }
        problems
    }

    /// What is worth knowing and stops nothing, as `(path, message)`: a
    /// rule that names a zone which is not there. (Its items stay where
    /// they are. It is no error, so that taking a zone away never leaves a
    /// configuration that cannot be saved.)
    pub fn remarks(&self) -> Vec<(String, String)> {
        self.actions()
            .filter_map(|(path, action)| match action {
                RuleAction::AssignToZone { zone }
                    if !zone.as_str().trim().is_empty() && self.zone(zone).is_none() =>
                {
                    Some((format!("{path}.zone"), format!("No zone is called '{zone}'")))
                }
                _ => None,
            })
            .collect()
    }
}

/// The side of an icon, in logical units, where nothing else is set.
pub const DEFAULT_ICON_SIZE: u32 = 48;

/// The smallest and the largest an icon is drawn.
pub const ICON_SIZES: (u32, u32) = (16, 256);

/// One zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ZoneDefinition {
    pub id: ZoneId,
    pub title: String,
    pub kind: ZoneKind,
    pub view: View,
    pub sort: Sort,
    /// The order turned around (the newest first, the largest first).
    pub sort_descending: bool,
    pub icon_size: u32,
    /// What this zone looks like where it differs from the others.
    pub look: ZoneLook,
    pub lock: Lock,
    /// Only the title is shown.
    pub rolled_up: bool,
    /// Stays when the zones are hidden with one key.
    pub always_visible: bool,
    /// Zones with the same name here share one place and are shown as tabs.
    pub tab_group: Option<String>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for ZoneDefinition {
    fn default() -> Self {
        Self {
            id: ZoneId::default(),
            title: String::new(),
            kind: ZoneKind::Manual,
            view: View::Grid,
            sort: Sort::Manual,
            sort_descending: false,
            icon_size: DEFAULT_ICON_SIZE,
            look: ZoneLook::default(),
            lock: Lock::default(),
            rolled_up: false,
            always_visible: false,
            tab_group: None,
            extra: Extra::new(),
        }
    }
}

impl ZoneDefinition {
    /// A zone filled by hand, with a title.
    pub fn manual(id: &str, title: &str) -> Self {
        Self {
            id: ZoneId::new(id),
            title: title.to_string(),
            ..Self::default()
        }
    }
}

/// Where a zone's items come from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZoneKind {
    /// Items of the desktop the person (or a rule) put here.
    #[default]
    Manual,
    /// A window into a folder: it shows what is in there, and what is
    /// dropped on it is moved there.
    Portal { directory: String },
    /// Every item of the desktop that all of these match, without taking
    /// it from where it is.
    Smart { query: Vec<Matcher> },
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Icons with their names below, in rows.
    #[default]
    Grid,
    /// One item a line, a small icon before the name.
    List,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// The order the person gave.
    #[default]
    Manual,
    Name,
    /// Folders, then by extension.
    Type,
    /// When last changed.
    Date,
    Size,
}

/// What of a zone may not be changed by dragging.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(default)]
pub struct Lock {
    /// The zone stays where it is and as large as it is.
    pub geometry: bool,
    /// Nothing is put into the zone or taken out of it.
    pub members: bool,
}

/// How a zone looks. Each value that is absent is taken from the look of
/// all zones, and there from the theme.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ZoneLook {
    /// The ground's colour (`#rrggbb` or `#rrggbbaa`).
    pub background: Option<String>,
    /// How much of the ground is seen, in hundredths.
    pub opacity: Option<u8>,
    /// What is behind the zone is blurred, where the system can.
    pub blur: Option<bool>,
    pub corner_radius: Option<u32>,
    pub title_colour: Option<String>,
    /// The title is shown only while the pointer is over the zone.
    pub title_on_hover: Option<bool>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl ZoneLook {
    /// This look, with what it leaves open taken from `general`.
    pub fn over(&self, general: &ZoneLook) -> ZoneLook {
        ZoneLook {
            background: self.background.clone().or_else(|| general.background.clone()),
            opacity: self.opacity.or(general.opacity),
            blur: self.blur.or(general.blur),
            corner_radius: self.corner_radius.or(general.corner_radius),
            title_colour: self.title_colour.clone().or_else(|| general.title_colour.clone()),
            title_on_hover: self.title_on_hover.or(general.title_on_hover),
            extra: Extra::new(),
        }
    }
}

/// What a dragged zone is drawn to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SnapSettings {
    pub enabled: bool,
    /// From how near an edge pulls, in logical units.
    pub distance: i32,
    /// The room left between two zones that are snapped to each other.
    pub gap: i32,
    /// Zones rest on multiples of this where no edge pulls; 0 for no grid.
    pub grid: i32,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            distance: 12,
            gap: 8,
            grid: 0,
            extra: Extra::new(),
        }
    }
}

/// One rule: every matcher must hold for the action to be taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Rule {
    /// What the person calls it.
    pub name: String,
    pub enabled: bool,
    /// All of them must hold. A rule without any matches nothing: what is
    /// to happen to everything else is said by `catch_all`, on purpose.
    pub matchers: Vec<Matcher>,
    pub action: RuleAction,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            matchers: Vec::new(),
            action: RuleAction::default(),
            extra: Extra::new(),
        }
    }
}

/// One thing asked of an item. Patterns are globs (`*` for any run of
/// letters, `?` for one) and letters are compared without regard to case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum Matcher {
    /// The extension is one of these (without the dot).
    Extension { any_of: Vec<String> },
    /// The MIME type fits (`image/*`).
    Mime { pattern: String },
    /// The shown name fits.
    Name { pattern: String },
    /// The item is one of these kinds.
    Kind { any_of: Vec<ItemKind> },
    /// A time of the item lies in a span. Every bound that is given must
    /// hold; `after` and `before` are seconds since 1970, the two others
    /// count days back from now.
    Date {
        field: DateField,
        #[serde(default)]
        after: Option<u64>,
        #[serde(default)]
        before: Option<u64>,
        #[serde(default)]
        newer_than_days: Option<u32>,
        #[serde(default)]
        older_than_days: Option<u32>,
    },
    /// The size in bytes lies between the two (each included).
    Size {
        #[serde(default)]
        at_least: Option<u64>,
        #[serde(default)]
        at_most: Option<u64>,
    },
    /// Where a shortcut leads fits.
    ShortcutTarget { pattern: String },
    /// Neither made, changed nor opened for this many days.
    Untouched { days: u32 },
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DateField {
    Created,
    #[default]
    Modified,
    Accessed,
}

/// What a rule does with an item it matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RuleAction {
    /// Show the item in a zone. The file stays where it is.
    AssignToZone { zone: ZoneId },
    /// Move the file itself into a folder.
    MoveToFolder { folder: String },
}

impl Default for RuleAction {
    fn default() -> Self {
        RuleAction::AssignToZone {
            zone: ZoneId::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_section_reads_as_the_defaults() {
        let config: ZonesConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, ZonesConfig::default());
        let zone: ZoneDefinition = serde_json::from_str(r#"{"id": "a"}"#).unwrap();
        assert_eq!(zone.icon_size, DEFAULT_ICON_SIZE);
        assert_eq!(zone.kind, ZoneKind::Manual);
    }

    #[test]
    fn keys_of_a_newer_version_are_kept() {
        let text = r#"{
            "enabled": true,
            "from_the_future": [1, 2],
            "zones": [{"id": "a", "title": "A", "glow": "soft", "look": {"sheen": 3}}],
            "rules": [{"name": "r", "matchers": [], "action": {"action": "assign_to_zone", "zone": "a"}, "weight": 2}],
            "snap": {"magnet": true}
        }"#;
        let config: ZonesConfig = serde_json::from_str(text).unwrap();
        let written = serde_json::to_value(&config).unwrap();
        assert_eq!(written["from_the_future"], serde_json::json!([1, 2]));
        assert_eq!(written["zones"][0]["glow"], "soft");
        assert_eq!(written["zones"][0]["look"]["sheen"], 3);
        assert_eq!(written["rules"][0]["weight"], 2);
        assert_eq!(written["snap"]["magnet"], true);
        let again: ZonesConfig = serde_json::from_value(written).unwrap();
        assert_eq!(again, config);
    }

    #[test]
    fn kinds_matchers_and_actions_are_written_with_their_names() {
        let zone = ZoneDefinition {
            kind: ZoneKind::Portal {
                directory: "/home/ada/Downloads".to_string(),
            },
            ..ZoneDefinition::manual("d", "Downloads")
        };
        let written = serde_json::to_value(&zone).unwrap();
        assert_eq!(written["kind"]["type"], "portal");
        let matcher = Matcher::Untouched { days: 30 };
        assert_eq!(
            serde_json::to_value(&matcher).unwrap(),
            serde_json::json!({"match": "untouched", "days": 30})
        );
        let action = RuleAction::MoveToFolder {
            folder: "/tmp/old".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&action).unwrap(),
            serde_json::json!({"action": "move_to_folder", "folder": "/tmp/old"})
        );
    }

    #[test]
    fn the_section_has_a_schema_for_the_configurations_own() {
        let schema = serde_json::to_value(schemars::schema_for!(ZonesConfig)).unwrap();
        for field in ["enabled", "zones", "rules", "catch_all", "look", "snap"] {
            assert!(schema["properties"].get(field).is_some(), "{field} is not in the schema");
        }
        let text = schema.to_string();
        for name in ["ZoneDefinition", "Matcher", "RuleAction", "untouched", "move_to_folder", "portal"] {
            assert!(text.contains(name), "{name} is not in the schema");
        }
    }

    #[test]
    fn what_cannot_be_right_is_said_with_its_place() {
        assert_eq!(ZonesConfig::default().problems(), []);
        assert_eq!(ZonesConfig::default().remarks(), []);
        let assign = |zone: &str| RuleAction::AssignToZone { zone: ZoneId::new(zone) };
        let rule = |action: RuleAction| Rule { action, ..Rule::default() };
        let mut config = ZonesConfig {
            zones: vec![
                ZoneDefinition::manual("a", "A"),
                ZoneDefinition::manual("a", "Again"),
                ZoneDefinition::manual(" ", "Nameless"),
                ZoneDefinition {
                    icon_size: 4,
                    kind: ZoneKind::Portal { directory: String::new() },
                    tab_group: Some(String::new()),
                    look: ZoneLook { opacity: Some(101), ..ZoneLook::default() },
                    ..ZoneDefinition::manual("b", "B")
                },
            ],
            rules: vec![
                rule(assign("a")),
                rule(assign("")),
                rule(assign("gone")),
                rule(RuleAction::MoveToFolder { folder: " ".to_string() }),
            ],
            catch_all: Some(assign("nowhere")),
            loose_icon_size: 1000,
            ..ZonesConfig::default()
        };
        config.look.opacity = Some(200);
        config.snap.distance = -1;
        config.snap.grid = 5000;
        let places: Vec<String> = config.problems().into_iter().map(|(path, _)| path).collect();
        assert_eq!(
            places,
            [
                "desktop.zones.loose_icon_size",
                "desktop.zones.look.opacity",
                "desktop.zones.snap.distance",
                "desktop.zones.snap.grid",
                "desktop.zones.zones[1].id",
                "desktop.zones.zones[2].id",
                "desktop.zones.zones[3].icon_size",
                "desktop.zones.zones[3].look.opacity",
                "desktop.zones.zones[3].kind.directory",
                "desktop.zones.zones[3].tab_group",
                "desktop.zones.rules[1].action.zone",
                "desktop.zones.rules[3].action.folder",
            ]
        );
        // A rule for a zone that is not there is said, and stops nothing.
        let places: Vec<String> = config.remarks().into_iter().map(|(path, _)| path).collect();
        assert_eq!(places, ["desktop.zones.rules[2].action.zone", "desktop.zones.catch_all.zone"]);
    }

    #[test]
    fn a_zones_look_fills_its_gaps_from_the_general_one() {
        let general = ZoneLook {
            opacity: Some(60),
            blur: Some(true),
            ..ZoneLook::default()
        };
        let own = ZoneLook {
            opacity: Some(90),
            ..ZoneLook::default()
        };
        let both = own.over(&general);
        assert_eq!(both.opacity, Some(90));
        assert_eq!(both.blur, Some(true));
        assert_eq!(both.corner_radius, None);
    }
}
