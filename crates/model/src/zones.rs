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
    /// Whether zones are shown at all. They are, unless this says no: a
    /// desktop comes with its zones. (On Windows that means Explorer's own
    /// desktop icons are put away while gevrek runs, since the zones draw
    /// every item of the desktop instead; unless
    /// `desktop.platform.windows.hide_desktop_icons` is off.) A file that
    /// says `false` stays switched off.
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
    /// How a rolled-up zone opens for a moment.
    pub roll_up: RollUpSettings,
    /// What no zone lets be changed by dragging, whatever its own lock
    /// says ("lock everything").
    pub lock: Lock,
    pub privacy: PrivacySettings,
    /// The rules that ask about an item's age, tried again and again.
    pub timed_rules: TimedRules,
    /// What pressing the bare desktop twice with the middle button does.
    pub gestures: DeskGestures,
    /// How zones are made by themselves, one for each type of item.
    pub auto: AutoZones,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for ZonesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            zones: Vec::new(),
            rules: Vec::new(),
            catch_all: None,
            rules_on_arrival: true,
            profile_per_workspace: false,
            loose_icon_size: DEFAULT_ICON_SIZE,
            look: ZoneLook::default(),
            snap: SnapSettings::default(),
            roll_up: RollUpSettings::default(),
            lock: Lock::default(),
            privacy: PrivacySettings::default(),
            timed_rules: TimedRules::default(),
            gestures: DeskGestures::default(),
            auto: AutoZones::default(),
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
        if size_is_wrong(self.loose_icon_size) {
            say("desktop.zones.loose_icon_size".to_string(), SIZE);
        }
        for (path, message) in self.look.problems("desktop.zones.look") {
            say(path, &message);
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
        for (name, value) in [
            ("expand_delay_ms", self.roll_up.expand_delay_ms),
            ("collapse_delay_ms", self.roll_up.collapse_delay_ms),
        ] {
            if value > MOST_DELAY_MS {
                say(
                    format!("desktop.zones.roll_up.{name}"),
                    &format!("A delay is at most {MOST_DELAY_MS} thousandths of a second"),
                );
            }
        }
        if self.timed_rules.every_minutes > MOST_MINUTES {
            say(
                "desktop.zones.timed_rules.every_minutes".to_string(),
                &format!("At most {MOST_MINUTES} minutes (a week); 0 for never"),
            );
        }
        if !(1..=MOST_AUTO_ITEMS).contains(&self.auto.min_items) {
            say(
                "desktop.zones.auto.min_items".to_string(),
                &format!("A zone is made for at least 1 item, and for {MOST_AUTO_ITEMS} at most"),
            );
        }
        let mut categories = BTreeSet::new();
        for (index, category) in self.auto.categories.iter().enumerate() {
            let path = format!("desktop.zones.auto.categories[{index}].id");
            if category.id.trim().is_empty() {
                say(path, "A type needs an id: the zone made for it is known by it");
            } else if !categories.insert(&category.id) {
                say(path, &format!("Another type is called '{}' already", category.id));
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
            for (path, message) in zone.look.problems(&format!("{path}.look")) {
                say(path, &message);
            }
            if matches!(&zone.kind, ZoneKind::Portal { directory } if directory.trim().is_empty()) {
                say(format!("{path}.kind.directory"), "A portal needs the folder it shows");
            }
            if zone.tab_group.as_ref().is_some_and(|group| group.trim().is_empty()) {
                say(format!("{path}.tab_group"), "A group of tabs needs a name (or leave it out)");
            }
            if matches!(&zone.source, SmartSource::Folder { directory } if directory.trim().is_empty()) {
                say(format!("{path}.source.directory"), "Name the folder the zone looks into");
            }
            if zone.workspaces.iter().any(|workspace| workspace.trim().is_empty()) {
                say(format!("{path}.workspaces"), "A workspace is named by its name, not by nothing");
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
    /// How this zone opens for a moment while it is rolled up, where that
    /// differs from the others.
    pub expand_on: Option<ExpandOn>,
    /// What is in the zone is covered until the person asks to see it.
    pub private: bool,
    /// Windows that are tiled keep off the zone's place, as they keep off
    /// a panel. Without it the zone lies under the windows.
    pub reserves_space: bool,
    /// The workspaces the zone is shown on, by name. None: on every one.
    pub workspaces: Vec<String>,
    /// For a zone that fills itself (`kind` is `smart`): what its query
    /// is asked of.
    pub source: SmartSource,
    /// For a zone that fills itself: how many items it shows at most, the
    /// first in its order; 0 for all.
    pub limit: u32,
    /// The type of item the zone was made for by itself
    /// (`auto.categories`, by its id). Making zones by type again puts
    /// what is new of that type into this zone, and makes no second one.
    /// A zone the person made has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<String>,
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
            expand_on: None,
            private: false,
            reserves_space: false,
            workspaces: Vec::new(),
            source: SmartSource::Desktop,
            limit: 0,
            auto: None,
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
    /// Everything that all of these match, without taking it from where
    /// it is. What is asked is the desktop, or what the zone's `source`
    /// names (the files used last, today's downloads, the programs that
    /// run). A source that is not the desktop shows all it has when
    /// nothing is asked of it.
    Smart { query: Vec<Matcher> },
}

/// What a zone that fills itself looks through.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum SmartSource {
    /// The items of the desktop.
    #[default]
    Desktop,
    /// The files the person used last, the newest first, as the system
    /// keeps them.
    Recent,
    /// The person's folder of downloads, wherever the system has it.
    Downloads,
    /// The programs that have a window now.
    RunningApps,
    /// The launchers the person pinned (to the panel, to the start menu).
    Pinned,
    /// Any folder.
    Folder { directory: String },
}

/// What opens a rolled-up zone for a moment.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExpandOn {
    /// The pointer resting on its title.
    #[default]
    Hover,
    /// A press on its title.
    Click,
}

/// How a rolled-up zone opens for a moment and closes again. Opening it
/// so changes nothing that is kept: it is rolled up as before once the
/// pointer has left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RollUpSettings {
    pub expand_on: ExpandOn,
    /// How long the pointer rests on the title before the zone opens, in
    /// thousandths of a second: passing over it opens nothing.
    pub expand_delay_ms: u32,
    /// How long after the pointer left the zone closes again.
    pub collapse_delay_ms: u32,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for RollUpSettings {
    fn default() -> Self {
        Self {
            expand_on: ExpandOn::Hover,
            expand_delay_ms: 150,
            collapse_delay_ms: 400,
            extra: Extra::new(),
        }
    }
}

/// The longest a zone waits before it opens or closes.
pub const MOST_DELAY_MS: u32 = 10_000;

/// What is covered while the screen is shown to others.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WhenShared {
    /// Nothing more than otherwise.
    Nothing,
    /// The private zones, also those the person had asked to see.
    #[default]
    Private,
    /// Every zone and every loose item.
    Everything,
}

/// Covering what zones hold from eyes it is not for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PrivacySettings {
    /// Every zone is private, and so are the loose items.
    pub everything: bool,
    /// What happens when the system says the screen is shared or recorded.
    pub when_shared: WhenShared,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Rules that ask how old an item is (`untouched`, a `date` counted in
/// days) come true by themselves as time passes. They are tried again on
/// a schedule; who runs the zones keeps the clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TimedRules {
    /// How often they are tried, in minutes; 0 for never.
    pub every_minutes: u32,
    /// Also the ones that move the file itself. Without it only those
    /// that show an item in a zone are carried out unasked.
    pub move_files: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for TimedRules {
    fn default() -> Self {
        Self {
            every_minutes: 60,
            move_files: false,
            extra: Extra::new(),
        }
    }
}

/// The longest between two tries of the timed rules: a week.
pub const MOST_MINUTES: u32 = 7 * 24 * 60;

/// What a press on the bare desktop can be made to do.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DeskGesture {
    /// Nothing.
    #[default]
    None,
    /// Put the zones away, or bring them back.
    QuickHide,
    /// Draw every icon grey, or in its colours again.
    GreyIcons,
    /// Bring the zones above the windows, or let them go back.
    Peek,
}

/// What pressing the bare desktop twice with the middle button does:
/// where no zone and no item is under the pointer. (Twice with the main
/// button puts the zones away or brings them back, always: that is not
/// set here.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DeskGestures {
    /// Twice with the middle button.
    pub double_middle_click: DeskGesture,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for DeskGestures {
    fn default() -> Self {
        Self {
            double_middle_click: DeskGesture::GreyIcons,
            extra: Extra::new(),
        }
    }
}

/// One type of item a zone is made for: what counts as of that type.
///
/// An item is of the first type that names its kind; if none does, of the
/// first that names its extension; if none does, of the first whose MIME
/// beginnings fit. A type that names nothing at all (no kind, no
/// extension, no MIME) takes what no other took.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AutoCategory {
    /// What the zone made for it is known by (`ZoneDefinition.auto`).
    pub id: String,
    /// What the zone is called.
    pub title: String,
    /// The extensions that are of this type: in small letters, without
    /// the dot.
    pub extensions: Vec<String>,
    /// The beginnings of the MIME types that are (`image/`).
    pub mime_prefixes: Vec<String>,
    /// The kinds of item that are of this type whatever their extension:
    /// `folder`, `shortcut`, `launcher`, `file`. (`virtual` things, This
    /// PC and the bin, are never sorted: naming them here does nothing.)
    pub kinds: Vec<ItemKind>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl AutoCategory {
    /// Whether it names nothing, and so takes what no other type took.
    pub fn takes_the_rest(&self) -> bool {
        self.extensions.is_empty() && self.mime_prefixes.is_empty() && self.kinds.is_empty()
    }

    /// The types as they come (the table at `AutoZones`).
    pub fn built_in() -> Vec<AutoCategory> {
        let words = |text: &str| text.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let category = |id: &str, title: &str, kinds: &[ItemKind], extensions: &str, mime: &str| AutoCategory {
            id: id.to_string(),
            title: title.to_string(),
            extensions: words(extensions),
            mime_prefixes: words(mime),
            kinds: kinds.to_vec(),
            extra: Extra::new(),
        };
        vec![
            category("folders", "Folders", &[ItemKind::Folder], "", ""),
            category(
                "programs",
                "Programs",
                &[ItemKind::Shortcut, ItemKind::Launcher],
                "lnk url exe msi bat cmd desktop appimage",
                "",
            ),
            category(
                "documents",
                "Documents",
                &[],
                "pdf doc docx odt rtf txt md xls xlsx ods csv ppt pptx odp epub",
                "text/ application/pdf application/msword application/vnd.",
            ),
            category("pictures", "Pictures", &[], "png jpg jpeg gif webp bmp svg tif tiff heic ico", "image/"),
            category("video", "Video", &[], "mp4 mkv mov avi webm wmv m4v", "video/"),
            category("audio", "Audio", &[], "mp3 flac wav ogg m4a aac opus wma", "audio/"),
            category("archives", "Archives", &[], "zip 7z rar tar gz bz2 xz zst iso cab", ""),
            category(
                "code",
                "Code",
                &[],
                "rs py js ts c h cpp hpp cs java go rb php sh ps1 json toml yaml yml xml html css sql",
                "",
            ),
            category("other", "Other", &[], "", ""),
        ]
    }
}

/// Making zones by themselves, one for each type of item on the desktop
/// (`auto_create`).
///
/// The types as they come, in this order (the order counts: see
/// `AutoCategory`):
///
/// | id | title | kinds | extensions | MIME |
/// |---|---|---|---|---|
/// | `folders` | Folders | folder | | |
/// | `programs` | Programs | shortcut, launcher | lnk url exe msi bat cmd desktop appimage | |
/// | `documents` | Documents | | pdf doc docx odt rtf txt md xls xlsx ods csv ppt pptx odp epub | text/ application/pdf application/msword application/vnd. |
/// | `pictures` | Pictures | | png jpg jpeg gif webp bmp svg tif tiff heic ico | image/ |
/// | `video` | Video | | mp4 mkv mov avi webm wmv m4v | video/ |
/// | `audio` | Audio | | mp3 flac wav ogg m4a aac opus wma | audio/ |
/// | `archives` | Archives | | zip 7z rar tar gz bz2 xz zst iso cab | |
/// | `code` | Code | | rs py js ts c h cpp hpp cs java go rb php sh ps1 json toml yaml yml xml html css sql | |
/// | `other` | Other | | | |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AutoZones {
    /// A zone is made for a type only when at least this many items are
    /// of it: one zone for one file tidies nothing. (A zone that was made
    /// for a type before takes what is new of it whatever the number.)
    pub min_items: u32,
    /// With each zone that is made, a rule that sends what comes to the
    /// desktop later and is of its type into it.
    pub add_rules: bool,
    /// The types, in the order they are tried.
    pub categories: Vec<AutoCategory>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The most items a type can be asked to have before a zone is made.
pub const MOST_AUTO_ITEMS: u32 = 1000;

impl Default for AutoZones {
    fn default() -> Self {
        Self {
            min_items: 2,
            add_rules: true,
            categories: AutoCategory::built_in(),
            extra: Extra::new(),
        }
    }
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

impl Lock {
    /// Locked wherever either is.
    pub fn or(self, other: Lock) -> Lock {
        Lock {
            geometry: self.geometry || other.geometry,
            members: self.members || other.members,
        }
    }
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
    /// How much of a title bar the zone has.
    pub title_bar: Option<TitleBar>,
    /// The items' names are shown with their icons.
    pub labels: Option<bool>,
    /// A colour laid over the icons (`#rrggbb` or `#rrggbbaa`).
    pub icon_tint: Option<String>,
    /// How much of their own colour the icons keep, in hundredths: 0 is
    /// grey, 100 as they are.
    pub icon_saturation: Option<u8>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

/// How much of a title bar a zone has.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TitleBar {
    /// A bar with the title and what can be pressed on it.
    #[default]
    Full,
    /// The title alone, on the zone's own ground.
    Plain,
    /// None. (A rolled-up zone still shows its title: nothing else of it
    /// is left.)
    Hidden,
}

/// The roundest a zone's corners are, in logical units.
pub const MOST_CORNER_RADIUS: u32 = 64;

/// Whether a text is a colour as zones are given them: `#rrggbb` or
/// `#rrggbbaa`.
pub fn is_colour(text: &str) -> bool {
    let Some(digits) = text.strip_prefix('#') else {
        return false;
    };
    matches!(digits.len(), 6 | 8) && digits.chars().all(|digit| digit.is_ascii_hexdigit())
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
            title_bar: self.title_bar.or(general.title_bar),
            labels: self.labels.or(general.labels),
            icon_tint: self.icon_tint.clone().or_else(|| general.icon_tint.clone()),
            icon_saturation: self.icon_saturation.or(general.icon_saturation),
            extra: Extra::new(),
        }
    }

    /// What cannot be right, as `(path, message)`, for a look found at
    /// `path`.
    pub fn problems(&self, path: &str) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        let mut say =
            |name: &str, message: &str| problems.push((format!("{path}.{name}"), message.to_string()));
        if self.opacity.is_some_and(|opacity| opacity > 100) {
            say("opacity", "Opacity is in hundredths, from 0 to 100");
        }
        if self.corner_radius.is_some_and(|radius| radius > MOST_CORNER_RADIUS) {
            say("corner_radius", "A corner is rounded by 64 units at most");
        }
        if self.icon_saturation.is_some_and(|saturation| saturation > 100) {
            say("icon_saturation", "Saturation is in hundredths, from 0 to 100");
        }
        for (name, colour) in [
            ("background", &self.background),
            ("title_colour", &self.title_colour),
            ("icon_tint", &self.icon_tint),
        ] {
            if colour.as_ref().is_some_and(|colour| !is_colour(colour)) {
                say(name, "A colour is written #rrggbb or #rrggbbaa");
            }
        }
        problems
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
    fn what_milestones_three_to_five_set_reads_with_its_defaults_and_is_checked() {
        let config: ZonesConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.roll_up.expand_on, ExpandOn::Hover);
        assert_eq!((config.roll_up.expand_delay_ms, config.roll_up.collapse_delay_ms), (150, 400));
        assert_eq!(config.lock, Lock::default());
        assert_eq!(config.privacy.when_shared, WhenShared::Private);
        assert_eq!((config.timed_rules.every_minutes, config.timed_rules.move_files), (60, false));
        let zone: ZoneDefinition = serde_json::from_str(
            r##"{"id": "a", "kind": {"type": "smart", "query": []}, "source": {"from": "folder", "directory": "/d"},
                "limit": 12, "private": true, "reserves_space": true, "workspaces": ["2"], "expand_on": "click",
                "look": {"title_bar": "plain", "labels": false, "icon_tint": "#10203040", "icon_saturation": 40}}"##,
        )
        .unwrap();
        assert_eq!(zone.source, SmartSource::Folder { directory: "/d".to_string() });
        assert_eq!((zone.limit, zone.private, zone.reserves_space), (12, true, true));
        assert_eq!(zone.expand_on, Some(ExpandOn::Click));
        assert_eq!(zone.look.title_bar, Some(TitleBar::Plain));
        assert_eq!(
            serde_json::to_value(SmartSource::RunningApps).unwrap(),
            serde_json::json!({"from": "running_apps"})
        );
        // What cannot be right is said with its place.
        let mut wrong = ZonesConfig {
            zones: vec![ZoneDefinition {
                source: SmartSource::Folder { directory: " ".to_string() },
                workspaces: vec![String::new()],
                look: ZoneLook {
                    background: Some("red".to_string()),
                    corner_radius: Some(65),
                    icon_saturation: Some(101),
                    icon_tint: Some("#12345".to_string()),
                    ..ZoneLook::default()
                },
                ..ZoneDefinition::manual("a", "A")
            }],
            ..ZonesConfig::default()
        };
        wrong.look.title_colour = Some("#GGGGGG".to_string());
        wrong.roll_up.collapse_delay_ms = 10_001;
        wrong.timed_rules.every_minutes = 10_081;
        let places: Vec<String> = wrong.problems().into_iter().map(|(path, _)| path).collect();
        assert_eq!(
            places,
            [
                "desktop.zones.look.title_colour",
                "desktop.zones.roll_up.collapse_delay_ms",
                "desktop.zones.timed_rules.every_minutes",
                "desktop.zones.zones[0].look.corner_radius",
                "desktop.zones.zones[0].look.icon_saturation",
                "desktop.zones.zones[0].look.background",
                "desktop.zones.zones[0].look.icon_tint",
                "desktop.zones.zones[0].source.directory",
                "desktop.zones.zones[0].workspaces",
            ]
        );
        for (text, colour) in [("#a0B1c2", true), ("#a0b1c2d3", true), ("a0b1c2", false), ("#abc", false)] {
            assert_eq!(is_colour(text), colour, "{text}");
        }
        assert_eq!(
            Lock { geometry: true, members: false }.or(Lock { geometry: false, members: true }),
            Lock { geometry: true, members: true }
        );
    }

    #[test]
    fn what_the_desktops_presses_and_the_zones_by_type_set_reads_with_its_defaults() {
        let config: ZonesConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.gestures.double_middle_click, DeskGesture::GreyIcons);
        assert_eq!((config.auto.min_items, config.auto.add_rules), (2, true));
        let ids: Vec<&str> = config.auto.categories.iter().map(|category| category.id.as_str()).collect();
        assert_eq!(
            ids,
            ["folders", "programs", "documents", "pictures", "video", "audio", "archives", "code", "other"]
        );
        // The last takes the rest, and it alone.
        let rest: Vec<bool> = config.auto.categories.iter().map(AutoCategory::takes_the_rest).collect();
        assert_eq!(rest.iter().filter(|rest| **rest).count(), 1);
        assert_eq!(rest.last(), Some(&true));
        // No extension is of two types, and each is written as an item's
        // extension is: small letters, no dot.
        let mut seen = BTreeSet::new();
        for extension in config.auto.categories.iter().flat_map(|category| &category.extensions) {
            assert!(seen.insert(extension), "{extension} is of two types");
            assert!(!extension.starts_with('.') && *extension == extension.to_lowercase(), "{extension}");
        }
        // Each gesture by the word it is written with.
        for (gesture, word) in [
            (DeskGesture::None, "none"),
            (DeskGesture::QuickHide, "quick_hide"),
            (DeskGesture::GreyIcons, "grey_icons"),
            (DeskGesture::Peek, "peek"),
        ] {
            assert_eq!(serde_json::to_value(gesture).unwrap(), serde_json::json!(word));
        }
        let set: ZonesConfig = serde_json::from_str(
            r#"{"gestures": {"double_middle_click": "none", "later": 2},
                "auto": {"min_items": 4, "add_rules": false, "later": 3, "categories": [
                    {"id": "notes", "title": "Notes", "extensions": ["md"], "kinds": ["launcher"], "later": 4}]},
                "zones": [{"id": "a", "auto": "notes"}]}"#,
        )
        .unwrap();
        assert_eq!(set.gestures.double_middle_click, DeskGesture::None);
        assert_eq!((set.auto.min_items, set.auto.add_rules, set.auto.categories.len()), (4, false, 1));
        assert_eq!(set.auto.categories[0].kinds, [ItemKind::Launcher]);
        assert_eq!(set.zones[0].auto.as_deref(), Some("notes"));
        assert_eq!(set.problems(), []);
        // Keys of a newer version are kept in each of them, and a zone
        // made by hand writes no `auto` at all.
        let written = serde_json::to_value(&set).unwrap();
        assert_eq!(written["gestures"]["later"], 2);
        assert_eq!(written["auto"]["later"], 3);
        assert_eq!(written["auto"]["categories"][0]["later"], 4);
        assert_eq!(written["zones"][0]["auto"], "notes");
        let by_hand = serde_json::to_value(ZoneDefinition::manual("b", "B")).unwrap();
        assert!(by_hand.get("auto").is_none());
        // What cannot be right is said with its place.
        let mut wrong = ZonesConfig::default();
        wrong.auto.min_items = 0;
        wrong.auto.categories[1].id = "folders".to_string();
        wrong.auto.categories[2].id = " ".to_string();
        let places: Vec<String> = wrong.problems().into_iter().map(|(path, _)| path).collect();
        assert_eq!(
            places,
            [
                "desktop.zones.auto.min_items",
                "desktop.zones.auto.categories[1].id",
                "desktop.zones.auto.categories[2].id",
            ]
        );
    }

    #[test]
    fn a_zones_look_fills_its_gaps_from_the_general_one() {
        let general = ZoneLook {
            opacity: Some(60),
            blur: Some(true),
            ..ZoneLook::default()
        };
        let general = ZoneLook {
            title_bar: Some(TitleBar::Plain),
            labels: Some(false),
            icon_saturation: Some(50),
            icon_tint: Some("#000000".to_string()),
            ..general
        };
        let own = ZoneLook {
            opacity: Some(90),
            labels: Some(true),
            ..ZoneLook::default()
        };
        let both = own.over(&general);
        assert_eq!(both.opacity, Some(90));
        assert_eq!(both.blur, Some(true));
        assert_eq!(both.corner_radius, None);
        assert_eq!(both.title_bar, Some(TitleBar::Plain));
        assert_eq!(both.labels, Some(true));
        assert_eq!(both.icon_saturation, Some(50));
        assert_eq!(both.icon_tint.as_deref(), Some("#000000"));
    }
}
