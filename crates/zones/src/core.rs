//! The zones as one machine: what is, what it is told, and what follows.
//!
//! It is told two kinds of things. `BackendEvent`s are what happened
//! outside (an item came, a display went, another workspace is shown);
//! `Command`s are what the person asked for (by the pointer, a key, the
//! command line). Each gives the new state and a list of `Effect`s: what
//! a renderer and whoever listens should hear, that the configuration or
//! the state should be written, and what the backend is asked to do.
//!
//! Zones are off unless the configuration says `enabled`. A core that is
//! off shows nothing (an empty scene), carries out no command, sorts
//! nothing by itself and asks the backend for nothing; it still notes
//! what it is told of displays and items, so that it is right at once
//! when it is switched on.
//!
//! Nothing here reads a clock, a file or a display. The time is given
//! with each call, files are the host's to write, and the backend is only
//! ever asked through an effect. So the same calls give the same state,
//! on every system and in a test.

use crate::backend::{BackendError, BackendEvent, BackendRequest, FileOperation, Location};
use crate::config::{RuleAction, Sort, View, ZoneDefinition, ZoneKind, ZoneLook, ZonesConfig};
use crate::geometry::{display_of, fallback_display, work_area, Display, ReservedArea, ZoneGeometry};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::Item;
use crate::layout::{self, ItemLayout, Metrics, PlacedItem};
use crate::paths;
use crate::placement::{Cell, LoosePosition};
use crate::profile::{self, Profile, ProfileKey};
use crate::rules::{self, Plan, Scope};
use crate::search::{self, Hit};
use crate::snap::{self, Edges, Surroundings};
use crate::state::{Snapshot, ZonesState};
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How many file operations can be taken back, the last first.
const FILE_UNDO_DEPTH: usize = 32;

fn yes() -> bool {
    true
}

/// What the person asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Make a zone: at `rect` (of the desktop) if one is given, else
    /// beside the ones there are.
    CreateZone {
        title: String,
        #[serde(default)]
        kind: ZoneKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rect: Option<Rect>,
    },
    RenameZone { zone: ZoneId, title: String },
    /// Take a zone away. Its items stay on the desktop, loose.
    DeleteZone { zone: ZoneId },
    /// Put a zone's top left corner at a point of the desktop.
    MoveZone {
        zone: ZoneId,
        x: i32,
        y: i32,
        #[serde(default = "yes")]
        snap: bool,
    },
    /// Give a zone another rectangle by dragging the named sides.
    ResizeZone {
        zone: ZoneId,
        rect: Rect,
        edges: Edges,
        #[serde(default = "yes")]
        snap: bool,
    },
    /// Put an item into a zone: into its order at `index`, or on a cell.
    /// Into a portal, the file itself is moved to the portal's folder.
    AddItem {
        zone: ZoneId,
        item: ItemId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<Cell>,
    },
    /// Take an item out of its zone; it is loose.
    RemoveItem { item: ItemId },
    /// Put an item on the bare desktop, at a place of its own.
    PlaceLooseItem { item: ItemId, display: DisplayId, x: i32, y: i32 },
    /// Roll a zone up to its title or down again; without a value, the
    /// other of what it is.
    RollUp {
        zone: ZoneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rolled_up: Option<bool>,
    },
    Lock {
        zone: ZoneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geometry: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        members: Option<bool>,
    },
    /// Make a zone a tab beside another: it takes that one's place.
    Tab { zone: ZoneId, with: ZoneId },
    /// Take a zone out of its tabs; it stands beside them.
    Untab { zone: ZoneId },
    /// Bring a tab to the front.
    ActivateTab { zone: ZoneId },
    /// How a zone shows its items; what is not given stays.
    SetView {
        zone: ZoneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<View>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sort: Option<Sort>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        descending: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon_size: Option<u32>,
    },
    /// Show or hide a zone on the displays as they are now.
    ShowZone { zone: ZoneId, shown: bool },
    /// Say what the rules would do, and do nothing.
    PreviewRules {
        #[serde(default)]
        scope: Scope,
    },
    ApplyRules {
        #[serde(default)]
        scope: Scope,
    },
    /// Take back the last application of the rules.
    UndoRules,
    /// Take back the last file operation of a portal.
    UndoFileOperation,
    SaveSnapshot { name: String },
    RestoreSnapshot { name: String },
    DeleteSnapshot { name: String },
    /// Hide every zone that is not always visible, or show them again.
    QuickHide {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<bool>,
    },
    /// Bring the zones above the windows while a key is held.
    Peek { on: bool },
    Search { typed: String },
}

/// What happened, for a renderer and for whoever listens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ZonesEvent {
    ZoneCreated { zone: ZoneId },
    /// Something of the zone itself is other than it was (title, view,
    /// lock, rolled up, tabs, shown).
    ZoneChanged { zone: ZoneId },
    ZoneDeleted { zone: ZoneId },
    /// The zone is at this rectangle of the desktop now.
    ZonePlaced { zone: ZoneId, rect: Rect },
    /// What a zone holds is other than it was; without a zone, what is
    /// loose.
    ItemsChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
    },
    /// Every zone may be elsewhere: the displays or the arrangement
    /// changed.
    LayoutChanged,
    /// Another arrangement is in use. `made`: it is new, for displays
    /// never seen in this set. `moved`: the zones whose display is not
    /// there and that went to another.
    ProfileChanged { made: bool, moved: Vec<ZoneId> },
    RulesPreviewed { plan: Plan },
    RulesApplied { plan: Plan },
    RulesUndone,
    SnapshotSaved { name: String },
    SnapshotRestored { name: String },
    SnapshotDeleted { name: String },
    VisibilityChanged { quick_hidden: bool, peeking: bool },
    Found { typed: String, hits: Vec<Hit> },
    /// The backend could not do what was asked of it.
    OperationFailed { operation: FileOperation, error: BackendError },
    /// A command was not carried out, and why.
    Refused { reason: String },
}

/// What follows from an event or a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    /// Tell the renderer and whoever listens.
    Notify(ZonesEvent),
    /// The configuration (`ZonesCore::config`) changed and should be
    /// written.
    SaveConfig,
    /// The state (`ZonesCore::state`) changed and should be written.
    SaveState,
    /// The backend is asked to do something.
    Ask(BackendRequest),
}

/// A zone as it is to be drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneScene {
    pub zone: ZoneId,
    pub title: String,
    pub display: DisplayId,
    /// On the desktop; of the title alone when the zone is rolled up.
    pub rect: Rect,
    pub shown: bool,
    pub rolled_up: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_group: Option<String>,
    /// The zone is the tab in front, or shares its place with none.
    pub front: bool,
    pub look: ZoneLook,
    pub view: View,
    pub icon_size: u32,
    /// The items, from the top left corner of the zone's content area.
    pub items: ItemLayout,
}

/// The loose items of one display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LooseScene {
    pub display: DisplayId,
    /// The room they are laid out in, on the desktop.
    pub area: Rect,
    pub items: Vec<PlacedItem>,
}

/// Everything there is to draw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub zones: Vec<ZoneScene>,
    pub loose: Vec<LooseScene>,
    pub quick_hidden: bool,
    pub peeking: bool,
}

/// What a file operation that was asked for belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// A rule moves this item's file.
    RuleMove { item: ItemId, operation: FileOperation },
    /// Something was dropped on a portal; it can be taken back.
    Portal { operation: FileOperation },
    /// Something is being taken back.
    TakingBack { operation: FileOperation },
}

/// The zones.
#[derive(Debug, Clone)]
pub struct ZonesCore {
    config: ZonesConfig,
    state: ZonesState,
    /// What is on the desktop.
    items: BTreeMap<ItemId, Item>,
    /// What is in the folders portals show.
    directories: BTreeMap<String, BTreeMap<ItemId, Item>>,
    /// The folders the backend was asked to watch.
    watched: BTreeSet<String>,
    displays: Vec<Display>,
    reserved: Vec<ReservedArea>,
    workspace: Option<String>,
    /// The arrangement in use, by its place in the state's list. None
    /// until the displays are known.
    current: Option<usize>,
    quick_hidden: bool,
    peeking: bool,
    metrics: Metrics,
    requests: u64,
    pending: BTreeMap<u64, Pending>,
    file_undo: Vec<FileOperation>,
}

type Outcome = Result<Vec<Effect>, String>;

impl ZonesCore {
    /// The zones of a configuration, with what was remembered of them.
    /// Nothing is shown until the displays are told (`BackendEvent::
    /// Displays`).
    pub fn new(config: ZonesConfig, state: ZonesState) -> Self {
        Self {
            config,
            state,
            items: BTreeMap::new(),
            directories: BTreeMap::new(),
            watched: BTreeSet::new(),
            displays: Vec::new(),
            reserved: Vec::new(),
            workspace: None,
            current: None,
            quick_hidden: false,
            peeking: false,
            metrics: Metrics::default(),
            requests: 0,
            pending: BTreeMap::new(),
            file_undo: Vec::new(),
        }
    }

    pub fn config(&self) -> &ZonesConfig {
        &self.config
    }

    pub fn state(&self) -> &ZonesState {
        &self.state
    }

    pub fn displays(&self) -> &[Display] {
        &self.displays
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Use the renderer's own measures.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        self.metrics = metrics;
    }

    /// An item of the desktop or of a portal's folder.
    pub fn item(&self, id: &ItemId) -> Option<&Item> {
        self.items
            .get(id)
            .or_else(|| self.directories.values().find_map(|items| items.get(id)))
    }

    /// The arrangement in use.
    pub fn profile(&self) -> Option<&Profile> {
        self.current.and_then(|index| self.state.profiles.get(index))
    }

    fn profile_mut(&mut self) -> Option<&mut Profile> {
        self.current.and_then(|index| self.state.profiles.get_mut(index))
    }

    /// What to ask of the backend at the start: the folders of the
    /// portals. (The desktop itself and the displays are the host's to
    /// list first; see `host::start`.)
    pub fn start(&mut self) -> Vec<Effect> {
        self.sync_watches()
    }

    /// Whether the configuration has the zones switched on.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Another configuration is in force (the file was changed).
    pub fn set_config(&mut self, config: ZonesConfig) -> Vec<Effect> {
        let per_workspace = self.config.profile_per_workspace;
        self.config = config;
        let mut effects = self.sync_watches();
        if per_workspace != self.config.profile_per_workspace {
            effects.extend(self.choose_profile());
        } else if self.reconcile() {
            effects.push(Effect::SaveState);
        }
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        effects
    }

    // ---- what is where ------------------------------------------------

    /// The zone whose place a zone has: the first of its tabs, or itself.
    fn leader(&self, zone: &ZoneId) -> ZoneId {
        let groups = layout::tab_groups(&self.config.zones);
        layout::group_of(&groups, zone)
            .and_then(|group| group.members.first().cloned())
            .unwrap_or_else(|| zone.clone())
    }

    /// The zones that share a zone's place, itself among them.
    fn companions(&self, zone: &ZoneId) -> Vec<ZoneId> {
        let groups = layout::tab_groups(&self.config.zones);
        layout::group_of(&groups, zone)
            .map(|group| group.members.clone())
            .unwrap_or_else(|| vec![zone.clone()])
    }

    /// Where a zone is, whole (not rolled up), and on which display.
    fn place_of(&self, zone: &ZoneId) -> Option<(Rect, &Display)> {
        let geometry = self.profile()?.geometry.get(&self.leader(zone))?;
        let display = self.displays.iter().find(|display| display.id == geometry.display)?;
        let rect = snap::keep_reachable(geometry.on(display), work_area(display, &self.reserved));
        Some((rect, display))
    }

    /// Where a zone is drawn on the desktop: its whole rectangle, or its
    /// title when it is rolled up. Always within the free room of a
    /// display that is there.
    pub fn zone_rect(&self, zone: &ZoneId) -> Option<Rect> {
        let (rect, _) = self.place_of(zone)?;
        let rolled = self.config.zone(zone).is_some_and(|zone| zone.rolled_up);
        Some(if rolled { layout::rolled_up(rect, &self.metrics) } else { rect })
    }

    /// Whether a zone is shown now.
    pub fn is_shown(&self, zone: &ZoneId) -> bool {
        let Some(definition) = self.config.zone(zone).filter(|_| self.config.enabled) else {
            return false;
        };
        let hidden_here = self.profile().is_some_and(|profile| profile.hidden.contains(zone));
        !hidden_here && (self.peeking || !self.quick_hidden || definition.always_visible)
    }

    /// The tab of a group that is in front.
    fn front_of(&self, group: &layout::TabGroup) -> ZoneId {
        self.state
            .active_tabs
            .get(&group.name)
            .filter(|zone| group.members.contains(zone))
            .or(group.members.first())
            .cloned()
            .unwrap_or_default()
    }

    /// The items a zone shows, in the order it shows them.
    pub fn zone_items(&self, zone: &ZoneId, now: u64) -> Vec<&Item> {
        let Some(definition) = self.config.zone(zone) else {
            return Vec::new();
        };
        let items: Vec<&Item> = match &definition.kind {
            ZoneKind::Manual => self
                .state
                .placements
                .items_of(zone)
                .iter()
                .filter_map(|id| self.items.get(id))
                .collect(),
            ZoneKind::Portal { directory } => self
                .directories
                .get(directory)
                .map(|items| items.values().collect())
                .unwrap_or_default(),
            ZoneKind::Smart { query } => self
                .items
                .values()
                .filter(|item| {
                    !query.is_empty() && query.iter().all(|matcher| rules::holds(matcher, item, now))
                })
                .collect(),
        };
        layout::sorted(items, definition.sort, definition.sort_descending)
    }

    /// The zone filled by hand that an item is shown in. An item placed
    /// in a zone that is not there is loose.
    pub fn zone_of(&self, item: &ItemId) -> Option<&ZoneId> {
        self.state.placements.zone_of(item).filter(|zone| {
            self.config.zone(zone).is_some_and(|zone| zone.kind == ZoneKind::Manual)
        })
    }

    /// The items of the desktop that are in no zone.
    pub fn loose_items(&self) -> Vec<&Item> {
        self.items.values().filter(|item| self.zone_of(&item.id).is_none()).collect()
    }

    /// Everything there is to draw, as it is at `now`.
    pub fn scene(&self, now: u64) -> Scene {
        let groups = layout::tab_groups(&self.config.zones);
        let mut scene = Scene {
            quick_hidden: self.quick_hidden,
            peeking: self.peeking,
            ..Scene::default()
        };
        if !self.config.enabled {
            return scene;
        }
        for definition in &self.config.zones {
            let Some((whole, display)) = self.place_of(&definition.id) else {
                continue;
            };
            let group = layout::group_of(&groups, &definition.id);
            let content = layout::content_area(whole, group.is_some(), &self.metrics);
            let items: Vec<ItemId> = self
                .zone_items(&definition.id, now)
                .into_iter()
                .map(|item| item.id.clone())
                .collect();
            let no_cells = BTreeMap::new();
            let cells = match (&definition.kind, definition.sort) {
                (ZoneKind::Manual, Sort::Manual) => self
                    .state
                    .placements
                    .zones
                    .get(&definition.id)
                    .map_or(&no_cells, |members| &members.cells),
                _ => &no_cells,
            };
            scene.zones.push(ZoneScene {
                zone: definition.id.clone(),
                title: definition.title.clone(),
                display: display.id.clone(),
                rect: if definition.rolled_up {
                    layout::rolled_up(whole, &self.metrics)
                } else {
                    whole
                },
                shown: self.is_shown(&definition.id),
                rolled_up: definition.rolled_up,
                tab_group: group.map(|group| group.name.clone()),
                front: group.is_none_or(|group| self.front_of(group) == definition.id),
                look: definition.look.over(&self.config.look),
                view: definition.view,
                icon_size: definition.icon_size,
                items: layout::lay_out(
                    definition.view,
                    definition.icon_size,
                    &self.metrics,
                    (content.width, content.height),
                    &items,
                    cells,
                ),
            });
        }
        let Some(fallback) = fallback_display(&self.displays).map(|display| display.id.clone()) else {
            return scene;
        };
        let mut by_display: BTreeMap<DisplayId, Vec<&Item>> = BTreeMap::new();
        for item in self.loose_items() {
            let own = self.state.placements.loose.get(&item.id).map(|position| &position.display);
            let display = own
                .filter(|wanted| self.displays.iter().any(|display| &display.id == *wanted))
                .unwrap_or(&fallback);
            by_display.entry(display.clone()).or_default().push(item);
        }
        for display in &self.displays {
            let Some(items) = by_display.remove(&display.id) else {
                continue;
            };
            let area = work_area(display, &self.reserved);
            let ordered = layout::sorted(items, Sort::Name, false);
            let ids: Vec<ItemId> = ordered.iter().map(|item| item.id.clone()).collect();
            let own: BTreeMap<ItemId, (i32, i32)> = ids
                .iter()
                .filter_map(|id| {
                    let position = self.state.placements.loose.get(id)?;
                    (position.display == display.id).then(|| {
                        (
                            id.clone(),
                            (
                                position.x - (area.x - display.rect.x),
                                position.y - (area.y - display.rect.y),
                            ),
                        )
                    })
                })
                .collect();
            scene.loose.push(LooseScene {
                display: display.id.clone(),
                area,
                items: layout::lay_out_loose(
                    self.config.loose_icon_size,
                    &self.metrics,
                    (area.width, area.height),
                    &ids,
                    &own,
                ),
            });
        }
        scene
    }

    /// The items whose names fit what was typed, wherever they are shown.
    pub fn search(&self, typed: &str) -> Vec<Hit> {
        let portals: Vec<(&ZoneId, &String)> = self
            .config
            .zones
            .iter()
            .filter_map(|zone| match &zone.kind {
                ZoneKind::Portal { directory } => Some((&zone.id, directory)),
                _ => None,
            })
            .collect();
        let desktop = self.items.values().map(|item| (item, self.zone_of(&item.id)));
        let in_portals = portals.iter().flat_map(|(zone, directory)| {
            self.directories
                .get(*directory)
                .into_iter()
                .flat_map(|items| items.values())
                .map(move |item| (item, Some(*zone)))
        });
        search::search(typed, desktop.chain(in_portals))
    }

    /// What is wrong with the state, if anything. What the long runs of
    /// the tests look at after every step; nothing calls it otherwise.
    pub fn broken(&self) -> Option<String> {
        if let Some(broken) = self.state.placements.broken() {
            return Some(broken);
        }
        let mut ids = BTreeSet::new();
        for zone in &self.config.zones {
            if !ids.insert(&zone.id) {
                return Some(format!("two zones are called {}", zone.id));
            }
        }
        for (group, front) in &self.state.active_tabs {
            if self.config.zone(front).is_none_or(|zone| zone.tab_group.as_ref() != Some(group)) {
                return Some(format!("{front} is in front of {group} and is no tab of it"));
            }
        }
        if self.displays.is_empty() {
            return None;
        }
        let Some(profile) = self.profile() else {
            return Some("there are displays and no arrangement".to_string());
        };
        if profile.key != self.key() {
            return Some("the arrangement is for other displays".to_string());
        }
        for zone in &self.config.zones {
            let Some(geometry) = profile.geometry.get(&zone.id) else {
                return Some(format!("{} has no place", zone.id));
            };
            let Some(display) = self.displays.iter().find(|display| display.id == geometry.display)
            else {
                return Some(format!("{} is on {}, which is not there", zone.id, geometry.display));
            };
            let Some(rect) = self.zone_rect(&zone.id) else {
                return Some(format!("{} has no rectangle", zone.id));
            };
            // The leader's display is where the zone is drawn.
            let on = self
                .place_of(&zone.id)
                .map(|(_, display)| display)
                .unwrap_or(display);
            if rect.width < 1
                || rect.height < 1
                || !snap::within(rect, work_area(on, &self.reserved))
            {
                return Some(format!("{} at {rect:?} cannot be reached on {}", zone.id, on.id));
            }
        }
        None
    }

    // ---- keeping the state whole --------------------------------------

    fn key(&self) -> ProfileKey {
        let workspace = if self.config.profile_per_workspace {
            self.workspace.as_deref()
        } else {
            None
        };
        ProfileKey::of(&self.displays, workspace)
    }

    /// A place for a zone that has none: beside the ones there are, each a
    /// step further down and right.
    fn default_geometry(&self, already: usize) -> Option<ZoneGeometry> {
        let display = fallback_display(&self.displays)?;
        let area = work_area(display, &self.reserved);
        let step = 32 * (already % 8) as i32;
        let rect = snap::keep_reachable(
            Rect::new(area.x + 40 + step, area.y + 40 + step, 320, 240),
            area,
        );
        Some(ZoneGeometry::nearest(rect, display))
    }

    /// Make the state fit the configuration and the displays: every zone
    /// has a place on a display that is there, and nothing is in front of
    /// tabs it is no tab of. Says whether anything had to change.
    fn reconcile(&mut self) -> bool {
        self.reconcile_from(None).0
    }

    /// As `reconcile`; a zone without a place takes the one it had in
    /// `before` (the arrangement that was in use until now). Also says
    /// which zones were on a display that is not there and went to
    /// another.
    fn reconcile_from(&mut self, before: Option<&BTreeMap<ZoneId, ZoneGeometry>>) -> (bool, Vec<ZoneId>) {
        let mut changed = false;
        let valid: Vec<(String, ZoneId)> = self
            .state
            .active_tabs
            .iter()
            .filter(|(group, zone)| {
                self.config.zone(zone).is_some_and(|zone| zone.tab_group.as_ref() == Some(*group))
            })
            .map(|(group, zone)| (group.clone(), zone.clone()))
            .collect();
        if valid.len() != self.state.active_tabs.len() {
            self.state.active_tabs = valid.into_iter().collect();
            changed = true;
        }
        let Some(index) = self.current else {
            return (changed, Vec::new());
        };
        let missing: Vec<ZoneId> = self
            .config
            .zones
            .iter()
            .filter(|zone| !self.state.profiles[index].geometry.contains_key(&zone.id))
            .map(|zone| zone.id.clone())
            .collect();
        for zone in missing {
            let already = self.state.profiles[index].geometry.len();
            let geometry = before
                .and_then(|before| before.get(&zone).cloned())
                .or_else(|| self.default_geometry(already));
            if let Some(geometry) = geometry {
                self.state.profiles[index].geometry.insert(zone, geometry);
                changed = true;
            }
        }
        let displays = self.displays.clone();
        let moved = profile::rehome(&mut self.state.profiles[index], &displays);
        (changed || !moved.is_empty(), moved)
    }

    /// Use the arrangement for the displays and the workspace as they
    /// are.
    fn choose_profile(&mut self) -> Vec<Effect> {
        if self.displays.is_empty() {
            return Vec::new();
        }
        let key = self.key();
        if self.profile().is_some_and(|profile| profile.key == key) {
            return if self.reconcile() { vec![Effect::SaveState] } else { Vec::new() };
        }
        let before = self.profile().map(|profile| profile.geometry.clone());
        let workspace = key.workspace.clone();
        self.state.choices += 1;
        let Some(chosen) = profile::choose(
            &mut self.state.profiles,
            &self.displays,
            workspace.as_deref(),
            self.state.choices,
        ) else {
            return Vec::new();
        };
        self.current = Some(chosen.index);
        let mut moved = chosen.moved;
        moved.extend(self.reconcile_from(before.as_ref()).1);
        vec![
            Effect::Notify(ZonesEvent::ProfileChanged { made: chosen.made.is_some(), moved }),
            Effect::SaveState,
        ]
    }

    /// Ask for the folders of the portals to be watched, and for those of
    /// portals that are gone to be let go.
    fn sync_watches(&mut self) -> Vec<Effect> {
        let wanted: BTreeSet<String> = self
            .config
            .zones
            .iter()
            // (Switched off, no folder is watched.)
            .filter(|_| self.config.enabled)
            .filter_map(|zone| match &zone.kind {
                ZoneKind::Portal { directory } => Some(directory.clone()),
                _ => None,
            })
            .collect();
        let mut effects = Vec::new();
        for directory in wanted.difference(&self.watched) {
            effects.push(Effect::Ask(BackendRequest::Watch { directory: directory.clone() }));
        }
        for directory in self.watched.difference(&wanted) {
            effects.push(Effect::Ask(BackendRequest::Unwatch { directory: directory.clone() }));
            self.directories.remove(directory);
        }
        self.watched = wanted;
        effects
    }

    fn ask(&mut self, pending: Pending) -> Effect {
        self.requests += 1;
        let operation = match &pending {
            Pending::RuleMove { operation, .. }
            | Pending::Portal { operation }
            | Pending::TakingBack { operation } => operation.clone(),
        };
        self.pending.insert(self.requests, pending);
        Effect::Ask(BackendRequest::Perform { request: self.requests, operation })
    }

    // ---- what happened outside ----------------------------------------

    /// Something happened outside. `now` is the time in seconds since
    /// 1970 (rules that ask about age are tried against it).
    pub fn event(&mut self, event: BackendEvent, now: u64) -> Vec<Effect> {
        match event {
            BackendEvent::Listed { location: Location::Desktop, items } => {
                self.items = items.into_iter().map(|item| (item.id.clone(), item)).collect();
                vec![Effect::Notify(ZonesEvent::LayoutChanged)]
            }
            BackendEvent::Listed { location: Location::Directory { path }, items } => {
                self.directories
                    .insert(path.clone(), items.into_iter().map(|item| (item.id.clone(), item)).collect());
                self.portals_changed(&path)
            }
            BackendEvent::ItemAdded { location: Location::Desktop, item } => self.arrived(item, now),
            BackendEvent::ItemRemoved { location: Location::Desktop, item } => {
                let zone = self.zone_of(&item).cloned();
                if self.items.remove(&item).is_none() {
                    return Vec::new();
                }
                self.state.placements.take(&item);
                vec![Effect::Notify(ZonesEvent::ItemsChanged { zone }), Effect::SaveState]
            }
            BackendEvent::ItemRenamed { location: Location::Desktop, from, item } => {
                self.items.remove(&from);
                self.state.placements.rename(&from, &item.id);
                let zone = self.zone_of(&item.id).cloned();
                self.items.insert(item.id.clone(), item);
                vec![Effect::Notify(ZonesEvent::ItemsChanged { zone }), Effect::SaveState]
            }
            BackendEvent::ItemChanged { location: Location::Desktop, item } => {
                let zone = self.zone_of(&item.id).cloned();
                self.items.insert(item.id.clone(), item);
                vec![Effect::Notify(ZonesEvent::ItemsChanged { zone })]
            }
            BackendEvent::ItemAdded { location: Location::Directory { path }, item }
            | BackendEvent::ItemChanged { location: Location::Directory { path }, item } => {
                if !self.watched.contains(&path) {
                    return Vec::new();
                }
                self.directories.entry(path.clone()).or_default().insert(item.id.clone(), item);
                self.portals_changed(&path)
            }
            BackendEvent::ItemRemoved { location: Location::Directory { path }, item } => {
                if let Some(items) = self.directories.get_mut(&path) {
                    items.remove(&item);
                }
                self.portals_changed(&path)
            }
            BackendEvent::ItemRenamed { location: Location::Directory { path }, from, item } => {
                if !self.watched.contains(&path) {
                    return Vec::new();
                }
                let items = self.directories.entry(path.clone()).or_default();
                items.remove(&from);
                items.insert(item.id.clone(), item);
                self.portals_changed(&path)
            }
            BackendEvent::Displays { displays, reserved } => {
                // No display at all: the machine sleeps or the session is
                // locked. Everything stays as it is until one is back.
                if displays.is_empty() {
                    return Vec::new();
                }
                self.displays = displays;
                self.reserved = reserved;
                let mut effects = self.choose_profile();
                effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
                effects
            }
            BackendEvent::Workspace { name } => {
                if self.workspace == name {
                    return Vec::new();
                }
                self.workspace = name;
                if !self.config.profile_per_workspace {
                    return Vec::new();
                }
                let mut effects = self.choose_profile();
                effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
                effects
            }
            BackendEvent::OperationDone { request, undo, error } => self.operation_done(request, undo, error),
            BackendEvent::IconReady { .. } => Vec::new(),
        }
    }

    /// The zones that show a folder have other items.
    fn portals_changed(&self, directory: &str) -> Vec<Effect> {
        self.config
            .zones
            .iter()
            .filter(|zone| matches!(&zone.kind, ZoneKind::Portal { directory: shown } if shown == directory))
            .map(|zone| Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone.id.clone()) }))
            .collect()
    }

    /// An item came to the desktop. Where the configuration says so, the
    /// zone its rule names takes it at once.
    fn arrived(&mut self, item: Item, now: u64) -> Vec<Effect> {
        let id = item.id.clone();
        let mut effects = Vec::new();
        if self.config.enabled && self.config.rules_on_arrival && self.zone_of(&id).is_none() {
            let decided = rules::decide(&self.config.rules, self.config.catch_all.as_ref(), &item, now);
            if let Some((_, RuleAction::AssignToZone { zone })) = decided {
                let takes = self
                    .config
                    .zone(zone)
                    .is_some_and(|zone| zone.kind == ZoneKind::Manual && !zone.lock.members);
                if takes {
                    let zone = zone.clone();
                    self.state.placements.assign(&id, &zone, None, None);
                    effects.push(Effect::SaveState);
                }
            }
        }
        self.items.insert(id.clone(), item);
        let zone = self.zone_of(&id).cloned();
        effects.insert(0, Effect::Notify(ZonesEvent::ItemsChanged { zone }));
        effects
    }

    fn operation_done(
        &mut self,
        request: u64,
        undo: Option<FileOperation>,
        error: Option<BackendError>,
    ) -> Vec<Effect> {
        let Some(pending) = self.pending.remove(&request) else {
            return Vec::new();
        };
        match (pending, error) {
            (Pending::RuleMove { item, operation }, Some(error)) => {
                // The file is still where it was: so is its place, and
                // there is nothing of it to take back.
                if let Some(record) = &mut self.state.undo {
                    if let Some(at) = record.entries.iter().position(|entry| entry.item == item) {
                        let entry = record.entries.remove(at);
                        self.state.placements.put_back(&item, &entry.before);
                    }
                    record.moved.retain(|moved| moved.item != item);
                }
                vec![
                    Effect::Notify(ZonesEvent::OperationFailed { operation, error }),
                    Effect::SaveState,
                ]
            }
            (Pending::RuleMove { item, .. }, None) => {
                let Some(undo) = undo else {
                    return Vec::new();
                };
                let moved = self
                    .state
                    .undo
                    .as_mut()
                    .and_then(|record| record.moved.iter_mut().find(|moved| moved.item == item));
                match moved {
                    Some(moved) if moved.undo != undo => {
                        moved.undo = undo;
                        vec![Effect::SaveState]
                    }
                    _ => Vec::new(),
                }
            }
            (Pending::Portal { operation } | Pending::TakingBack { operation }, Some(error)) => {
                vec![Effect::Notify(ZonesEvent::OperationFailed { operation, error })]
            }
            (Pending::Portal { operation }, None) => {
                if let Some(undo) = undo.or_else(|| operation.reversed()) {
                    self.file_undo.push(undo);
                    if self.file_undo.len() > FILE_UNDO_DEPTH {
                        self.file_undo.remove(0);
                    }
                }
                Vec::new()
            }
            (Pending::TakingBack { .. }, None) => Vec::new(),
        }
    }

    // ---- what the person asks for -------------------------------------

    /// Carry out a command. One that cannot be carried out changes
    /// nothing and gives `ZonesEvent::Refused` with the reason.
    pub fn command(&mut self, command: Command, now: u64) -> Vec<Effect> {
        if !self.config.enabled {
            let reason = "zones are switched off (desktop.zones.enabled)".to_string();
            return vec![Effect::Notify(ZonesEvent::Refused { reason })];
        }
        let outcome = match command {
            Command::CreateZone { title, kind, rect } => self.create_zone(title, kind, rect),
            Command::RenameZone { zone, title } => self.change_zone(&zone, |zone| zone.title = title),
            Command::DeleteZone { zone } => self.delete_zone(&zone),
            Command::MoveZone { zone, x, y, snap } => self.move_zone(&zone, x, y, snap),
            Command::ResizeZone { zone, rect, edges, snap } => self.resize_zone(&zone, rect, edges, snap),
            Command::AddItem { zone, item, index, cell } => self.add_item(&zone, &item, index, cell),
            Command::RemoveItem { item } => self.loosen_item(&item, None),
            Command::PlaceLooseItem { item, display, x, y } => {
                self.loosen_item(&item, Some(LoosePosition { display, x, y }))
            }
            Command::RollUp { zone, rolled_up } => self.roll_up(&zone, rolled_up),
            Command::Lock { zone, geometry, members } => self.change_zone(&zone, |zone| {
                zone.lock.geometry = geometry.unwrap_or(zone.lock.geometry);
                zone.lock.members = members.unwrap_or(zone.lock.members);
            }),
            Command::Tab { zone, with } => self.tab(&zone, &with),
            Command::Untab { zone } => self.untab(&zone),
            Command::ActivateTab { zone } => self.activate_tab(&zone),
            Command::SetView { zone, view, sort, descending, icon_size } => {
                self.change_zone(&zone, |zone| {
                    zone.view = view.unwrap_or(zone.view);
                    zone.sort = sort.unwrap_or(zone.sort);
                    zone.sort_descending = descending.unwrap_or(zone.sort_descending);
                    zone.icon_size = icon_size.unwrap_or(zone.icon_size).clamp(16, 256);
                })
            }
            Command::ShowZone { zone, shown } => self.show_zone(&zone, shown),
            Command::PreviewRules { scope } => {
                Ok(vec![Effect::Notify(ZonesEvent::RulesPreviewed { plan: self.plan(scope, now) })])
            }
            Command::ApplyRules { scope } => self.apply_rules(scope, now),
            Command::UndoRules => self.undo_rules(),
            Command::UndoFileOperation => match self.file_undo.pop() {
                Some(operation) => Ok(vec![self.ask(Pending::TakingBack { operation })]),
                None => Err("no file operation to take back".to_string()),
            },
            Command::SaveSnapshot { name } => self.save_snapshot(name, now),
            Command::RestoreSnapshot { name } => self.restore_snapshot(&name),
            Command::DeleteSnapshot { name } => {
                let before = self.state.snapshots.len();
                self.state.snapshots.retain(|snapshot| snapshot.name != name);
                if self.state.snapshots.len() == before {
                    Err(format!("no snapshot is called {name}"))
                } else {
                    Ok(vec![Effect::Notify(ZonesEvent::SnapshotDeleted { name }), Effect::SaveState])
                }
            }
            Command::QuickHide { hidden } => {
                self.quick_hidden = hidden.unwrap_or(!self.quick_hidden);
                Ok(vec![self.visibility()])
            }
            Command::Peek { on } => {
                self.peeking = on;
                Ok(vec![self.visibility()])
            }
            Command::Search { typed } => {
                let hits = self.search(&typed);
                Ok(vec![Effect::Notify(ZonesEvent::Found { typed, hits })])
            }
        };
        outcome.unwrap_or_else(|reason| vec![Effect::Notify(ZonesEvent::Refused { reason })])
    }

    fn visibility(&self) -> Effect {
        Effect::Notify(ZonesEvent::VisibilityChanged {
            quick_hidden: self.quick_hidden,
            peeking: self.peeking,
        })
    }

    fn known(&self, zone: &ZoneId) -> Result<&ZoneDefinition, String> {
        self.config.zone(zone).ok_or_else(|| format!("no zone is called {zone}"))
    }

    /// Change something of a zone's own in the configuration.
    fn change_zone(&mut self, zone: &ZoneId, change: impl FnOnce(&mut ZoneDefinition)) -> Outcome {
        let before = self.known(zone)?.clone();
        let Some(definition) = self.config.zone_mut(zone) else {
            return Ok(Vec::new());
        };
        change(definition);
        if *definition == before {
            return Ok(Vec::new());
        }
        Ok(vec![Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }), Effect::SaveConfig])
    }

    fn create_zone(&mut self, title: String, kind: ZoneKind, rect: Option<Rect>) -> Outcome {
        if self.current.is_none() {
            return Err("no display is known yet".to_string());
        }
        let id = loop {
            self.state.zones_made += 1;
            let id = ZoneId::new(format!("zone-{}", self.state.zones_made));
            if self.config.zone(&id).is_none() {
                break id;
            }
        };
        let geometry = match rect {
            Some(rect) => {
                let display = display_of(rect, &self.displays).ok_or("no display is known yet")?;
                let (least_width, least_height) = self.metrics.least();
                let sized = Rect::new(rect.x, rect.y, rect.width.max(least_width), rect.height.max(least_height));
                let rect = snap::keep_reachable(sized, work_area(display, &self.reserved));
                ZoneGeometry::nearest(rect, display)
            }
            None => {
                let already = self.profile().map_or(0, |profile| profile.geometry.len());
                self.default_geometry(already).ok_or("no display is known yet")?
            }
        };
        self.config.zones.push(ZoneDefinition { id: id.clone(), title, kind, ..ZoneDefinition::default() });
        // A zone that was here under this name once left nothing behind.
        self.state.placements.drop_zone(&id);
        if let Some(profile) = self.profile_mut() {
            profile.geometry.insert(id.clone(), geometry);
            profile.hidden.remove(&id);
        }
        let mut effects = vec![Effect::Notify(ZonesEvent::ZoneCreated { zone: id })];
        effects.extend(self.sync_watches());
        effects.extend([Effect::SaveConfig, Effect::SaveState]);
        Ok(effects)
    }

    fn delete_zone(&mut self, zone: &ZoneId) -> Outcome {
        let group = self.known(zone)?.tab_group.clone();
        self.config.zones.retain(|definition| &definition.id != zone);
        self.state.placements.drop_zone(zone);
        for profile in &mut self.state.profiles {
            profile.geometry.remove(zone);
            profile.hidden.remove(zone);
        }
        self.dissolve_lone_tab(group);
        self.reconcile();
        let mut effects = vec![Effect::Notify(ZonesEvent::ZoneDeleted { zone: zone.clone() })];
        effects.extend(self.sync_watches());
        effects.extend([Effect::SaveConfig, Effect::SaveState]);
        Ok(effects)
    }

    /// A group that has one tab left is no group.
    fn dissolve_lone_tab(&mut self, group: Option<String>) {
        let Some(group) = group else {
            return;
        };
        let mut members = self
            .config
            .zones
            .iter_mut()
            .filter(|zone| zone.tab_group.as_ref() == Some(&group));
        if let (Some(last), None) = (members.next(), members.next()) {
            last.tab_group = None;
            self.state.active_tabs.remove(&group);
        }
    }

    /// The rectangles of the other zones shown on a display, for snapping.
    fn others_on(&self, display: &DisplayId, but: &[ZoneId]) -> Vec<Rect> {
        let mut seen = BTreeSet::new();
        self.config
            .zones
            .iter()
            .filter(|zone| !but.contains(&zone.id) && self.is_shown(&zone.id))
            // (Tabs share one place: once is enough.)
            .filter(|zone| seen.insert(self.leader(&zone.id)))
            .filter(|zone| self.place_of(&zone.id).is_some_and(|(_, on)| &on.id == display))
            .filter_map(|zone| self.zone_rect(&zone.id))
            .collect()
    }

    /// Give a zone, and the zones that share its place, a rectangle on a
    /// display.
    fn place(&mut self, zone: &ZoneId, rect: Rect, display: &Display) -> Vec<Effect> {
        let geometry = ZoneGeometry::nearest(rect, display);
        let companions = self.companions(zone);
        if let Some(profile) = self.profile_mut() {
            for companion in &companions {
                profile.geometry.insert(companion.clone(), geometry.clone());
            }
        }
        let mut effects: Vec<Effect> = companions
            .into_iter()
            .filter_map(|companion| {
                let rect = self.zone_rect(&companion)?;
                Some(Effect::Notify(ZonesEvent::ZonePlaced { zone: companion, rect }))
            })
            .collect();
        effects.push(Effect::SaveState);
        effects
    }

    fn movable(&self, zone: &ZoneId) -> Result<(Rect, Display), String> {
        self.known(zone)?;
        let companions = self.companions(zone);
        if companions.iter().any(|zone| self.config.zone(zone).is_some_and(|zone| zone.lock.geometry)) {
            return Err(format!("{zone} is locked where it is"));
        }
        let (rect, display) = self.place_of(zone).ok_or("no display is known yet")?;
        Ok((rect, display.clone()))
    }

    fn move_zone(&mut self, zone: &ZoneId, x: i32, y: i32, snapping: bool) -> Outcome {
        let (whole, _) = self.movable(zone)?;
        let wanted = Rect::new(x, y, whole.width, whole.height);
        let display = display_of(wanted, &self.displays).ok_or("no display is known yet")?.clone();
        let area = work_area(&display, &self.reserved);
        let rest = if snapping {
            let others = self.others_on(&display.id, &self.companions(zone));
            let around = Surroundings { display: display.rect, work_area: area, others: &others };
            snap::snap_move(wanted, &around, &self.config.snap)
        } else {
            snap::keep_reachable(wanted, area)
        };
        Ok(self.place(zone, rest, &display))
    }

    fn resize_zone(&mut self, zone: &ZoneId, rect: Rect, edges: Edges, snapping: bool) -> Outcome {
        let (_, display) = self.movable(zone)?;
        let area = work_area(&display, &self.reserved);
        let others = self.others_on(&display.id, &self.companions(zone));
        let around = Surroundings { display: display.rect, work_area: area, others: &others };
        let settings = crate::config::SnapSettings {
            enabled: snapping && self.config.snap.enabled,
            ..self.config.snap.clone()
        };
        let sized = snap::snap_resize(rect, edges, &around, &settings, self.metrics.least());
        Ok(self.place(zone, snap::keep_reachable(sized, area), &display))
    }

    fn roll_up(&mut self, zone: &ZoneId, rolled_up: Option<bool>) -> Outcome {
        let rolled_up = rolled_up.unwrap_or(!self.known(zone)?.rolled_up);
        let mut effects = Vec::new();
        // Tabs roll up together: they are one thing on the desktop.
        for companion in self.companions(zone) {
            effects.extend(self.change_zone(&companion, |zone| zone.rolled_up = rolled_up)?);
        }
        Ok(effects)
    }

    fn add_item(&mut self, zone: &ZoneId, item: &ItemId, index: Option<usize>, cell: Option<Cell>) -> Outcome {
        let target = self.known(zone)?.clone();
        if target.lock.members {
            return Err(format!("{zone} is locked: nothing goes in or out"));
        }
        let from = self.zone_of(item).cloned();
        if from.as_ref().and_then(|from| self.config.zone(from)).is_some_and(|from| from.lock.members) {
            return Err(format!("{item} is in a locked zone"));
        }
        match &target.kind {
            ZoneKind::Manual => {
                if !self.items.contains_key(item) {
                    return Err(format!("{item} is not on the desktop"));
                }
                self.state.placements.assign(item, zone, index, cell);
                let mut effects = vec![Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone.clone()) })];
                if from.as_ref() != Some(zone) {
                    effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: from }));
                }
                effects.push(Effect::SaveState);
                Ok(effects)
            }
            ZoneKind::Portal { directory } => {
                let path = self
                    .item(item)
                    .and_then(|item| item.path.clone())
                    .ok_or_else(|| format!("{item} is no file that could be moved"))?;
                if paths::parent(&path).is_some_and(|at| paths::same_folder(at, directory)) {
                    return Err(format!("{item} is in that folder already"));
                }
                let operation = FileOperation::Move { from: path, to_folder: directory.clone() };
                Ok(vec![self.ask(Pending::Portal { operation })])
            }
            ZoneKind::Smart { .. } => Err(format!("{zone} fills itself")),
        }
    }

    fn loosen_item(&mut self, item: &ItemId, position: Option<LoosePosition>) -> Outcome {
        if !self.items.contains_key(item) {
            return Err(format!("{item} is not on the desktop"));
        }
        let from = self.zone_of(item).cloned();
        if from.as_ref().and_then(|from| self.config.zone(from)).is_some_and(|from| from.lock.members) {
            return Err(format!("{item} is in a locked zone"));
        }
        if let Some(position) = &position {
            if !self.displays.iter().any(|display| display.id == position.display) {
                return Err(format!("no display is called {}", position.display));
            }
        }
        if from.is_none() && position.is_none() && !self.state.placements.loose.contains_key(item) {
            return Ok(Vec::new());
        }
        self.state.placements.loosen(item, position);
        let mut effects = Vec::new();
        if from.is_some() {
            effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: from }));
        }
        effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: None }));
        effects.push(Effect::SaveState);
        Ok(effects)
    }

    fn tab(&mut self, zone: &ZoneId, with: &ZoneId) -> Outcome {
        if zone == with {
            return Err("a zone is no tab of itself".to_string());
        }
        self.known(zone)?;
        self.known(with)?;
        self.movable(zone)?;
        self.movable(with)?;
        let leader = self.leader(with);
        let geometry = self
            .profile()
            .and_then(|profile| profile.geometry.get(&leader).cloned())
            .ok_or("no display is known yet")?;
        let left = self.config.zone(zone).and_then(|zone| zone.tab_group.clone());
        let group = match self.config.zone(with).and_then(|with| with.tab_group.clone()) {
            Some(group) => group,
            None => {
                let mut number = 1;
                loop {
                    let name = format!("tabs-{number}");
                    if !self.config.zones.iter().any(|zone| zone.tab_group.as_ref() == Some(&name)) {
                        break name;
                    }
                    number += 1;
                }
            }
        };
        if left.as_ref() == Some(&group) {
            return Ok(Vec::new());
        }
        let rolled_up = self.config.zone(with).is_some_and(|with| with.rolled_up);
        for id in [zone, with] {
            if let Some(definition) = self.config.zone_mut(id) {
                definition.tab_group = Some(group.clone());
                definition.rolled_up = rolled_up;
            }
        }
        self.dissolve_lone_tab(left);
        if let Some(profile) = self.profile_mut() {
            profile.geometry.insert(zone.clone(), geometry);
        }
        self.state.active_tabs.insert(group, zone.clone());
        self.reconcile();
        Ok(vec![
            Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }),
            Effect::Notify(ZonesEvent::ZoneChanged { zone: with.clone() }),
            Effect::SaveConfig,
            Effect::SaveState,
        ])
    }

    fn untab(&mut self, zone: &ZoneId) -> Outcome {
        let group = self.known(zone)?.tab_group.clone().ok_or_else(|| format!("{zone} is no tab"))?;
        let place = self.place_of(zone).map(|(rect, display)| (rect, display.clone()));
        if let Some(definition) = self.config.zone_mut(zone) {
            definition.tab_group = None;
        }
        if self.state.active_tabs.get(&group) == Some(zone) {
            self.state.active_tabs.remove(&group);
        }
        self.dissolve_lone_tab(Some(group));
        // It stands a step beside where the tabs are, so that both are seen.
        if let Some((rect, display)) = place {
            let beside = snap::keep_reachable(
                Rect::new(rect.x + 24, rect.y + 24, rect.width, rect.height),
                work_area(&display, &self.reserved),
            );
            let geometry = ZoneGeometry::nearest(beside, &display);
            if let Some(profile) = self.profile_mut() {
                profile.geometry.insert(zone.clone(), geometry);
            }
        }
        self.reconcile();
        Ok(vec![
            Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }),
            Effect::Notify(ZonesEvent::LayoutChanged),
            Effect::SaveConfig,
            Effect::SaveState,
        ])
    }

    fn activate_tab(&mut self, zone: &ZoneId) -> Outcome {
        let group = self.known(zone)?.tab_group.clone().ok_or_else(|| format!("{zone} is no tab"))?;
        if self.state.active_tabs.insert(group, zone.clone()).as_ref() == Some(zone) {
            return Ok(Vec::new());
        }
        Ok(vec![Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }), Effect::SaveState])
    }

    fn show_zone(&mut self, zone: &ZoneId, shown: bool) -> Outcome {
        self.known(zone)?;
        let profile = self.profile_mut().ok_or("no display is known yet")?;
        let changed = if shown { profile.hidden.remove(zone) } else { profile.hidden.insert(zone.clone()) };
        if !changed {
            return Ok(Vec::new());
        }
        Ok(vec![Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }), Effect::SaveState])
    }

    /// What the rules would do now.
    pub fn plan(&self, scope: Scope, now: u64) -> Plan {
        rules::plan(&self.config, self.items.values(), &self.state.placements, scope, now)
    }

    fn apply_rules(&mut self, scope: Scope, now: u64) -> Outcome {
        let plan = self.plan(scope, now);
        if plan.steps.is_empty() {
            // Nothing to do, and nothing to take back that was not there before.
            return Ok(vec![Effect::Notify(ZonesEvent::RulesApplied { plan })]);
        }
        let items = &self.items;
        let path_of = |id: &ItemId| items.get(id).and_then(|item| item.path.clone());
        let applied = rules::apply(&plan, &path_of, &mut self.state.placements, now);
        let mut effects = vec![Effect::Notify(ZonesEvent::RulesApplied { plan })];
        for moved in &applied.undo.moved {
            effects.push(self.ask(Pending::RuleMove {
                item: moved.item.clone(),
                operation: moved.operation.clone(),
            }));
        }
        self.state.undo = Some(applied.undo);
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        effects.push(Effect::SaveState);
        Ok(effects)
    }

    fn undo_rules(&mut self) -> Outcome {
        let record = self.state.undo.take().ok_or("the rules were not applied, or it was taken back already")?;
        let operations = rules::undo(&record, &mut self.state.placements);
        let mut effects = vec![Effect::Notify(ZonesEvent::RulesUndone)];
        for operation in operations {
            effects.push(self.ask(Pending::TakingBack { operation }));
        }
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        effects.push(Effect::SaveState);
        Ok(effects)
    }

    fn save_snapshot(&mut self, name: String, now: u64) -> Outcome {
        if name.trim().is_empty() {
            return Err("a snapshot needs a name".to_string());
        }
        let snapshot = Snapshot {
            name: name.clone(),
            taken_at: now,
            zones: self.config.zones.clone(),
            rules: self.config.rules.clone(),
            catch_all: self.config.catch_all.clone(),
            placements: self.state.placements.clone(),
            profiles: self.state.profiles.clone(),
            active_tabs: self.state.active_tabs.clone(),
            extra: Default::default(),
        };
        match self.state.snapshots.iter_mut().find(|snapshot| snapshot.name == name) {
            Some(existing) => *existing = snapshot,
            None => self.state.snapshots.push(snapshot),
        }
        Ok(vec![Effect::Notify(ZonesEvent::SnapshotSaved { name }), Effect::SaveState])
    }

    fn restore_snapshot(&mut self, name: &str) -> Outcome {
        let snapshot = self
            .state
            .snapshots
            .iter()
            .find(|snapshot| snapshot.name == name)
            .cloned()
            .ok_or_else(|| format!("no snapshot is called {name}"))?;
        self.config.zones = snapshot.zones;
        self.config.rules = snapshot.rules;
        self.config.catch_all = snapshot.catch_all;
        self.state.placements = snapshot.placements;
        self.state.profiles = snapshot.profiles;
        self.state.active_tabs = snapshot.active_tabs;
        // What the rules did before no longer describes what is there.
        self.state.undo = None;
        self.current = None;
        let mut effects = vec![Effect::Notify(ZonesEvent::SnapshotRestored { name: name.to_string() })];
        effects.extend(self.sync_watches());
        self.choose_profile();
        self.reconcile();
        effects.extend([
            Effect::Notify(ZonesEvent::LayoutChanged),
            Effect::SaveConfig,
            Effect::SaveState,
        ]);
        Ok(effects)
    }
}
