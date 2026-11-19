//! What passes between the zones and whoever asks from outside (the
//! command line, a script, a key): what is asked (`Request`), what is
//! answered (`Answer`), and the zones as they are told to others
//! (`ZonesSnapshot`).
//!
//! The zones live in the shell, and the window manager is the one place
//! other programs talk to. So the shell tells the window manager how the
//! zones are, and the window manager hands on what is asked of them. All
//! of it travels as JSON under one number (`VERSION`): the shapes here
//! still move, and a reader that meets a number it does not know says so
//! instead of guessing. See docs/zones/milestone-2-ipc.md.

use crate::config::{Lock, View, ZoneKind};
use crate::core::{Command, Effect, ZonesCore, ZonesEvent};
use crate::geometry::{Display, ZoneGeometry};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::ItemKind;
use crate::layout;
use crate::profile::ProfileKey;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// The number of these shapes. It goes up when a reader of the old ones
/// would read the new ones wrongly; a field that is merely new does not
/// change it.
pub const VERSION: u32 = 1;

/// The word that asks for the system's own icons to be put back.
pub const RESTORE_SYSTEM_ICONS: &str = "restore_system_icons";

/// What is asked of whoever runs the zones.
///
/// Nearly all of it is a `Command` for the core. What is not, is asked
/// of the backend directly, because it must work when the zones are
/// switched off, which is when the core refuses every command.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// For the core (`Host::command`).
    Command(Command),
    /// Put the system's own desktop icons back as they were written down
    /// before the zones took them over
    /// (`ZonesBackend::restore_system_icons`; on Windows, Explorer's).
    /// Written `{"command": "restore_system_icons"}`.
    RestoreSystemIcons,
}

impl Request {
    /// Read a request from its JSON: a `Command` as the core writes it,
    /// or one of the words that are not the core's.
    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let word = value.get("command").and_then(|word| word.as_str());
        match word {
            None => Err("a zones command is an object with a \"command\"".to_string()),
            Some(RESTORE_SYSTEM_ICONS) => Ok(Self::RestoreSystemIcons),
            Some(_) => serde_json::from_value(value)
                .map(Self::Command)
                .map_err(|error| format!("not a zones command: {error}")),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        Self::from_value(value)
    }

    pub fn to_value(&self) -> serde_json::Value {
        match self {
            Self::Command(command) => serde_json::to_value(command).unwrap_or_default(),
            Self::RestoreSystemIcons => serde_json::json!({ "command": RESTORE_SYSTEM_ICONS }),
        }
    }
}

/// What came of a request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// It was carried out.
    pub ok: bool,
    /// Why not, where it was not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What happened because of it (a zone was made, and which; what the
    /// rules would do; what was found).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ZonesEvent>,
}

impl Answer {
    pub fn done() -> Self {
        Self { ok: true, error: None, events: Vec::new() }
    }

    pub fn refused(reason: impl Into<String>) -> Self {
        Self { ok: false, error: Some(reason.into()), events: Vec::new() }
    }

    /// The answer to a command, from the effects it gave: refused with
    /// the core's reason where the core refused it, else done, with what
    /// happened.
    pub fn of(effects: &[Effect]) -> Self {
        let events = events_of(effects);
        let reason = events.iter().find_map(|event| match event {
            ZonesEvent::Refused { reason } => Some(reason.clone()),
            _ => None,
        });
        Self { ok: reason.is_none(), error: reason, events }
    }
}

/// The events among some effects, in their order: what listeners are told.
pub fn events_of(effects: &[Effect]) -> Vec<ZonesEvent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

/// The word an event is known by (`zone_created`): its `event` in JSON.
pub fn event_name(event: &ZonesEvent) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| value.get("event").and_then(|name| name.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// An item, as far as a listing tells of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemSnapshot {
    pub id: ItemId,
    pub name: String,
    pub kind: ItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One zone as it is now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneSnapshot {
    pub id: ZoneId,
    pub title: String,
    pub kind: ZoneKind,
    /// The display it is on, by the id that stays with the display. None
    /// while no display is known, or while the zones are switched off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayId>,
    /// Where it is drawn on the desktop, in logical units: of the title
    /// alone when it is rolled up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// The same as it is remembered: from a corner or side of its display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<ZoneGeometry>,
    /// It is shown now (not hidden here, not put away by the quick hide).
    pub visible: bool,
    pub rolled_up: bool,
    pub always_visible: bool,
    pub locked: Lock,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_group: Option<String>,
    /// It is the tab in front, or shares its place with none.
    pub front: bool,
    pub view: View,
    pub icon_size: u32,
    pub item_count: usize,
    /// In the order the zone shows them.
    pub items: Vec<ItemSnapshot>,
}

/// The zones as they are told to others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZonesSnapshot {
    /// `VERSION` of whoever wrote it.
    pub version: u32,
    /// `desktop.zones.enabled`. Off: the zones are listed as they are
    /// configured, none is shown and none has a place.
    pub enabled: bool,
    /// Why the zones are not shown although they are switched on (the
    /// system's own icons could not be put away safely, say). None while
    /// nothing stands in their way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_shown: Option<String>,
    /// Every zone that is not always visible is put away.
    pub quick_hidden: bool,
    /// The zones are above the windows while a key is held.
    pub peeking: bool,
    /// What the arrangement in use is for: the set of displays, and the
    /// workspace where each has its own. None until the displays are known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileKey>,
    pub displays: Vec<Display>,
    pub zones: Vec<ZoneSnapshot>,
    /// The items of the desktop that are in no zone.
    pub loose: Vec<ItemSnapshot>,
}

impl Default for ZonesSnapshot {
    fn default() -> Self {
        Self {
            version: VERSION,
            enabled: false,
            not_shown: None,
            quick_hidden: false,
            peeking: false,
            profile: None,
            displays: Vec::new(),
            zones: Vec::new(),
            loose: Vec::new(),
        }
    }
}

impl ZonesSnapshot {
    /// The zones of a core as they are at `now` (seconds since 1970, as
    /// every time the core is given).
    pub fn of(core: &ZonesCore, now: u64) -> Self {
        let scene = core.scene(now);
        let config = core.config();
        let groups = layout::tab_groups(&config.zones);
        let item = |item: &crate::item::Item| ItemSnapshot {
            id: item.id.clone(),
            name: item.name.clone(),
            kind: item.kind,
            path: item.path.clone(),
        };
        let zones = config
            .zones
            .iter()
            .map(|definition| {
                let drawn = scene.zones.iter().find(|zone| zone.zone == definition.id);
                // Tabs share the place of the first of them.
                let group = layout::group_of(&groups, &definition.id);
                let leader = group.and_then(|group| group.members.first()).unwrap_or(&definition.id);
                let items: Vec<ItemSnapshot> =
                    core.zone_items(&definition.id, now).into_iter().map(item).collect();
                ZoneSnapshot {
                    id: definition.id.clone(),
                    title: definition.title.clone(),
                    kind: definition.kind.clone(),
                    display: drawn.map(|zone| zone.display.clone()),
                    rect: drawn.map(|zone| zone.rect),
                    geometry: core
                        .profile()
                        .filter(|_| core.enabled())
                        .and_then(|profile| profile.geometry.get(leader))
                        .cloned(),
                    visible: drawn.is_some_and(|zone| zone.shown),
                    rolled_up: definition.rolled_up,
                    always_visible: definition.always_visible,
                    locked: definition.lock,
                    tab_group: group.map(|group| group.name.clone()),
                    front: drawn.map_or(group.is_none(), |zone| zone.front),
                    view: definition.view,
                    icon_size: definition.icon_size,
                    item_count: items.len(),
                    items,
                }
            })
            .collect();
        let mut loose: Vec<ItemSnapshot> = core.loose_items().into_iter().map(item).collect();
        loose.sort_by(|one, other| layout::compare_names(&one.name, &other.name));
        Self {
            version: VERSION,
            enabled: core.enabled(),
            // (Whoever runs the zones knows, and says: `not_shown`.)
            not_shown: None,
            quick_hidden: scene.quick_hidden,
            peeking: scene.peeking,
            profile: core.profile().map(|profile| profile.key.clone()),
            displays: core.displays().to_vec(),
            zones,
            loose,
        }
    }

    pub fn zone(&self, id: &str) -> Option<&ZoneSnapshot> {
        self.zones.iter().find(|zone| zone.id.as_str() == id)
    }

    /// Read a snapshot from its JSON. One of a number this reader does
    /// not know is refused: its fields may mean something else.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| format!("the zones' state is not JSON: {error}"))?;
        let version = value.get("version").and_then(|version| version.as_u64()).unwrap_or(0);
        if version != u64::from(VERSION) {
            return Err(format!(
                "the zones' state is of version {version}, and this program reads version {VERSION}"
            ));
        }
        serde_json::from_value(value).map_err(|error| format!("the zones' state is not read: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendEvent;
    use crate::config::{ZoneDefinition, ZonesConfig};
    use crate::state::ZonesState;

    const NOW: u64 = 1_800_000_000;

    fn core(enabled: bool) -> ZonesCore {
        let config = ZonesConfig {
            enabled,
            zones: vec![ZoneDefinition::manual("work", "Work")],
            ..ZonesConfig::default()
        };
        let mut core = ZonesCore::new(config, ZonesState::default());
        core.event(
            BackendEvent::Displays {
                displays: vec![Display {
                    primary: true,
                    ..Display::new("DEL-A1B2-77", Rect::new(0, 0, 1920, 1080))
                }],
                reserved: Vec::new(),
            },
            NOW,
        );
        core
    }

    #[test]
    fn a_request_is_a_core_command_or_one_of_the_other_words() {
        let hide = Request::from_json(r#"{"command":"quick_hide","hidden":true}"#).unwrap();
        assert_eq!(hide, Request::Command(Command::QuickHide { hidden: Some(true) }));
        assert_eq!(Request::from_value(hide.to_value()).unwrap(), hide);

        let restore = Request::from_json(r#"{"command":"restore_system_icons"}"#).unwrap();
        assert_eq!(restore, Request::RestoreSystemIcons);
        assert_eq!(Request::from_value(restore.to_value()).unwrap(), restore);

        // What cannot be read says why, in words that name the trouble.
        let unknown = Request::from_json(r#"{"command":"fly"}"#).unwrap_err();
        assert!(unknown.starts_with("not a zones command: "), "{unknown}");
        let missing = Request::from_json(r#"{"command":"rename_zone","zone":"work"}"#).unwrap_err();
        assert!(missing.contains("title"), "{missing}");
        assert!(Request::from_json("[1]").unwrap_err().contains("\"command\""));
        assert!(Request::from_json("{").unwrap_err().starts_with("not JSON"));
    }

    #[test]
    fn an_answer_says_what_the_core_said() {
        let mut core = core(true);
        let made = core.command(
            Command::CreateZone { title: "Play".to_string(), kind: ZoneKind::Manual, rect: None },
            NOW,
        );
        let answer = Answer::of(&made);
        assert!(answer.ok && answer.error.is_none());
        assert!(answer.events.iter().any(|event| matches!(event, ZonesEvent::ZoneCreated { .. })));

        let refused = core.command(Command::DeleteZone { zone: ZoneId::new("nowhere") }, NOW);
        let answer = Answer::of(&refused);
        assert!(!answer.ok);
        assert!(answer.error.is_some());

        // Switched off, the core refuses everything, and says that.
        let mut off = self::core(false);
        let answer = Answer::of(&off.command(Command::QuickHide { hidden: None }, NOW));
        assert_eq!(answer.error.as_deref(), Some("zones are switched off (desktop.zones.enabled)"));

        let text = serde_json::to_string(&answer).unwrap();
        assert_eq!(serde_json::from_str::<Answer>(&text).unwrap(), answer);
        assert_eq!(serde_json::to_string(&Answer::done()).unwrap(), r#"{"ok":true}"#);
    }

    #[test]
    fn events_have_the_names_they_are_told_by() {
        assert_eq!(event_name(&ZonesEvent::LayoutChanged), "layout_changed");
        assert_eq!(event_name(&ZonesEvent::ZoneCreated { zone: ZoneId::new("a") }), "zone_created");
    }

    #[test]
    fn a_snapshot_tells_the_zones_as_they_are() {
        let mut core = core(true);
        core.event(
            BackendEvent::Listed {
                location: crate::backend::Location::Desktop,
                items: vec![
                    crate::item::Item::file("/desk/b.txt"),
                    crate::item::Item::file("/desk/a.txt"),
                ],
            },
            NOW,
        );
        let refused = core.command(
            Command::AddItem {
                zone: ZoneId::new("work"),
                item: ItemId::new("/desk/b.txt"),
                index: None,
                cell: None,
            },
            NOW,
        );
        assert!(Answer::of(&refused).ok);
        core.command(Command::RollUp { zone: ZoneId::new("work"), rolled_up: Some(true) }, NOW);
        core.command(
            Command::Lock { zone: ZoneId::new("work"), geometry: Some(true), members: None },
            NOW,
        );

        let snapshot = ZonesSnapshot::of(&core, NOW);
        assert_eq!(snapshot.version, VERSION);
        assert!(snapshot.enabled && !snapshot.quick_hidden && !snapshot.peeking);
        assert_eq!(snapshot.profile.as_ref().unwrap().displays[0].id.as_str(), "DEL-A1B2-77");
        assert_eq!(snapshot.displays.len(), 1);
        let zone = snapshot.zone("work").unwrap();
        assert_eq!(zone.title, "Work");
        assert_eq!(zone.display.as_ref().unwrap().as_str(), "DEL-A1B2-77");
        assert_eq!(zone.geometry.as_ref().unwrap().display.as_str(), "DEL-A1B2-77");
        assert!(zone.visible && zone.rolled_up && zone.front);
        assert!(zone.locked.geometry && !zone.locked.members);
        // Rolled up, the rectangle is the title's: lower than the zone is.
        assert!(zone.rect.unwrap().height < zone.geometry.as_ref().unwrap().height);
        assert_eq!(zone.item_count, 1);
        assert_eq!(zone.items[0].name, "b.txt");
        assert_eq!(snapshot.loose.len(), 1);
        assert_eq!(snapshot.loose[0].id.as_str(), "/desk/a.txt");

        core.command(Command::QuickHide { hidden: Some(true) }, NOW);
        let hidden = ZonesSnapshot::of(&core, NOW);
        assert!(hidden.quick_hidden && !hidden.zone("work").unwrap().visible);

        // It survives its text, and a text of another version is refused.
        let text = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(ZonesSnapshot::from_json(&text).unwrap(), snapshot);
        let newer = text.replacen("\"version\":1", "\"version\":2", 1);
        assert!(ZonesSnapshot::from_json(&newer).unwrap_err().contains("version 2"));
    }

    #[test]
    fn zones_that_are_switched_off_are_listed_and_not_shown() {
        let snapshot = ZonesSnapshot::of(&core(false), NOW);
        assert!(!snapshot.enabled);
        let zone = snapshot.zone("work").unwrap();
        assert!(!zone.visible);
        assert_eq!((zone.rect, zone.display.clone(), zone.geometry.clone()), (None, None, None));
    }
}
