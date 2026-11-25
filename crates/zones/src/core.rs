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
//!
//! Two kinds of time come in. `now`, with every call, is the time of the
//! day in seconds since 1970: rules ask it how old a file is. What is
//! measured in parts of a second (how long the pointer has rested on a
//! rolled-up zone) comes as `at_ms` with the commands that need it: any
//! count of thousandths of a second that only goes forward. The core
//! says when it wants to be called again (`wake_at_ms`) and keeps no
//! timer of its own.
//!
//! Beside the configuration and the state there is what is so only for
//! the moment (`ViewState`): which rolled-up zone is open, where a portal
//! stands, what is typed into a zone's filter, what has the keyboard,
//! what is covered from view. It is never written anywhere, and the
//! parts of this module that deal with it are `shown`, `folders`, `keys`
//! and `layouts`. (`auto` is the zones made by type, which are kept.)

mod auto;
mod folders;
mod keys;
mod layouts;
mod shown;

pub use layouts::merge;

use crate::auto::AutoPlan;
use crate::backend::{BackendError, BackendEvent, BackendRequest, FileOperation, Location};
use crate::config::{
    ExpandOn, Lock, RuleAction, SmartSource, Sort, View, ZoneDefinition, ZoneKind, ZoneLook,
    ZonesConfig,
};
use crate::export::LayoutDocument;
use crate::focus::{Direction, Focus, SelectMode, Step};
use crate::geometry::{
    display_of, fallback_display, work_area, Display, ReservedArea, ZoneGeometry,
};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::layout::{self, ItemLayout, Metrics, PlacedItem};
use crate::paths;
use crate::placement::{Cell, LoosePosition};
use crate::portal::{Listing, PortalScene, PortalView};
use crate::profile::{self, Profile, ProfileKey};
use crate::reserve::ReservedRegion;
use crate::rules::{self, Plan, PlanLine, Scope};
use crate::search::{self, Hit};
use crate::snap::{self, Edges, Surroundings};
use crate::state::ZonesState;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::focus::DESK;

/// In a command, in place of a zone's id: the zone that has the keyboard.
/// So one key binding rolls up, locks or hides whichever zone the person
/// is in (`{"command": "roll_up", "zone": "@focused"}`).
pub const FOCUSED: &str = "@focused";

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
    RenameZone {
        zone: ZoneId,
        title: String,
    },
    /// Take a zone away. Its items stay on the desktop, loose.
    DeleteZone {
        zone: ZoneId,
    },
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
    RemoveItem {
        item: ItemId,
    },
    /// Put an item on the bare desktop, at a place of its own.
    PlaceLooseItem {
        item: ItemId,
        display: DisplayId,
        x: i32,
        y: i32,
    },
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
    Tab {
        zone: ZoneId,
        with: ZoneId,
    },
    /// Take a zone out of its tabs; it stands beside them.
    Untab {
        zone: ZoneId,
    },
    /// Bring a tab to the front.
    ActivateTab {
        zone: ZoneId,
    },
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
    ShowZone {
        zone: ZoneId,
        shown: bool,
    },
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
    /// Take back the last file operation (of a portal, of the bin, of
    /// files brought in).
    UndoFileOperation,
    /// Give an item's file another name. It can be taken back with
    /// `UndoFileOperation`.
    RenameItem {
        item: ItemId,
        name: String,
    },
    /// Put items into the system's bin (never deleted outright). It can
    /// be taken back with `UndoFileOperation`.
    TrashItems {
        items: Vec<ItemId>,
    },
    /// Bring files from elsewhere to the desktop: copied or moved into
    /// `folder` (the desktop's own, which the backend names), and each
    /// put into a zone or at a place of its own once it is there.
    BringIn {
        paths: Vec<String>,
        folder: String,
        /// Copied, not moved.
        #[serde(default)]
        copy: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<LoosePosition>,
    },
    SaveSnapshot {
        name: String,
    },
    RestoreSnapshot {
        name: String,
    },
    DeleteSnapshot {
        name: String,
    },
    /// Hide every zone that is not always visible, or show them again.
    QuickHide {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<bool>,
    },
    /// Bring the zones above the windows while a key is held.
    Peek {
        on: bool,
    },
    Search {
        typed: String,
    },

    // ---- roll-up ----
    /// The pointer is over a zone, or over none. A rolled-up zone it
    /// rests on opens after a moment where zones open on hover (always
    /// while something is `dragging`, so that it can be dropped inside);
    /// one it has left closes after a moment. Tabs count as one.
    PointerOver {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        at_ms: u64,
        #[serde(default)]
        dragging: bool,
    },
    /// Open a rolled-up zone for now, or close it again; without a value,
    /// the other of what it is. What a press on its title does. Nothing
    /// kept changes: the zone is rolled up as before.
    Expand {
        zone: ZoneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expanded: Option<bool>,
    },
    /// Time has passed: do what was waiting for it (`wake_at_ms`).
    Tick {
        at_ms: u64,
    },

    // ---- tabs ----
    /// Put a tab at another place among its tabs, counted from 0.
    MoveTab {
        zone: ZoneId,
        index: usize,
    },
    /// Take a tab out of its tabs and put its top left corner at a point
    /// of the desktop: what dragging a tab out does.
    SplitTab {
        zone: ZoneId,
        x: i32,
        y: i32,
        #[serde(default = "yes")]
        snap: bool,
    },
    /// Bring the next tab of a zone's tabs to the front (the one before,
    /// `backwards`), going round at the ends.
    NextTab {
        zone: ZoneId,
        #[serde(default)]
        backwards: bool,
    },

    // ---- hiding, peeking, opening ----
    /// A zone stays when the others are put away with the one key;
    /// without a value, the other of what it is.
    AlwaysVisible {
        zone: ZoneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
    },
    /// An item stays when the others are put away, whichever zone it is
    /// in or if it is in none.
    ItemAlwaysVisible {
        item: ItemId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
    },
    /// Peek, or stop peeking: what the peek key does when it is pressed
    /// rather than held.
    TogglePeek,
    /// What Escape does: stop peeking; else empty the filter of the zone
    /// that has the keyboard; else choose nothing; else let the keyboard
    /// go.
    Dismiss,
    /// Open items as a double click does. Whoever runs the zones starts
    /// them (`ZonesEvent::OpenRequested`); peeking ends. One folder of a
    /// portal is gone into instead, in place, unless `in_place` is false.
    Open {
        items: Vec<ItemId>,
        /// The program to open them with.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        with: Option<String>,
        #[serde(default = "yes")]
        in_place: bool,
    },

    // ---- lock, look and everything else of the configuration ----
    /// Lock every zone at once, whatever its own lock says; what is not
    /// given stays, and with neither given both are turned to the other
    /// of what they are.
    LockAll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geometry: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        members: Option<bool>,
    },
    /// Change what a zone is configured as: `changes` is laid over the
    /// zone's definition (a JSON merge patch; `null` puts a key back to
    /// its default). Its look, its kind, its source, whether it is
    /// private, the workspaces it is on. The id stays.
    ModifyZone {
        zone: ZoneId,
        changes: serde_json::Value,
    },
    /// The same for what is set for all zones (`look`, `snap`, `roll_up`,
    /// `lock`, `privacy`, `rules`...). The zones themselves are changed
    /// one by one, not here.
    ModifySettings {
        changes: serde_json::Value,
    },

    // ---- portals ----
    /// Go into a folder a portal shows, without leaving the zone.
    EnterFolder {
        zone: ZoneId,
        item: ItemId,
    },
    /// Go to a folder at or below the portal's own (a press on a part of
    /// the way shown above its items).
    NavigateTo {
        zone: ZoneId,
        directory: String,
    },
    NavigateUp {
        zone: ZoneId,
    },
    NavigateBack {
        zone: ZoneId,
    },
    NavigateForward {
        zone: ZoneId,
    },
    /// Ask again what a zone's folder or source has (it was not
    /// reachable, and may be now).
    RefreshZone {
        zone: ZoneId,
    },
    /// Make a folder in the folder a portal shows. It can be taken back
    /// with `UndoFileOperation`.
    NewFolder {
        zone: ZoneId,
        name: String,
    },
    /// Make a folder in any folder: on the desktop itself, with the
    /// desktop's own folder (which the backend names). It can be taken
    /// back with `UndoFileOperation` like the one above.
    NewFolderIn {
        folder: String,
        name: String,
    },
    /// Move files (or copy them) into a folder: an item of a portal onto
    /// a folder in it, or out of it onto the desktop. Each can be taken
    /// back with `UndoFileOperation`.
    MoveItemsToFolder {
        items: Vec<ItemId>,
        folder: String,
        #[serde(default)]
        copy: bool,
    },
    /// Show only the items of a zone whose names fit what was typed;
    /// nothing typed shows all again. Without a zone: the one that has
    /// the keyboard.
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        typed: String,
    },

    // ---- rules ----
    /// Try the rules that ask how old an item is, and carry out what
    /// they say now. Whoever runs the zones asks this on a schedule
    /// (`timed_rules_every`). It can be taken back with `UndoRules`.
    CheckTimedRules,

    // ---- whole layouts ----
    RenameSnapshot {
        name: String,
        to: String,
    },
    /// Say a whole layout as one document (`ZonesEvent::LayoutExported`):
    /// the saved one of that name, or without a name the layout as it is.
    ExportLayout {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Take a layout document in: it is kept as a snapshot, and put in
    /// force unless `apply` is false. The layout there was is kept as a
    /// snapshot first, so nothing is lost. A document that cannot be
    /// right changes nothing.
    ImportLayout {
        document: serde_json::Value,
        #[serde(default = "yes")]
        apply: bool,
    },
    /// Arrange the zones as they are kept for another set of displays.
    CopyArrangement {
        from: ProfileKey,
    },

    // ---- the keyboard ----
    /// Give a zone the keyboard; without a zone, let it go.
    FocusZone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
    },
    /// Give the keyboard to the zone that lies in a direction from the
    /// one that has it; from none, to the first.
    FocusNeighbour {
        direction: Direction,
    },
    /// Give an item the keyboard, in whichever zone shows it: its tab
    /// comes to the front and a rolled-up zone opens. Without an item,
    /// the zone keeps the keyboard and no item has it.
    FocusItem {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
    },
    /// Go from the item that has the keyboard to one beside it. It is
    /// the chosen one then; with `extend`, chosen as well as the others.
    MoveItemFocus {
        step: Step,
        #[serde(default)]
        extend: bool,
    },
    /// Choose items.
    Select {
        items: Vec<ItemId>,
        #[serde(default)]
        mode: SelectMode,
    },
    /// Open the chosen items, or the one that has the keyboard.
    OpenFocused,
    /// Put the chosen items (or the one that has the keyboard) into
    /// another zone: the one named, or the one that lies in a direction.
    MoveFocusedItems {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<Direction>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
    },

    /// Call off a file operation that has not ended (`Scene.operations`
    /// names them).
    CancelOperation {
        request: u64,
    },

    // ---- privacy ----
    /// Cover what every zone holds, and the loose items; or stop doing
    /// so. Without a value, the other of what it is.
    Mask {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        masked: Option<bool>,
    },
    /// Show what a covered zone holds (or cover it again); without a
    /// zone, everything. It lasts until the screen is next shared or
    /// everything is covered by hand.
    Reveal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revealed: Option<bool>,
    },

    // ---- the whole desktop ----
    /// Draw every icon grey, or in its colours again; without a value,
    /// the other of what it is. A way of looking at the desktop: the
    /// looks the person set stay as they are, and this is laid over them.
    /// It is remembered, as the one key's putting away is.
    GreyIcons {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
    },
    /// Make a zone for each type of item (`desktop.zones.auto`) and put
    /// the items into them; a type that has its zone already gets what is
    /// new of it. `dry_run` says what would be done
    /// (`ZonesEvent::AutoPlanned`) and does nothing. What is not given is
    /// taken from the configuration; without a display the zones are made
    /// on the primary one. It can be taken back with `UndoAutoCreate`.
    AutoCreate {
        /// `loose`: the items that are in no zone. `all`: also the items
        /// of the zones filled by hand (not of those made by type).
        #[serde(default)]
        scope: Scope,
        /// How many items a type needs for a zone of its own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<DisplayId>,
        /// With each zone that is made, a rule for what comes later.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        add_rules: Option<bool>,
        #[serde(default)]
        dry_run: bool,
    },
    /// Take back the last making of zones by type: its zones, where it
    /// put the items, the rules it added.
    UndoAutoCreate,
}

/// What happened, for a renderer and for whoever listens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ZonesEvent {
    ZoneCreated {
        zone: ZoneId,
    },
    /// Something of the zone itself is other than it was (title, view,
    /// lock, rolled up, tabs, shown).
    ZoneChanged {
        zone: ZoneId,
    },
    ZoneDeleted {
        zone: ZoneId,
    },
    /// The zone is at this rectangle of the desktop now.
    ZonePlaced {
        zone: ZoneId,
        rect: Rect,
    },
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
    ProfileChanged {
        made: bool,
        moved: Vec<ZoneId>,
    },
    /// What the rules would do. `lines` say the same as the plan in the
    /// words a person is shown: which item, from where, to where, by
    /// which rule; and what is held back, and why.
    RulesPreviewed {
        plan: Plan,
        #[serde(default)]
        lines: Vec<PlanLine>,
    },
    RulesApplied {
        plan: Plan,
        #[serde(default)]
        lines: Vec<PlanLine>,
    },
    RulesUndone,
    SnapshotSaved {
        name: String,
    },
    SnapshotRestored {
        name: String,
    },
    SnapshotDeleted {
        name: String,
    },
    VisibilityChanged {
        quick_hidden: bool,
        peeking: bool,
    },
    Found {
        typed: String,
        hits: Vec<Hit>,
    },
    /// The backend could not do what was asked of it.
    OperationFailed {
        operation: FileOperation,
        error: BackendError,
    },
    /// A command was not carried out, and why.
    Refused {
        reason: String,
    },
    /// A rolled-up zone is open for now, or closed again. Its rectangle
    /// is the whole zone's while it is open.
    ZoneExpanded {
        zone: ZoneId,
        expanded: bool,
    },
    /// Items are to be opened; whoever runs the zones starts them.
    OpenRequested {
        items: Vec<ItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        with: Option<String>,
    },
    /// Something set for all zones is other than it was.
    SettingsChanged,
    /// A portal shows another folder.
    Navigated {
        zone: ZoneId,
        directory: String,
    },
    /// How far the listing of what a zone shows has come.
    ListingChanged {
        zone: ZoneId,
        listing: Listing,
    },
    /// What is typed into a zone's filter; empty when it shows all again.
    FilterChanged {
        zone: ZoneId,
        typed: String,
    },
    SnapshotRenamed {
        name: String,
        to: String,
    },
    /// A whole layout, as a document to be written somewhere. (Boxed: it
    /// is far larger than anything else that is told.)
    LayoutExported {
        document: Box<LayoutDocument>,
    },
    /// A layout was taken in under `name`. `kept_as`: the snapshot that
    /// holds the layout there was before it was put in force.
    LayoutImported {
        name: String,
        applied: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kept_as: Option<String>,
    },
    /// Another zone or item has the keyboard, or none has.
    FocusChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
    },
    /// Other items are chosen.
    SelectionChanged {
        items: Vec<ItemId>,
    },
    /// What is covered from view changed: everything was covered by hand
    /// or uncovered, or the screen is shared or no longer.
    PrivacyChanged {
        masked: bool,
        screen_shared: bool,
    },
    /// The room tiled windows are to keep off is other than it was. For
    /// the window manager, by way of whoever runs the zones.
    ReservedChanged {
        regions: Vec<ReservedRegion>,
    },
    /// The command that was refused (the `Refused` before this) was
    /// refused by a lock of this zone: its own, or the one for all zones.
    RefusedByLock {
        zone: ZoneId,
        geometry: bool,
        members: bool,
    },
    /// What the command that was refused was about: the zone and the item
    /// it named (for a lock's refusal, the zone that is locked), with the
    /// reason again. So that a refusal which came by a key or from
    /// outside can be shown where it belongs. Told after the `Refused`
    /// of every command that names a zone or an item.
    RefusedAbout {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
        /// A lock refused it (`RefusedByLock` says which kind).
        #[serde(default)]
        locked: bool,
    },
    /// A file operation takes long: it is carried out by the backend and
    /// ends later. It is in `Scene.operations` until then.
    OperationBegun {
        request: u64,
        operation: FileOperation,
    },
    OperationProgress {
        request: u64,
        done: u64,
        total: u64,
    },
    /// An operation that took long has ended, well or not (an
    /// `OperationFailed` before this says if not).
    OperationEnded {
        request: u64,
    },
    /// Every icon is drawn grey now, or in its colours again.
    GreyIconsChanged {
        on: bool,
    },
    /// What making zones by type would do: the answer to a dry run.
    AutoPlanned {
        plan: AutoPlan,
    },
    /// Zones were made by type: the ones that are new, how many items
    /// were put into zones (new ones and those that were there), and how
    /// many rules were added. Nothing at all where there was nothing to
    /// do.
    AutoCreated {
        zones: Vec<ZoneId>,
        items: usize,
        rules: usize,
    },
    /// The last making of zones by type was taken back: how many zones
    /// went, how many items are where they were, how many rules went.
    AutoUndone {
        zones: usize,
        items: usize,
        rules: usize,
    },
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
    /// Each with its name, its kind, its place among them, and whether
    /// it is chosen or has the keyboard. None while the zone is covered.
    pub items: ItemLayout,
    /// Where the items are laid out, on the desktop: the zone without
    /// its title, its tabs and its padding.
    #[serde(default)]
    pub content: Rect,
    /// A rolled-up zone that is open for now. `rect` is the whole zone's
    /// then.
    #[serde(default)]
    pub expanded: bool,
    /// What opens it while it is rolled up.
    #[serde(default)]
    pub expand_on: ExpandOn,
    /// What may not be changed by dragging: by the zone's own lock or by
    /// the one for all zones.
    #[serde(default)]
    pub lock: Lock,
    #[serde(default)]
    pub always_visible: bool,
    /// What it holds is covered; `items` is empty.
    #[serde(default)]
    pub masked: bool,
    /// It is one of those that are covered until the person asks.
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub reserves_space: bool,
    /// It has the keyboard.
    #[serde(default)]
    pub focused: bool,
    /// How many items it holds, whatever the filter leaves of them and
    /// also while it is covered.
    #[serde(default)]
    pub item_count: usize,
    /// What is typed into its filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Which tab it is among how many, where it shares its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabPlace>,
    /// For a portal: where it stands, and how its listing does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal: Option<PortalScene>,
    /// For a zone that fills itself from something other than the
    /// desktop: how far the listing of that has come.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Listing>,
    /// It belongs to what is shown here: not hidden on these displays,
    /// and of the workspace that is shown. A zone that is `here` and not
    /// `shown` was put away with the one key.
    #[serde(default)]
    pub here: bool,
    /// The whole zone's rectangle, whether it is rolled up or not: where
    /// `rect` grows to when it opens.
    #[serde(default)]
    pub whole: Rect,
    /// A file operation that takes long goes into it or out of it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub busy: bool,
}

/// A file operation that takes long, while it has not ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationScene {
    /// What it is called off by (`Command::CancelOperation`).
    pub request: u64,
    pub operation: FileOperation,
    /// How far it has come, in whatever the backend counts; a `total` of
    /// 0 is "not known".
    pub done: u64,
    pub total: u64,
    /// The zones it goes into or out of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<ZoneId>,
    /// The item it is done to, where that is one that is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
}

/// A tab's place among its tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabPlace {
    /// Counted from 0, in the order the tabs are shown.
    pub index: usize,
    pub count: usize,
}

/// The loose items of one display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LooseScene {
    pub display: DisplayId,
    /// The room they are laid out in, on the desktop.
    pub area: Rect,
    /// None while they are covered.
    pub items: Vec<PlacedItem>,
    #[serde(default)]
    pub masked: bool,
    #[serde(default)]
    pub item_count: usize,
    /// The keyboard is on these items.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
}

/// Everything there is to draw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub zones: Vec<ZoneScene>,
    pub loose: Vec<LooseScene>,
    pub quick_hidden: bool,
    pub peeking: bool,
    /// Everything is covered by hand.
    #[serde(default)]
    pub masked: bool,
    /// The system says the screen is shown to others.
    #[serde(default)]
    pub screen_shared: bool,
    /// The lock for all zones.
    #[serde(default)]
    pub lock: Lock,
    /// What has the keyboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<Focus>,
    /// The chosen items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selection: Vec<ItemId>,
    /// The room tiled windows keep off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved: Vec<ReservedRegion>,
    /// The workspace that is shown, as far as it was told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The file operations that take long and have not ended.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<OperationScene>,
    /// Every icon is to be drawn grey, whatever the looks say of
    /// saturation: laid over them, not written into them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grey_icons: bool,
    /// The loose items are put away with the zones: the one key was
    /// pressed and nobody peeks. Of them only those with
    /// `always_visible` are drawn then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loose_hidden: bool,
}

/// What is so for the moment only. None of it is configuration and none
/// of it is written to the state file: after a restart no zone is open,
/// every portal stands in its own folder, nothing is typed anywhere and
/// nothing has the keyboard.
#[derive(Debug, Clone, Default)]
struct ViewState {
    /// The zone the pointer is over.
    pointer: Option<ZoneId>,
    /// The rolled-up zones that are open for now.
    expanded: BTreeSet<ZoneId>,
    /// The zone that opens when the pointer has rested on it until then.
    opening: Option<(ZoneId, u64)>,
    /// The open zones the pointer has left, and when each closes.
    closing: BTreeMap<ZoneId, u64>,
    /// What is typed into each zone's filter.
    filters: BTreeMap<ZoneId, String>,
    /// Where each portal stands that is not in its own folder.
    portals: BTreeMap<ZoneId, PortalView>,
    focus: Option<Focus>,
    selection: BTreeSet<ItemId>,
    /// Everything is covered by hand.
    masked: bool,
    /// The covered zones the person asked to see.
    revealed: BTreeSet<ZoneId>,
    /// The loose items likewise.
    loose_revealed: bool,
    screen_shared: bool,
    /// The lock that refused the command being carried out.
    refused_by: Option<(ZoneId, bool)>,
    /// The file operations that the backend carries out by itself, with
    /// how far each has come.
    long: BTreeMap<u64, (u64, u64)>,
}

/// What `settle` compares with afterwards, to tell of what changed by
/// the way.
struct Before {
    focus: Option<Focus>,
    selection: BTreeSet<ItemId>,
}

/// What a file operation that was asked for belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// A rule moves this item's file.
    RuleMove {
        item: ItemId,
        operation: FileOperation,
    },
    /// Something was dropped on a portal; it can be taken back.
    Portal { operation: FileOperation },
    /// A file is brought to the desktop; where it goes once it is there.
    Arriving {
        operation: FileOperation,
        zone: Option<ZoneId>,
        at: Option<LoosePosition>,
    },
    /// Something is being taken back.
    TakingBack { operation: FileOperation },
}

impl Pending {
    fn operation(&self) -> &FileOperation {
        match self {
            Pending::RuleMove { operation, .. }
            | Pending::Arriving { operation, .. }
            | Pending::Portal { operation }
            | Pending::TakingBack { operation } => operation,
        }
    }
}

/// Where an operation puts its file, if it goes exactly as asked.
fn lands_at(operation: &FileOperation) -> Option<String> {
    match operation {
        FileOperation::Copy { from, to_folder } | FileOperation::Move { from, to_folder } => {
            Some(paths::join(to_folder, paths::file_name(from)))
        }
        FileOperation::Rename { path, new_name } => {
            Some(paths::join(paths::parent(path)?, new_name))
        }
        FileOperation::NewFolder { parent, name } => Some(paths::join(parent, name)),
        FileOperation::Restore { path } => Some(path.clone()),
        FileOperation::Trash { .. } => None,
    }
}

/// How many paths of files the zones moved themselves are remembered.
const RETURNING_KEPT: usize = 64;

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
    peeking: bool,
    metrics: Metrics,
    view: ViewState,
    /// How far the listing of each watched folder has come.
    listings: BTreeMap<String, Listing>,
    /// What the sources of smart zones have, in each source's own order.
    sources: BTreeMap<SmartSource, Vec<Item>>,
    source_listings: BTreeMap<SmartSource, Listing>,
    /// The sources the backend was asked to watch.
    watched_sources: BTreeSet<SmartSource>,
    /// The room for tiled windows to keep off, as it was last told.
    told_reserved: Vec<ReservedRegion>,
    requests: u64,
    pending: BTreeMap<u64, Pending>,
    file_undo: Vec<FileOperation>,
    /// Files that were brought to the desktop and are not told of yet,
    /// by their paths: where each goes when it is.
    arriving: BTreeMap<String, (Option<ZoneId>, Option<LoosePosition>)>,
    /// Where the files lie that the zones themselves moved a moment ago
    /// (something taken back, something dropped on a portal), by their
    /// paths: one of them that shows up on the desktop did not come by
    /// itself. Only the last few.
    returning: Vec<String>,
    /// The system's own desktop icons are put away (on Windows,
    /// Explorer's): what the zones do not draw is not on the desktop at
    /// all. Told by whoever shows the zones; not so until it is.
    system_icons_away: bool,
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
            peeking: false,
            metrics: Metrics::default(),
            view: ViewState::default(),
            listings: BTreeMap::new(),
            sources: BTreeMap::new(),
            source_listings: BTreeMap::new(),
            watched_sources: BTreeSet::new(),
            told_reserved: Vec::new(),
            requests: 0,
            pending: BTreeMap::new(),
            file_undo: Vec::new(),
            arriving: BTreeMap::new(),
            returning: Vec::new(),
            system_icons_away: false,
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

    /// An item of the desktop, of a folder a zone shows, or of what a
    /// smart zone looks through.
    pub fn item(&self, id: &ItemId) -> Option<&Item> {
        self.items
            .get(id)
            .or_else(|| self.directories.values().find_map(|items| items.get(id)))
            .or_else(|| {
                self.sources
                    .values()
                    .find_map(|items| items.iter().find(|item| &item.id == id))
            })
    }

    /// The workspace that is shown, as far as it was told.
    pub fn workspace(&self) -> Option<&str> {
        self.workspace.as_deref()
    }

    /// The arrangement in use.
    pub fn profile(&self) -> Option<&Profile> {
        self.current
            .and_then(|index| self.state.profiles.get(index))
    }

    fn profile_mut(&mut self) -> Option<&mut Profile> {
        self.current
            .and_then(|index| self.state.profiles.get_mut(index))
    }

    /// What to ask of the backend at the start: the folders of the
    /// portals. (The desktop itself and the displays are the host's to
    /// list first; see `host::start`.)
    pub fn start(&mut self) -> Vec<Effect> {
        self.sync_watches()
    }

    /// Whether everything was hidden at once (`Command::QuickHide`) and
    /// is still: only zones that are always visible are shown then.
    pub fn quick_hidden(&self) -> bool {
        self.state.quick_hidden
    }

    /// Whether what is hidden at once is looked at for a moment all the
    /// same (`Command::Peek`).
    pub fn peeking(&self) -> bool {
        self.peeking
    }

    /// Whether the items that are in no zone are put away now: the one
    /// key was pressed, and nobody peeks. (They go with the zones.)
    pub fn loose_hidden(&self) -> bool {
        self.state.quick_hidden && !self.peeking
    }

    /// Whether the configuration has the zones switched on.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// The system's own desktop icons are put away, or no longer. Says
    /// whether something is drawn that was not, or the other way round.
    pub fn set_system_icons_away(&mut self, away: bool) -> bool {
        let changed = self.system_icons_away != away;
        self.system_icons_away = away;
        changed && !self.state.withheld.is_empty()
    }

    /// Whether the system's own desktop icons are put away, as it was told.
    pub fn system_icons_away(&self) -> bool {
        self.system_icons_away
    }

    /// Whether an item of the desktop is not drawn because nobody asked
    /// for it there: it came by itself while no zone could take it and
    /// the system's own desktop was put away (`ZonesState::withheld`).
    /// While the system shows its own icons, it is drawn like the rest.
    pub fn is_withheld(&self, item: &ItemId) -> bool {
        self.system_icons_away && self.state.withheld.contains(item) && self.zone_of(item).is_none()
    }

    /// Another configuration is in force (the file was changed).
    pub fn set_config(&mut self, config: ZonesConfig) -> Vec<Effect> {
        let before = self.before();
        let mut effects = self.take_config(config);
        // (No time is given here; what asks the time is asked again with
        // the next event or command, which is at once.)
        self.settle(&mut effects, before, None);
        effects
    }

    /// Put a configuration in force and say what follows from that.
    fn take_config(&mut self, config: ZonesConfig) -> Vec<Effect> {
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
        let display = self
            .displays
            .iter()
            .find(|display| display.id == geometry.display)?;
        let rect = snap::keep_reachable(geometry.on(display), work_area(display, &self.reserved));
        Some((rect, display))
    }

    /// Where a zone is drawn on the desktop: its whole rectangle, or its
    /// title when it is rolled up. Always within the free room of a
    /// display that is there.
    pub fn zone_rect(&self, zone: &ZoneId) -> Option<Rect> {
        let (rect, _) = self.place_of(zone)?;
        Some(if self.is_rolled(zone) {
            layout::rolled_up(rect, &self.metrics)
        } else {
            rect
        })
    }

    /// Whether a zone is shown now: not hidden on these displays, on the
    /// workspace that is shown, and not put away with the one key (unless
    /// the zones are peeked at).
    pub fn is_shown(&self, zone: &ZoneId) -> bool {
        self.is_about(zone)
            && (self.peeking
                || !self.state.quick_hidden
                || self
                    .config
                    .zone(zone)
                    .is_some_and(|zone| zone.always_visible))
    }

    /// Whether a zone belongs to what is shown here at all: not hidden on
    /// these displays, and of the workspace that is shown. A workspace
    /// that was never told hides nothing.
    fn is_about(&self, zone: &ZoneId) -> bool {
        let Some(definition) = self.config.zone(zone).filter(|_| self.config.enabled) else {
            return false;
        };
        let hidden_here = self
            .profile()
            .is_some_and(|profile| profile.hidden.contains(zone));
        let on_this_workspace = definition.workspaces.is_empty()
            || self
                .workspace
                .as_ref()
                .is_none_or(|shown| definition.workspaces.contains(shown));
        !hidden_here && on_this_workspace
    }

    /// What of a zone may not be changed by dragging: by its own lock or
    /// by the one for all zones.
    pub fn lock_of(&self, zone: &ZoneId) -> Lock {
        self.config
            .zone(zone)
            .map(|zone| zone.lock)
            .unwrap_or_default()
            .or(self.config.lock)
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

    /// The items a zone shows, in the order it shows them: what it
    /// holds, without what its filter leaves out.
    pub fn zone_items(&self, zone: &ZoneId, now: u64) -> Vec<&Item> {
        let mut items = self.zone_holds(zone, now);
        if let Some(typed) = self.view.filters.get(zone) {
            items.retain(|item| search::score(&item.name, typed).is_some());
        }
        items
    }

    /// Everything a zone holds, in its order, whatever is typed into its
    /// filter.
    pub fn zone_holds(&self, zone: &ZoneId, now: u64) -> Vec<&Item> {
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
            ZoneKind::Portal { .. } => self
                .directory_of(definition)
                .and_then(|directory| self.directories.get(&directory))
                .map(|items| items.values().collect())
                .unwrap_or_default(),
            ZoneKind::Smart { query } => self.smart_items(definition, query, now),
        };
        let mut items = layout::sorted(items, definition.sort, definition.sort_descending);
        if matches!(definition.kind, ZoneKind::Smart { .. }) && definition.limit > 0 {
            items.truncate(definition.limit as usize);
        }
        items
    }

    /// The zone filled by hand that an item is shown in. An item placed
    /// in a zone that is not there is loose.
    pub fn zone_of(&self, item: &ItemId) -> Option<&ZoneId> {
        self.state.placements.zone_of(item).filter(|zone| {
            self.config
                .zone(zone)
                .is_some_and(|zone| zone.kind == ZoneKind::Manual)
        })
    }

    /// The items of the desktop that are in no zone.
    pub fn loose_items(&self) -> Vec<&Item> {
        self.items
            .values()
            .filter(|item| self.zone_of(&item.id).is_none())
            .collect()
    }

    /// Say of each placed item what a renderer and a screen reader need:
    /// its name and kind, which one it is among how many, and whether it
    /// is chosen, has the keyboard, or stays when the rest is put away.
    fn describe(&self, placed: &mut [PlacedItem], focused: Option<&ItemId>) {
        let of = placed.len() as u32;
        for (index, placed) in placed.iter_mut().enumerate() {
            if let Some(item) = self.item(&placed.item) {
                placed.name = item.name.clone();
                placed.kind = item.kind;
            }
            placed.position = index as u32 + 1;
            placed.of = of;
            placed.selected = self.view.selection.contains(&placed.item);
            placed.focused = focused == Some(&placed.item);
            placed.always_visible = self.state.always_visible_items.contains(&placed.item);
        }
    }

    /// One zone as it is to be drawn; nothing for a zone without a place.
    fn zone_scene(
        &self,
        definition: &ZoneDefinition,
        groups: &[layout::TabGroup],
        now: u64,
    ) -> Option<ZoneScene> {
        let (whole, display) = self.place_of(&definition.id)?;
        let group = layout::group_of(groups, &definition.id);
        let content = layout::content_area(whole, group.is_some(), &self.metrics);
        let item_count = self.zone_holds(&definition.id, now).len();
        let masked = self.is_masked(&definition.id);
        // What is covered is not in the scene at all: what does not leave
        // the core cannot be drawn, read out or listed by mistake.
        let items: Vec<ItemId> = if masked {
            Vec::new()
        } else {
            self.zone_items(&definition.id, now)
                .into_iter()
                .map(|item| item.id.clone())
                .collect()
        };
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
        let mut laid_out = layout::lay_out(
            definition.view,
            definition.icon_size,
            &self.metrics,
            (content.width, content.height),
            &items,
            cells,
        );
        let focus = self
            .view
            .focus
            .as_ref()
            .filter(|focus| focus.zone == definition.id);
        self.describe(
            &mut laid_out.items,
            focus.and_then(|focus| focus.item.as_ref()),
        );
        let rolled = self.is_rolled(&definition.id);
        Some(ZoneScene {
            zone: definition.id.clone(),
            title: definition.title.clone(),
            display: display.id.clone(),
            rect: if rolled {
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
            items: laid_out,
            content,
            expanded: self.is_expanded(&definition.id),
            expand_on: self.expand_on(definition),
            lock: definition.lock.or(self.config.lock),
            always_visible: definition.always_visible,
            masked,
            private: definition.private || self.config.privacy.everything,
            reserves_space: definition.reserves_space,
            focused: focus.is_some(),
            item_count,
            filter: self.view.filters.get(&definition.id).cloned(),
            tab: group.and_then(|group| {
                let index = group
                    .members
                    .iter()
                    .position(|member| member == &definition.id)?;
                Some(TabPlace {
                    index,
                    count: group.members.len(),
                })
            }),
            portal: self.portal_scene(definition),
            source: self.source_listing(definition),
            here: self.is_about(&definition.id),
            whole,
            busy: false,
        })
    }

    /// Everything there is to draw, as it is at `now`.
    pub fn scene(&self, now: u64) -> Scene {
        let groups = layout::tab_groups(&self.config.zones);
        let mut scene = Scene {
            quick_hidden: self.state.quick_hidden,
            peeking: self.peeking,
            masked: self.view.masked,
            screen_shared: self.view.screen_shared,
            lock: self.config.lock,
            grey_icons: self.state.grey_icons,
            loose_hidden: self.loose_hidden(),
            ..Scene::default()
        };
        if !self.config.enabled {
            return scene;
        }
        scene.focus = self.view.focus.clone();
        scene.selection = self.view.selection.iter().cloned().collect();
        scene.reserved = self.reserved_regions();
        scene.workspace = self.workspace.clone();
        scene.zones = self
            .config
            .zones
            .iter()
            .filter_map(|definition| self.zone_scene(definition, &groups, now))
            .collect();
        scene.operations = self.operations();
        for zone in &mut scene.zones {
            zone.busy = scene
                .operations
                .iter()
                .any(|operation| operation.zones.contains(&zone.zone));
        }
        scene.loose = self.loose_scenes();
        scene
    }

    /// The items that are in no zone, laid out on each display that has
    /// some.
    pub(crate) fn loose_scenes(&self) -> Vec<LooseScene> {
        let mut loose = Vec::new();
        let Some(fallback) = fallback_display(&self.displays).map(|display| display.id.clone())
        else {
            return loose;
        };
        let mut by_display: BTreeMap<DisplayId, Vec<&Item>> = BTreeMap::new();
        for item in self.loose_items() {
            if self.is_withheld(&item.id) {
                continue;
            }
            let own = self
                .state
                .placements
                .loose
                .get(&item.id)
                .map(|position| &position.display);
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
            let masked = self.loose_masked();
            let mut placed = if masked {
                Vec::new()
            } else {
                layout::lay_out_loose(
                    self.config.loose_icon_size,
                    &self.metrics,
                    (area.width, area.height),
                    &ids,
                    &own,
                )
            };
            let focus = self
                .view
                .focus
                .as_ref()
                .filter(|focus| focus.desk.as_ref() == Some(&display.id));
            self.describe(&mut placed, focus.and_then(|focus| focus.item.as_ref()));
            loose.push(LooseScene {
                display: display.id.clone(),
                area,
                items: placed,
                masked,
                item_count: ids.len(),
                focused: focus.is_some(),
            });
        }
        loose
    }

    /// The items whose names fit what was typed, wherever they are
    /// shown: on the desktop (loose or in a zone), in the folder each
    /// portal stands in, and in what the zones that fill themselves show.
    /// The best fit first, each with the zone it is in. What a covered
    /// zone holds is not found. A filter typed into a zone hides nothing
    /// here.
    pub fn search(&self, typed: &str, now: u64) -> Vec<Hit> {
        let covered = |zone: Option<&ZoneId>| match zone {
            Some(zone) => self.is_masked(zone),
            None => self.loose_masked(),
        };
        let mut found: Vec<(&Item, Option<&ZoneId>)> = self
            .items
            .values()
            .map(|item| (item, self.zone_of(&item.id)))
            .filter(|(_, zone)| !covered(*zone))
            .collect();
        let mut smart: BTreeMap<&ItemId, Vec<ZoneId>> = BTreeMap::new();
        for definition in &self.config.zones {
            if definition.kind == ZoneKind::Manual || self.is_masked(&definition.id) {
                continue;
            }
            for item in self.zone_holds(&definition.id, now) {
                if matches!(definition.kind, ZoneKind::Smart { .. }) {
                    smart
                        .entry(&item.id)
                        .or_default()
                        .push(definition.id.clone());
                }
                // Once is enough: an item of the desktop is found there,
                // and what two zones show is found in the first.
                if !found.iter().any(|(known, _)| known.id == item.id) {
                    found.push((item, Some(&definition.id)));
                }
            }
        }
        let mut hits = search::search(typed, found);
        for hit in &mut hits {
            let mut also = smart.remove(&hit.item).unwrap_or_default();
            also.retain(|zone| Some(zone) != hit.zone.as_ref());
            hit.also_in = also;
        }
        hits
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
            if self
                .config
                .zone(front)
                .is_none_or(|zone| zone.tab_group.as_ref() != Some(group))
            {
                return Some(format!(
                    "{front} is in front of {group} and is no tab of it"
                ));
            }
        }
        if let Some(focus) = self
            .view
            .focus
            .as_ref()
            .filter(|focus| focus.desk.is_none())
        {
            if self.config.zone(&focus.zone).is_none() {
                return Some(format!("{} has the keyboard and is no zone", focus.zone));
            }
        }
        if let Some(stray) = self
            .view
            .selection
            .iter()
            .find(|item| self.item(item).is_none())
        {
            return Some(format!("{stray} is chosen and is not there"));
        }
        if let Some(open) = self
            .view
            .expanded
            .iter()
            .find(|zone| self.config.zone(zone).is_none_or(|zone| !zone.rolled_up))
        {
            return Some(format!("{open} is open for now and is not rolled up"));
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
            let Some(display) = self
                .displays
                .iter()
                .find(|display| display.id == geometry.display)
            else {
                return Some(format!(
                    "{} is on {}, which is not there",
                    zone.id, geometry.display
                ));
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
                return Some(format!(
                    "{} at {rect:?} cannot be reached on {}",
                    zone.id, on.id
                ));
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
    fn reconcile_from(
        &mut self,
        before: Option<&BTreeMap<ZoneId, ZoneGeometry>>,
    ) -> (bool, Vec<ZoneId>) {
        let mut changed = false;
        let valid: Vec<(String, ZoneId)> = self
            .state
            .active_tabs
            .iter()
            .filter(|(group, zone)| {
                self.config
                    .zone(zone)
                    .is_some_and(|zone| zone.tab_group.as_ref() == Some(*group))
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
            return if self.reconcile() {
                vec![Effect::SaveState]
            } else {
                Vec::new()
            };
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
            Effect::Notify(ZonesEvent::ProfileChanged {
                made: chosen.made.is_some(),
                moved,
            }),
            Effect::SaveState,
        ]
    }

    /// Ask for what the zones show to be watched: the folder each portal
    /// stands in, and what the zones that fill themselves look through.
    /// What no zone shows any longer is let go.
    fn sync_watches(&mut self) -> Vec<Effect> {
        // (Switched off, nothing is watched.)
        let zones = self.config.zones.iter().filter(|_| self.config.enabled);
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        let mut wanted_sources: BTreeSet<SmartSource> = BTreeSet::new();
        for zone in zones {
            match (&zone.kind, &zone.source) {
                (ZoneKind::Portal { .. }, _) => wanted.extend(self.directory_of(zone)),
                (ZoneKind::Smart { .. }, SmartSource::Folder { directory }) => {
                    wanted.insert(directory.clone());
                }
                (ZoneKind::Smart { .. }, SmartSource::Desktop) | (ZoneKind::Manual, _) => {}
                (ZoneKind::Smart { .. }, source) => {
                    wanted_sources.insert(source.clone());
                }
            }
        }
        let mut effects = Vec::new();
        for directory in wanted.difference(&self.watched) {
            self.listings.insert(directory.clone(), Listing::Loading);
            effects.push(Effect::Ask(BackendRequest::Watch {
                directory: directory.clone(),
            }));
        }
        for directory in self.watched.difference(&wanted) {
            effects.push(Effect::Ask(BackendRequest::Unwatch {
                directory: directory.clone(),
            }));
            self.directories.remove(directory);
            self.listings.remove(directory);
        }
        self.watched = wanted;
        for source in wanted_sources.difference(&self.watched_sources) {
            self.source_listings
                .insert(source.clone(), Listing::Loading);
            effects.push(Effect::Ask(BackendRequest::WatchSource {
                source: source.clone(),
            }));
        }
        for source in self.watched_sources.difference(&wanted_sources) {
            effects.push(Effect::Ask(BackendRequest::UnwatchSource {
                source: source.clone(),
            }));
            self.sources.remove(source);
            self.source_listings.remove(source);
        }
        self.watched_sources = wanted_sources;
        effects
    }

    fn ask(&mut self, pending: Pending) -> Effect {
        self.requests += 1;
        let operation = pending.operation().clone();
        log::debug!(
            "request {}: {operation} is asked of the backend",
            self.requests
        );
        self.pending.insert(self.requests, pending);
        Effect::Ask(BackendRequest::Perform {
            request: self.requests,
            operation,
        })
    }

    // ---- what happened outside ----------------------------------------

    /// Something happened outside. `now` is the time in seconds since
    /// 1970 (rules that ask about age are tried against it).
    pub fn event(&mut self, event: BackendEvent, now: u64) -> Vec<Effect> {
        let before = self.before();
        told(&event);
        let mut effects = self.happened(event, now);
        self.settle(&mut effects, before, Some(now));
        say(&effects);
        effects
    }

    fn happened(&mut self, event: BackendEvent, now: u64) -> Vec<Effect> {
        match event {
            BackendEvent::Listed {
                location: Location::Desktop,
                items,
            } => {
                self.items = items
                    .into_iter()
                    .map(|item| (item.id.clone(), item))
                    .collect();
                let items = &self.items;
                self.state.withheld.retain(|item| items.contains_key(item));
                vec![Effect::Notify(ZonesEvent::LayoutChanged)]
            }
            BackendEvent::Listed {
                location: Location::Directory { path },
                items,
            } => {
                // (An answer that comes after the folder was let go.)
                if !self.watched.contains(&path) {
                    return Vec::new();
                }
                self.directories.insert(
                    path.clone(),
                    items
                        .into_iter()
                        .map(|item| (item.id.clone(), item))
                        .collect(),
                );
                let mut effects = self.listing_is(&path, Listing::Ready);
                effects.extend(self.portals_changed(&path));
                effects
            }
            BackendEvent::ListingFailed {
                location: Location::Directory { path },
                error,
            } => {
                if !self.watched.contains(&path) {
                    return Vec::new();
                }
                log::debug!("{path} could not be listed: {error}");
                // What was shown of it is not known to be there any more.
                self.directories.remove(&path);
                let mut effects = self.listing_is(
                    &path,
                    Listing::Unavailable {
                        reason: error.to_string(),
                    },
                );
                effects.extend(self.portals_changed(&path));
                effects
            }
            // The desktop that cannot be listed keeps what was last known
            // of it: there is nothing better to show.
            BackendEvent::ListingFailed {
                location: Location::Desktop,
                error,
            } => {
                log::debug!(
                    "the desktop could not be listed; what was known of it is kept: {error}"
                );
                Vec::new()
            }
            BackendEvent::SourceListed { source, items } => self.source_listed(source, items),
            BackendEvent::SourceFailed { source, error } => self.source_failed(source, error),
            BackendEvent::ScreenShared { shared } => self.screen_shared(shared),
            BackendEvent::OperationBegun { request } => match self.pending.get(&request) {
                Some(pending) => {
                    self.view.long.insert(request, (0, 0));
                    let operation = pending.operation().clone();
                    vec![Effect::Notify(ZonesEvent::OperationBegun {
                        request,
                        operation,
                    })]
                }
                None => Vec::new(),
            },
            BackendEvent::OperationProgress {
                request,
                done,
                total,
            } => {
                // (Word of one that has ended, or was never asked for.)
                if !self.pending.contains_key(&request) {
                    return Vec::new();
                }
                self.view.long.insert(request, (done, total));
                vec![Effect::Notify(ZonesEvent::OperationProgress {
                    request,
                    done,
                    total,
                })]
            }
            BackendEvent::ItemAdded {
                location: Location::Desktop,
                item,
            } => self.arrived(item, now),
            BackendEvent::ItemRemoved {
                location: Location::Desktop,
                item,
            } => {
                let zone = self.zone_of(&item).cloned();
                if self.items.remove(&item).is_none() {
                    return Vec::new();
                }
                self.state.placements.take(&item);
                self.state.always_visible_items.remove(&item);
                self.state.withheld.remove(&item);
                vec![
                    Effect::Notify(ZonesEvent::ItemsChanged { zone }),
                    Effect::SaveState,
                ]
            }
            BackendEvent::ItemRenamed {
                location: Location::Desktop,
                from,
                item,
            } => {
                self.items.remove(&from);
                self.state.placements.rename(&from, &item.id);
                if self.state.always_visible_items.remove(&from) {
                    self.state.always_visible_items.insert(item.id.clone());
                }
                if self.state.withheld.remove(&from) {
                    self.state.withheld.insert(item.id.clone());
                }
                let zone = self.zone_of(&item.id).cloned();
                self.items.insert(item.id.clone(), item);
                vec![
                    Effect::Notify(ZonesEvent::ItemsChanged { zone }),
                    Effect::SaveState,
                ]
            }
            BackendEvent::ItemChanged {
                location: Location::Desktop,
                item,
            } => {
                let zone = self.zone_of(&item.id).cloned();
                self.items.insert(item.id.clone(), item);
                vec![Effect::Notify(ZonesEvent::ItemsChanged { zone })]
            }
            BackendEvent::ItemAdded {
                location: Location::Directory { path },
                item,
            }
            | BackendEvent::ItemChanged {
                location: Location::Directory { path },
                item,
            } => {
                if !self.follows(&path) {
                    return Vec::new();
                }
                self.directories
                    .entry(path.clone())
                    .or_default()
                    .insert(item.id.clone(), item);
                self.portals_changed(&path)
            }
            BackendEvent::ItemRemoved {
                location: Location::Directory { path },
                item,
            } => {
                if !self.follows(&path) {
                    return Vec::new();
                }
                if let Some(items) = self.directories.get_mut(&path) {
                    items.remove(&item);
                }
                self.state.always_visible_items.remove(&item);
                self.portals_changed(&path)
            }
            BackendEvent::ItemRenamed {
                location: Location::Directory { path },
                from,
                item,
            } => {
                if !self.follows(&path) {
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
                    if !self.displays.is_empty() {
                        log::debug!("no display is left for the zones; they stay as they are until one is back");
                    }
                    return Vec::new();
                }
                let ids = |displays: &[Display]| {
                    displays
                        .iter()
                        .map(|display| display.id.to_string())
                        .collect::<Vec<_>>()
                };
                if ids(&self.displays) != ids(&displays) {
                    log::debug!(
                        "the zones are on {} now: {}",
                        crate::timed::count(displays.len(), "display", "displays"),
                        ids(&displays).join(", ")
                    );
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
                log::debug!(
                    "the zones are on the workspace {} now",
                    name.as_deref().unwrap_or("(none)")
                );
                self.workspace = name;
                if !self.config.profile_per_workspace {
                    // Zones that are of some workspaces only come and go
                    // with them, also where all share one arrangement.
                    return if self
                        .config
                        .zones
                        .iter()
                        .any(|zone| !zone.workspaces.is_empty())
                    {
                        vec![Effect::Notify(ZonesEvent::LayoutChanged)]
                    } else {
                        Vec::new()
                    };
                }
                let mut effects = self.choose_profile();
                effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
                effects
            }
            BackendEvent::OperationDone {
                request,
                undo,
                error,
            } => {
                let long = self.view.long.remove(&request).is_some();
                let mut effects = self.operation_done(request, undo, error);
                if long {
                    effects.push(Effect::Notify(ZonesEvent::OperationEnded { request }));
                }
                effects
            }
            BackendEvent::IconReady { .. } => Vec::new(),
        }
    }

    /// Whether word of single items of a folder is taken: the folder is
    /// watched and was listed whole. Until it was (it is still being
    /// listed, or could not be), a single item says nothing of the rest,
    /// and the listing that comes will have it.
    fn follows(&self, directory: &str) -> bool {
        self.watched.contains(directory)
            && self.listings.get(directory).is_some_and(Listing::is_ready)
    }

    /// The zones that show a folder have other items.
    fn portals_changed(&self, directory: &str) -> Vec<Effect> {
        self.showing(directory)
            .into_iter()
            .map(|zone| Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone) }))
            .collect()
    }

    /// The zone an item goes to that came to the desktop by itself and
    /// that no rule decides about: the one made for its type, or else the
    /// first of the person's own that is filled by hand and takes items,
    /// of those shown here before the others. (A zone made for another
    /// type is not for it.) None where no zone takes it.
    fn zone_for_newcomer(&self, item: &Item) -> Option<ZoneId> {
        if self.config.lock.members {
            return None;
        }
        let takes = |zone: &&ZoneDefinition| zone.kind == ZoneKind::Manual && !zone.lock.members;
        let open = || self.config.zones.iter().filter(takes);
        let category = crate::auto::category_of(&self.config.auto.categories, item)
            .map(|category| &category.id);
        let own = || open().filter(|zone| zone.auto.is_none());
        open()
            .find(|zone| category.is_some() && zone.auto.as_ref() == category)
            .or_else(|| own().find(|zone| self.is_about(&zone.id)))
            .or_else(|| own().next())
            .map(|zone| zone.id.clone())
    }

    /// An item came to the desktop. Where the configuration says so, the
    /// zone its rule names takes it at once. One that no rule decides
    /// about (an icon a program's installation leaves, a file saved
    /// there) goes into a zone that takes it; where there is none it
    /// lies loose, and where the system's own desktop is put away as
    /// well it is not drawn at all (`ZonesState::withheld`). What the
    /// person let go on the desktop is where they let it go.
    fn arrived(&mut self, item: Item, now: u64) -> Vec<Effect> {
        let id = item.id.clone();
        let mut effects = Vec::new();
        // What the zones moved here themselves (something taken back) did
        // not come by itself: it lies where it lay.
        let ours = item.path.as_ref().is_some_and(|path| {
            let returned = self.returning.iter().position(|known| known == path);
            returned.map(|at| self.returning.remove(at)).is_some()
                || self
                    .pending
                    .values()
                    .any(|pending| lands_at(pending.operation()).as_ref() == Some(path))
        });
        // (Told of twice, or known from before with a place of its own:
        // it is where the person has it.)
        let new = !ours
            && !self.items.contains_key(&id)
            && !self.state.placements.loose.contains_key(&id);
        // Brought here by the person: where they let it go, before any rule.
        let brought = item
            .path
            .as_ref()
            .and_then(|path| self.arriving.remove(path));
        if let Some(place) = brought {
            self.items.insert(id.clone(), item);
            self.settle_arrival(&id, place);
            let zone = self.zone_of(&id).cloned();
            return vec![
                Effect::Notify(ZonesEvent::ItemsChanged { zone }),
                Effect::SaveState,
            ];
        }
        if self.config.enabled && self.zone_of(&id).is_none() {
            let decided = self
                .config
                .rules_on_arrival
                .then(|| {
                    rules::decide(
                        &self.config.rules,
                        self.config.catch_all.as_ref(),
                        &item,
                        now,
                    )
                })
                .flatten()
                .map(|(_, action)| action.clone());
            // (What is no file, as This PC or the bin, is on the desktop
            // because the person switched it on.)
            let by_itself = new && item.kind != ItemKind::Virtual;
            match decided {
                Some(RuleAction::AssignToZone { zone }) => {
                    let takes =
                        self.config.zone(&zone).is_some_and(|zone| {
                            zone.kind == ZoneKind::Manual && !zone.lock.members
                        }) && !self.config.lock.members;
                    if takes {
                        self.state.placements.assign(&id, &zone, None, None);
                        effects.push(Effect::SaveState);
                    }
                }
                // (A rule that moves the file itself waits to be applied.)
                Some(RuleAction::MoveToFolder { .. }) => {}
                None if by_itself => match self.zone_for_newcomer(&item) {
                    Some(zone) => {
                        self.state.placements.assign(&id, &zone, None, None);
                        effects.push(Effect::SaveState);
                    }
                    None if self.system_icons_away => {
                        self.state.withheld.insert(id.clone());
                        effects.push(Effect::SaveState);
                    }
                    None => {}
                },
                None => {}
            }
        }
        self.items.insert(id.clone(), item);
        let zone = self.zone_of(&id).cloned();
        effects.insert(0, Effect::Notify(ZonesEvent::ItemsChanged { zone }));
        effects
    }

    /// Put an item that was brought to the desktop where it was let go:
    /// into the zone, if that still takes it, or at its place.
    fn settle_arrival(
        &mut self,
        item: &ItemId,
        (zone, at): (Option<ZoneId>, Option<LoosePosition>),
    ) {
        let takes = !self.config.lock.members
            && zone.as_ref().is_some_and(|zone| {
                self.config
                    .zone(zone)
                    .is_some_and(|zone| zone.kind == ZoneKind::Manual && !zone.lock.members)
            });
        match (zone, at) {
            (Some(zone), _) if takes => self.state.placements.assign(item, &zone, None, None),
            (_, Some(at)) => self.state.placements.loosen(item, Some(at)),
            _ => {}
        }
    }

    fn operation_done(
        &mut self,
        request: u64,
        undo: Option<FileOperation>,
        error: Option<BackendError>,
    ) -> Vec<Effect> {
        let Some(pending) = self.pending.remove(&request) else {
            log::debug!("request {request} ended, and was not waited for any more");
            return Vec::new();
        };
        match &error {
            None => log::debug!("request {request} is done: {}", pending.operation()),
            Some(BackendError::Cancelled) => {
                log::debug!("request {request} was called off: {}", pending.operation())
            }
            Some(error) => {
                log::debug!("request {request} failed: {}: {error}", pending.operation())
            }
        }
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
                let moved =
                    self.state.undo.as_mut().and_then(|record| {
                        record.moved.iter_mut().find(|moved| moved.item == item)
                    });
                match moved {
                    Some(moved) if moved.undo != undo => {
                        moved.undo = undo;
                        vec![Effect::SaveState]
                    }
                    _ => Vec::new(),
                }
            }
            (
                Pending::Portal { operation }
                | Pending::TakingBack { operation }
                | Pending::Arriving { operation, .. },
                Some(error),
            ) => {
                vec![Effect::Notify(ZonesEvent::OperationFailed {
                    operation,
                    error,
                })]
            }
            (
                Pending::Arriving {
                    operation,
                    zone,
                    at,
                },
                None,
            ) => {
                let undo = undo.or_else(|| operation.reversed());
                // Where the file is now is what takes it back says.
                let path = match &undo {
                    Some(FileOperation::Trash { path }) => Some(path.clone()),
                    Some(FileOperation::Move { from, .. }) => Some(from.clone()),
                    _ => None,
                };
                if let Some(undo) = undo {
                    self.file_undo.push(undo);
                    if self.file_undo.len() > FILE_UNDO_DEPTH {
                        self.file_undo.remove(0);
                    }
                }
                let Some(path) = path else {
                    return Vec::new();
                };
                // Told of already, or still to come.
                let there = self
                    .items
                    .values()
                    .find(|item| item.path.as_ref() == Some(&path))
                    .map(|item| item.id.clone());
                match there {
                    Some(item) => {
                        self.settle_arrival(&item, (zone, at));
                        let zone = self.zone_of(&item).cloned();
                        vec![
                            Effect::Notify(ZonesEvent::ItemsChanged { zone }),
                            Effect::SaveState,
                        ]
                    }
                    None => {
                        self.arriving.insert(path, (zone, at));
                        Vec::new()
                    }
                }
            }
            (Pending::Portal { operation }, None) => {
                self.moved_by_us(&operation, undo.as_ref());
                if let Some(undo) = undo.or_else(|| operation.reversed()) {
                    self.file_undo.push(undo);
                    if self.file_undo.len() > FILE_UNDO_DEPTH {
                        self.file_undo.remove(0);
                    }
                }
                Vec::new()
            }
            (Pending::TakingBack { operation }, None) => {
                self.moved_by_us(&operation, undo.as_ref());
                Vec::new()
            }
        }
    }

    /// The zones moved a file themselves: remember where it lies now
    /// (what takes the operation back says it best), for when it is told
    /// of as an item that came.
    fn moved_by_us(&mut self, operation: &FileOperation, undo: Option<&FileOperation>) {
        let path = match undo {
            Some(FileOperation::Trash { path }) => Some(path.clone()),
            Some(FileOperation::Move { from, .. }) => Some(from.clone()),
            _ => lands_at(operation),
        };
        if let Some(path) = path {
            self.returning.push(path);
            if self.returning.len() > RETURNING_KEPT {
                self.returning.remove(0);
            }
        }
    }

    // ---- what the person asks for -------------------------------------

    /// Carry out a command. One that cannot be carried out changes
    /// nothing and gives `ZonesEvent::Refused` with the reason.
    pub fn command(&mut self, command: Command, now: u64) -> Vec<Effect> {
        let level = if busy(&command) {
            log::Level::Trace
        } else {
            log::Level::Debug
        };
        let name = if log::log_enabled!(level) {
            name_of(&command)
        } else {
            String::new()
        };
        let refused = |reason: String| vec![Effect::Notify(ZonesEvent::Refused { reason })];
        if !self.config.enabled {
            log::log!(level, "{name} was refused: the zones are switched off");
            return refused("zones are switched off (desktop.zones.enabled)".to_string());
        }
        let command = match self.aimed(command) {
            Ok(command) => command,
            Err(reason) => {
                log::log!(level, "{name} was refused: {reason}");
                return refused(reason);
            }
        };
        let before = self.before();
        self.view.refused_by = None;
        let (mut zone, item) = Self::about(&command);
        if log::log_enabled!(level) {
            let zone = zone
                .as_ref()
                .map(|zone| format!(" for the zone {zone}"))
                .unwrap_or_default();
            let item = item
                .as_ref()
                .map(|item| format!(" about {item}"))
                .unwrap_or_default();
            log::log!(level, "the zones were asked to {name}{zone}{item}");
        }
        let mut effects = self.carry_out(command, now).unwrap_or_else(refused);
        // A refusal says what it was about, for the zone to show it; one
        // that was a lock's says whose lock.
        let reason = effects.iter().find_map(|effect| match effect {
            Effect::Notify(ZonesEvent::Refused { reason }) => Some(reason.clone()),
            _ => None,
        });
        let by_lock = self.view.refused_by.take();
        if let Some(reason) = reason {
            log::log!(level, "{name} was refused: {reason}");
            let locked = by_lock.is_some();
            if let Some((locked_zone, geometry)) = by_lock {
                zone = Some(locked_zone.clone());
                let event = ZonesEvent::RefusedByLock {
                    zone: locked_zone,
                    geometry,
                    members: !geometry,
                };
                effects.push(Effect::Notify(event));
            }
            if zone.is_some() || item.is_some() {
                effects.push(Effect::Notify(ZonesEvent::RefusedAbout {
                    reason,
                    zone,
                    item,
                    locked,
                }));
            }
        }
        self.settle(&mut effects, before, Some(now));
        say(&effects);
        effects
    }

    /// A command that names `@focused` for a zone, with the zone that has
    /// the keyboard in its place.
    fn aimed(&self, command: Command) -> Result<Command, String> {
        // (A whole layout is large, and names no zone to act on.)
        if matches!(command, Command::ImportLayout { .. }) {
            return Ok(command);
        }
        let Ok(mut written) = serde_json::to_value(&command) else {
            return Ok(command);
        };
        let Some(fields) = written.as_object_mut() else {
            return Ok(command);
        };
        let mut aimed = false;
        for key in ["zone", "with"] {
            if fields.get(key).and_then(|zone| zone.as_str()) == Some(FOCUSED) {
                let focused = self.view.focus.as_ref().ok_or("no zone has the keyboard")?;
                if focused.desk.is_some() {
                    return Err("the desktop has the keyboard, not a zone".to_string());
                }
                fields.insert(
                    key.to_string(),
                    serde_json::Value::from(focused.zone.as_str()),
                );
                aimed = true;
            }
        }
        if !aimed {
            return Ok(command);
        }
        serde_json::from_value(written).map_err(|error| error.to_string())
    }

    /// The zone and the item a command names, where it names one.
    fn about(command: &Command) -> (Option<ZoneId>, Option<ItemId>) {
        // (A whole layout is large, and is about no one zone.)
        if matches!(command, Command::ImportLayout { .. }) {
            return (None, None);
        }
        let written = serde_json::to_value(command).unwrap_or_default();
        let named = |key: &str| {
            written
                .get(key)
                .and_then(|name| name.as_str())
                .map(str::to_string)
        };
        let first = written
            .get("items")
            .and_then(|items| items.get(0))
            .and_then(|item| item.as_str());
        (
            named("zone").map(ZoneId::new),
            named("item").or(first.map(str::to_string)).map(ItemId::new),
        )
    }

    fn before(&self) -> Before {
        Before {
            focus: self.view.focus.clone(),
            selection: self.view.selection.clone(),
        }
    }

    /// After anything happened: bring what is so for the moment in line
    /// with what there is (a zone that is gone is not open, nothing that
    /// is gone has the keyboard or is chosen), and tell of what changed
    /// by the way: the keyboard is elsewhere, other items are chosen,
    /// other room is to be kept free of tiled windows.
    fn settle(&mut self, effects: &mut Vec<Effect>, before: Before, now: Option<u64>) {
        let rolled =
            |core: &Self, zone: &ZoneId| core.config.zone(zone).is_some_and(|zone| zone.rolled_up);
        let open: Vec<ZoneId> = self
            .view
            .expanded
            .iter()
            .filter(|zone| !rolled(self, zone))
            .cloned()
            .collect();
        for zone in open {
            self.view.expanded.remove(&zone);
            self.view.closing.remove(&zone);
        }
        if self
            .view
            .opening
            .as_ref()
            .is_some_and(|(zone, _)| !rolled(self, zone))
        {
            self.view.opening = None;
        }
        let known = |core: &Self, zone: &ZoneId| core.config.zone(zone).is_some();
        if self
            .view
            .pointer
            .as_ref()
            .is_some_and(|zone| !known(self, zone))
        {
            self.view.pointer = None;
        }
        let gone: Vec<ZoneId> = self
            .view
            .filters
            .keys()
            .chain(self.view.revealed.iter())
            .chain(self.view.portals.keys())
            .filter(|zone| !known(self, zone))
            .cloned()
            .collect();
        for zone in gone {
            self.view.filters.remove(&zone);
            self.view.revealed.remove(&zone);
            self.view.portals.remove(&zone);
        }
        // A portal that shows another folder now begins there.
        let moved: Vec<ZoneId> = self
            .view
            .portals
            .iter()
            .filter(|(zone, view)| {
                !matches!(self.config.zone(zone).map(|zone| &zone.kind),
                    Some(ZoneKind::Portal { directory }) if directory == view.root())
            })
            .map(|(zone, _)| zone.clone())
            .collect();
        for zone in moved {
            self.view.portals.remove(&zone);
        }
        // The keyboard: on a zone that is there and shown, and on an item
        // the zone shows. (Without the time nothing that depends on it is
        // looked at: the item stays until the next call, which has it.)
        if let Some(focus) = self.view.focus.clone() {
            if let Some(display) = &focus.desk {
                // The bare desktop: while something lies loose on that
                // display and is to be seen, and on an item that does.
                match self.desks().into_iter().find(|(on, _, _)| on == display) {
                    None => self.view.focus = None,
                    Some((_, _, items)) => {
                        if focus
                            .item
                            .as_ref()
                            .is_some_and(|item| !items.iter().any(|placed| &placed.item == item))
                        {
                            self.view.focus = Some(Focus::on_desk(display, None));
                        }
                    }
                }
            } else if !self.is_shown(&focus.zone)
                || (self.is_masked(&focus.zone) && focus.item.is_some())
            {
                self.view.focus = self
                    .is_shown(&focus.zone)
                    .then(|| Focus::on_zone(&focus.zone, None));
            } else if let (Some(item), Some(now)) = (&focus.item, now) {
                if !self
                    .zone_items(&focus.zone, now)
                    .iter()
                    .any(|shown| &shown.id == item)
                {
                    self.view.focus = Some(Focus::on_zone(&focus.zone, None));
                }
            }
        }
        // What was put somewhere is drawn from then on, wherever it goes
        // afterwards.
        let placements = &self.state.placements;
        self.state.withheld.retain(|item| {
            placements.zone_of(item).is_none() && !placements.loose.contains_key(item)
        });
        let stray: Vec<ItemId> = self
            .view
            .selection
            .iter()
            .filter(|item| self.item(item).is_none())
            .cloned()
            .collect();
        for item in stray {
            self.view.selection.remove(&item);
        }
        // A zone that was open because the keyboard was in it closes when
        // the keyboard leaves, unless the pointer holds it open.
        let was_in = before.focus.as_ref().map(|focus| &focus.zone);
        let is_in = self.view.focus.as_ref().map(|focus| focus.zone.clone());
        if let Some(left) = was_in.filter(|zone| Some(*zone) != is_in.as_ref()) {
            let held = self
                .view
                .pointer
                .as_ref()
                .is_some_and(|over| self.companions(over).contains(left));
            if !held && self.view.expanded.contains(left) {
                effects.extend(self.set_expanded(&left.clone(), false));
            }
        }
        if self.view.focus != before.focus {
            let focus = self.view.focus.clone();
            effects.push(Effect::Notify(ZonesEvent::FocusChanged {
                zone: focus.as_ref().map(|focus| focus.zone.clone()),
                item: focus.and_then(|focus| focus.item),
            }));
        }
        if self.view.selection != before.selection {
            effects.push(Effect::Notify(ZonesEvent::SelectionChanged {
                items: self.view.selection.iter().cloned().collect(),
            }));
        }
        let regions = self.reserved_regions();
        if regions != self.told_reserved {
            self.told_reserved = regions.clone();
            effects.push(Effect::Notify(ZonesEvent::ReservedChanged { regions }));
        }
    }

    fn carry_out(&mut self, command: Command, now: u64) -> Outcome {
        match command {
            Command::CreateZone { title, kind, rect } => self.create_zone(title, kind, rect),
            Command::RenameZone { zone, title } => {
                self.change_zone(&zone, |zone| zone.title = title)
            }
            Command::DeleteZone { zone } => self.delete_zone(&zone),
            Command::MoveZone { zone, x, y, snap } => self.move_zone(&zone, x, y, snap),
            Command::ResizeZone {
                zone,
                rect,
                edges,
                snap,
            } => self.resize_zone(&zone, rect, edges, snap),
            Command::AddItem {
                zone,
                item,
                index,
                cell,
            } => self.add_item(&zone, &item, index, cell),
            Command::RemoveItem { item } => self.loosen_item(&item, None),
            Command::PlaceLooseItem {
                item,
                display,
                x,
                y,
            } => self.loosen_item(&item, Some(LoosePosition { display, x, y })),
            Command::RollUp { zone, rolled_up } => self.roll_up(&zone, rolled_up),
            Command::Lock {
                zone,
                geometry,
                members,
            } => self.change_zone(&zone, |zone| {
                zone.lock.geometry = geometry.unwrap_or(zone.lock.geometry);
                zone.lock.members = members.unwrap_or(zone.lock.members);
            }),
            Command::LockAll { geometry, members } => {
                let before = self.config.lock;
                self.config.lock = match (geometry, members) {
                    (None, None) => {
                        let on = !(before.geometry && before.members);
                        Lock {
                            geometry: on,
                            members: on,
                        }
                    }
                    _ => Lock {
                        geometry: geometry.unwrap_or(before.geometry),
                        members: members.unwrap_or(before.members),
                    },
                };
                if self.config.lock == before {
                    Ok(Vec::new())
                } else {
                    Ok(vec![
                        Effect::Notify(ZonesEvent::SettingsChanged),
                        Effect::Notify(ZonesEvent::LayoutChanged),
                        Effect::SaveConfig,
                    ])
                }
            }
            Command::ModifyZone { zone, changes } => self.modify_zone(&zone, &changes),
            Command::ModifySettings { changes } => self.modify_settings(&changes),
            Command::PointerOver {
                zone,
                at_ms,
                dragging,
            } => Ok(self.pointer_over(zone, at_ms, dragging)),
            Command::Expand { zone, expanded } => self.expand(&zone, expanded),
            Command::Tick { at_ms } => Ok(self.tick(at_ms)),
            Command::MoveTab { zone, index } => self.move_tab(&zone, index),
            Command::SplitTab { zone, x, y, snap } => {
                // (Refused before anything is taken apart.)
                self.known(&zone)?
                    .tab_group
                    .as_ref()
                    .ok_or_else(|| format!("{zone} is no tab"))?;
                self.movable(&zone)?;
                let mut effects = self.untab(&zone)?;
                effects.extend(self.move_zone(&zone, x, y, snap)?);
                Ok(effects)
            }
            Command::NextTab { zone, backwards } => self.next_tab(&zone, backwards),
            Command::AlwaysVisible { zone, on } => {
                let on = on.unwrap_or(!self.known(&zone)?.always_visible);
                self.change_zone(&zone, |zone| zone.always_visible = on)
            }
            Command::ItemAlwaysVisible { item, on } => self.item_always_visible(&item, on),
            Command::TogglePeek => {
                self.peeking = !self.peeking;
                Ok(vec![self.visibility()])
            }
            Command::Dismiss => Ok(self.dismiss()),
            Command::Open {
                items,
                with,
                in_place,
            } => self.open(items, with, in_place),
            Command::EnterFolder { zone, item } => self.navigate(&zone, folders::Way::Into(item)),
            Command::NavigateTo { zone, directory } => {
                self.navigate(&zone, folders::Way::To(directory))
            }
            Command::NavigateUp { zone } => self.navigate(&zone, folders::Way::Up),
            Command::NavigateBack { zone } => self.navigate(&zone, folders::Way::Back),
            Command::NavigateForward { zone } => self.navigate(&zone, folders::Way::Forward),
            Command::RefreshZone { zone } => self.refresh_zone(&zone),
            Command::NewFolder { zone, name } => self.new_folder(&zone, &name),
            Command::NewFolderIn { folder, name } => self.new_folder_in(folder, &name),
            Command::MoveItemsToFolder {
                items,
                folder,
                copy,
            } => self.move_items_to_folder(&items, &folder, copy),
            Command::Filter { zone, typed } => self.filter(zone, &typed),
            Command::CheckTimedRules => self.check_timed_rules(now),
            Command::RenameSnapshot { name, to } => self.rename_snapshot(&name, &to),
            Command::ExportLayout { name } => self.export_layout(name.as_deref(), now),
            Command::ImportLayout { document, apply } => self.import_layout(&document, apply, now),
            Command::CopyArrangement { from } => self.copy_arrangement(&from),
            Command::FocusZone { zone } => self.focus_zone(zone),
            Command::FocusNeighbour { direction } => self.focus_neighbour(direction),
            Command::FocusItem { item } => self.focus_item(item, now),
            Command::MoveItemFocus { step, extend } => self.move_item_focus(step, extend, now),
            Command::Select { items, mode } => Ok(self.select(&items, mode)),
            Command::OpenFocused => {
                let items = self.acted_on();
                if items.is_empty() {
                    Err("no item has the keyboard and none is chosen".to_string())
                } else {
                    self.open(items, None, true)
                }
            }
            Command::MoveFocusedItems { direction, zone } => {
                self.move_focused_items(direction, zone, now)
            }
            Command::CancelOperation { request } => {
                if self.pending.contains_key(&request) && self.view.long.contains_key(&request) {
                    Ok(vec![Effect::Ask(BackendRequest::Cancel { request })])
                } else {
                    Err("no such operation is under way".to_string())
                }
            }
            Command::Mask { masked } => Ok(self.mask(masked)),
            Command::GreyIcons { on } => self.set_grey_icons(on),
            Command::AutoCreate {
                scope,
                min_items,
                display,
                add_rules,
                dry_run,
            } => self.auto_create(scope, min_items, display.as_ref(), add_rules, dry_run, now),
            Command::UndoAutoCreate => self.undo_auto_create(),
            Command::Reveal { zone, revealed } => self.reveal(zone, revealed),
            Command::Tab { zone, with } => self.tab(&zone, &with),
            Command::Untab { zone } => self.untab(&zone),
            Command::ActivateTab { zone } => self.activate_tab(&zone),
            Command::SetView {
                zone,
                view,
                sort,
                descending,
                icon_size,
            } => self.change_zone(&zone, |zone| {
                zone.view = view.unwrap_or(zone.view);
                zone.sort = sort.unwrap_or(zone.sort);
                zone.sort_descending = descending.unwrap_or(zone.sort_descending);
                zone.icon_size = icon_size.unwrap_or(zone.icon_size).clamp(16, 256);
            }),
            Command::ShowZone { zone, shown } => self.show_zone(&zone, shown),
            Command::PreviewRules { scope } => {
                let plan = self.plan(scope, now);
                let lines = self.explain(&plan);
                Ok(vec![Effect::Notify(ZonesEvent::RulesPreviewed {
                    plan,
                    lines,
                })])
            }
            Command::ApplyRules { scope } => self.apply_rules(scope, now),
            Command::UndoRules => self.undo_rules(),
            Command::UndoFileOperation => match self.file_undo.pop() {
                Some(operation) => Ok(vec![self.ask(Pending::TakingBack { operation })]),
                None => Err("no file operation to take back".to_string()),
            },
            Command::TrashItems { items } => self.trash_items(&items),
            Command::RenameItem { item, name } => {
                let path = self.item(&item).and_then(|item| item.path.clone());
                let name = name.trim().to_string();
                match path {
                    None => Err(format!("{item} is no file that could be renamed")),
                    Some(_) if name.is_empty() || name.contains(['/', '\\']) => {
                        Err("that is no name for a file".to_string())
                    }
                    Some(path) if paths::file_name(&path) == name => Ok(Vec::new()),
                    Some(path) => {
                        let operation = FileOperation::Rename {
                            path,
                            new_name: name,
                        };
                        Ok(vec![self.ask(Pending::Portal { operation })])
                    }
                }
            }
            Command::BringIn {
                paths,
                folder,
                copy,
                zone,
                at,
            } => {
                if paths.is_empty() || folder.trim().is_empty() {
                    Err("nothing to bring in, or nowhere to".to_string())
                } else if zone
                    .as_ref()
                    .is_some_and(|zone| self.config.zone(zone).is_none())
                {
                    Err("no such zone".to_string())
                } else {
                    Ok(paths
                        .into_iter()
                        .enumerate()
                        .map(|(number, from)| {
                            let operation = if copy {
                                FileOperation::Copy {
                                    from,
                                    to_folder: folder.clone(),
                                }
                            } else {
                                FileOperation::Move {
                                    from,
                                    to_folder: folder.clone(),
                                }
                            };
                            // Several let go at one point lie a step apart.
                            let at = at.clone().map(|at| LoosePosition {
                                x: at.x + 24 * number as i32,
                                y: at.y + 24 * number as i32,
                                ..at
                            });
                            self.ask(Pending::Arriving {
                                operation,
                                zone: zone.clone(),
                                at,
                            })
                        })
                        .collect())
                }
            }
            Command::SaveSnapshot { name } => self.save_snapshot(name, now),
            Command::RestoreSnapshot { name } => self.restore_snapshot(&name, now),
            Command::DeleteSnapshot { name } => {
                let before = self.state.snapshots.len();
                self.state
                    .snapshots
                    .retain(|snapshot| snapshot.name != name);
                if self.state.snapshots.len() == before {
                    Err(format!("no snapshot is called {name}"))
                } else {
                    Ok(vec![
                        Effect::Notify(ZonesEvent::SnapshotDeleted { name }),
                        Effect::SaveState,
                    ])
                }
            }
            Command::QuickHide { hidden } => {
                let hidden = hidden.unwrap_or(!self.state.quick_hidden);
                if hidden == self.state.quick_hidden {
                    return Ok(vec![self.visibility()]);
                }
                self.state.quick_hidden = hidden;
                Ok(vec![self.visibility(), Effect::SaveState])
            }
            Command::Peek { on } => {
                self.peeking = on;
                Ok(vec![self.visibility()])
            }
            Command::Search { typed } => {
                let hits = self.search(&typed, now);
                Ok(vec![Effect::Notify(ZonesEvent::Found { typed, hits })])
            }
        }
    }

    fn visibility(&self) -> Effect {
        Effect::Notify(ZonesEvent::VisibilityChanged {
            quick_hidden: self.state.quick_hidden,
            peeking: self.peeking,
        })
    }

    fn known(&self, zone: &ZoneId) -> Result<&ZoneDefinition, String> {
        self.config
            .zone(zone)
            .ok_or_else(|| format!("no zone is called {zone}"))
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
        Ok(vec![
            Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }),
            Effect::SaveConfig,
        ])
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
                let sized = Rect::new(
                    rect.x,
                    rect.y,
                    rect.width.max(least_width),
                    rect.height.max(least_height),
                );
                let rect = snap::keep_reachable(sized, work_area(display, &self.reserved));
                ZoneGeometry::nearest(rect, display)
            }
            None => {
                let already = self.profile().map_or(0, |profile| profile.geometry.len());
                self.default_geometry(already)
                    .ok_or("no display is known yet")?
            }
        };
        self.config.zones.push(ZoneDefinition {
            id: id.clone(),
            title,
            kind,
            ..ZoneDefinition::default()
        });
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
        self.config
            .zones
            .retain(|definition| &definition.id != zone);
        self.state.placements.drop_zone(zone);
        for profile in &mut self.state.profiles {
            profile.geometry.remove(zone);
            profile.hidden.remove(zone);
        }
        self.dissolve_lone_tab(group);
        self.reconcile();
        let mut effects = vec![Effect::Notify(ZonesEvent::ZoneDeleted {
            zone: zone.clone(),
        })];
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
            .filter(|zone| {
                self.place_of(&zone.id)
                    .is_some_and(|(_, on)| &on.id == display)
            })
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
                Some(Effect::Notify(ZonesEvent::ZonePlaced {
                    zone: companion,
                    rect,
                }))
            })
            .collect();
        effects.push(Effect::SaveState);
        effects
    }

    fn movable(&mut self, zone: &ZoneId) -> Result<(Rect, Display), String> {
        self.known(zone)?;
        let companions = self.companions(zone);
        if companions.iter().any(|zone| self.lock_of(zone).geometry) {
            self.view.refused_by = Some((zone.clone(), true));
            return Err(format!("{zone} is locked where it is"));
        }
        let (rect, display) = self.place_of(zone).ok_or("no display is known yet")?;
        Ok((rect, display.clone()))
    }

    fn move_zone(&mut self, zone: &ZoneId, x: i32, y: i32, snapping: bool) -> Outcome {
        let (whole, _) = self.movable(zone)?;
        let wanted = Rect::new(x, y, whole.width, whole.height);
        let display = display_of(wanted, &self.displays)
            .ok_or("no display is known yet")?
            .clone();
        let area = work_area(&display, &self.reserved);
        let rest = if snapping {
            let others = self.others_on(&display.id, &self.companions(zone));
            let around = Surroundings {
                display: display.rect,
                work_area: area,
                others: &others,
            };
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
        let around = Surroundings {
            display: display.rect,
            work_area: area,
            others: &others,
        };
        let settings = crate::config::SnapSettings {
            enabled: snapping && self.config.snap.enabled,
            ..self.config.snap.clone()
        };
        let sized = snap::snap_resize(rect, edges, &around, &settings, self.metrics.least());
        Ok(self.place(zone, snap::keep_reachable(sized, area), &display))
    }

    fn roll_up(&mut self, zone: &ZoneId, rolled_up: Option<bool>) -> Outcome {
        let rolled_up = rolled_up.unwrap_or(!self.known(zone)?.rolled_up);
        log::debug!(
            "the zone {zone} is {}",
            if rolled_up { "rolled up" } else { "unrolled" }
        );
        let mut effects = Vec::new();
        // Tabs roll up together: they are one thing on the desktop.
        for companion in self.companions(zone) {
            effects.extend(self.change_zone(&companion, |zone| zone.rolled_up = rolled_up)?);
        }
        Ok(effects)
    }

    fn add_item(
        &mut self,
        zone: &ZoneId,
        item: &ItemId,
        index: Option<usize>,
        cell: Option<Cell>,
    ) -> Outcome {
        let target = self.known(zone)?.clone();
        if self.lock_of(zone).members {
            self.view.refused_by = Some((zone.clone(), false));
            return Err(format!("{zone} is locked: nothing goes in or out"));
        }
        let from = self.zone_of(item).cloned();
        if let Some(from) = from.as_ref().filter(|from| self.lock_of(from).members) {
            self.view.refused_by = Some((from.clone(), false));
            return Err(format!("{item} is in a locked zone"));
        }
        match &target.kind {
            ZoneKind::Manual => {
                if !self.items.contains_key(item) {
                    return Err(format!("{item} is not on the desktop"));
                }
                self.state.placements.assign(item, zone, index, cell);
                let mut effects = vec![Effect::Notify(ZonesEvent::ItemsChanged {
                    zone: Some(zone.clone()),
                })];
                if from.as_ref() != Some(zone) {
                    effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: from }));
                }
                effects.push(Effect::SaveState);
                Ok(effects)
            }
            ZoneKind::Portal { directory } => {
                // Into the folder the portal stands in, which is the one
                // the person sees.
                let directory = self
                    .directory_of(&target)
                    .unwrap_or_else(|| directory.clone());
                let path = self
                    .item(item)
                    .and_then(|item| item.path.clone())
                    .ok_or_else(|| format!("{item} is no file that could be moved"))?;
                if paths::parent(&path).is_some_and(|at| paths::same_folder(at, &directory)) {
                    return Err(format!("{item} is in that folder already"));
                }
                let operation = FileOperation::Move {
                    from: path,
                    to_folder: directory,
                };
                Ok(vec![self.ask(Pending::Portal { operation })])
            }
            ZoneKind::Smart { .. } => Err(format!("{zone} fills itself")),
        }
    }

    fn trash_items(&mut self, items: &[ItemId]) -> Outcome {
        let mut effects = Vec::new();
        for id in items {
            let locked = self
                .zone_of(id)
                .is_some_and(|zone| self.lock_of(zone).members);
            // What is no file (This PC) and what a locked zone holds stays.
            let Some(path) = self
                .item(id)
                .and_then(|item| item.path.clone())
                .filter(|_| !locked)
            else {
                continue;
            };
            effects.push(self.ask(Pending::Portal {
                operation: FileOperation::Trash { path },
            }));
        }
        if effects.is_empty() {
            return Err("nothing of that can be put into the bin".to_string());
        }
        Ok(effects)
    }

    fn loosen_item(&mut self, item: &ItemId, position: Option<LoosePosition>) -> Outcome {
        if !self.items.contains_key(item) {
            return Err(format!("{item} is not on the desktop"));
        }
        let from = self.zone_of(item).cloned();
        if let Some(from) = from.as_ref().filter(|from| self.lock_of(from).members) {
            self.view.refused_by = Some((from.clone(), false));
            return Err(format!("{item} is in a locked zone"));
        }
        if let Some(position) = &position {
            if !self
                .displays
                .iter()
                .any(|display| display.id == position.display)
            {
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
        let left = self
            .config
            .zone(zone)
            .and_then(|zone| zone.tab_group.clone());
        let group = match self
            .config
            .zone(with)
            .and_then(|with| with.tab_group.clone())
        {
            Some(group) => group,
            None => {
                let mut number = 1;
                loop {
                    let name = format!("tabs-{number}");
                    if !self
                        .config
                        .zones
                        .iter()
                        .any(|zone| zone.tab_group.as_ref() == Some(&name))
                    {
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
        let group = self
            .known(zone)?
            .tab_group
            .clone()
            .ok_or_else(|| format!("{zone} is no tab"))?;
        let place = self
            .place_of(zone)
            .map(|(rect, display)| (rect, display.clone()));
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
        let group = self
            .known(zone)?
            .tab_group
            .clone()
            .ok_or_else(|| format!("{zone} is no tab"))?;
        if self.state.active_tabs.insert(group, zone.clone()).as_ref() == Some(zone) {
            return Ok(Vec::new());
        }
        Ok(vec![
            Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }),
            Effect::SaveState,
        ])
    }

    fn show_zone(&mut self, zone: &ZoneId, shown: bool) -> Outcome {
        self.known(zone)?;
        let profile = self.profile_mut().ok_or("no display is known yet")?;
        let changed = if shown {
            profile.hidden.remove(zone)
        } else {
            profile.hidden.insert(zone.clone())
        };
        if !changed {
            return Ok(Vec::new());
        }
        Ok(vec![
            Effect::Notify(ZonesEvent::ZoneChanged { zone: zone.clone() }),
            Effect::SaveState,
        ])
    }

    /// What the rules would do now.
    pub fn plan(&self, scope: Scope, now: u64) -> Plan {
        rules::plan(
            &self.config,
            self.items.values(),
            &self.state.placements,
            scope,
            now,
        )
    }

    /// A plan in the words a person is shown, as the items are now.
    pub fn explain(&self, plan: &Plan) -> Vec<PlanLine> {
        rules::explain(
            plan,
            &self.config,
            &|id| self.items.get(id),
            &self.state.placements,
        )
    }

    /// How often the rules that ask the time want to be tried
    /// (`Command::CheckTimedRules`), in seconds. None while there is no
    /// such rule, or the configuration says never.
    pub fn timed_rules_every(&self) -> Option<u64> {
        let minutes = self.config.timed_rules.every_minutes;
        (self.config.enabled && minutes > 0 && self.config.rules.iter().any(rules::is_timed))
            .then(|| u64::from(minutes) * 60)
    }

    fn check_timed_rules(&mut self, now: u64) -> Outcome {
        let plan = rules::plan_timed(
            &self.config,
            self.items.values(),
            &self.state.placements,
            now,
            self.config.timed_rules.move_files,
        );
        // Nothing has come of age: nothing is said, and what could be
        // taken back before still can be.
        if plan.steps.is_empty() {
            return Ok(Vec::new());
        }
        self.apply_plan(plan, now)
    }

    fn apply_rules(&mut self, scope: Scope, now: u64) -> Outcome {
        let plan = self.plan(scope, now);
        self.apply_plan(plan, now)
    }

    fn apply_plan(&mut self, plan: Plan, now: u64) -> Outcome {
        let lines = self.explain(&plan);
        if plan.steps.is_empty() {
            // Nothing to do, and nothing to take back that was not there before.
            return Ok(vec![Effect::Notify(ZonesEvent::RulesApplied {
                plan,
                lines,
            })]);
        }
        let items = &self.items;
        let path_of = |id: &ItemId| items.get(id).and_then(|item| item.path.clone());
        let applied = rules::apply(&plan, &path_of, &mut self.state.placements, now);
        let mut effects = vec![Effect::Notify(ZonesEvent::RulesApplied { plan, lines })];
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
        let record = self
            .state
            .undo
            .take()
            .ok_or("the rules were not applied, or it was taken back already")?;
        let operations = rules::undo(&record, &mut self.state.placements);
        let mut effects = vec![Effect::Notify(ZonesEvent::RulesUndone)];
        for operation in operations {
            effects.push(self.ask(Pending::TakingBack { operation }));
        }
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        effects.push(Effect::SaveState);
        Ok(effects)
    }
}

// ---- what is said in the log ----------------------------------------

/// What a command is called, for the log ("create_zone"). Only its name:
/// what was typed (a search, a filter) is never said.
fn name_of(command: &Command) -> String {
    // (A whole layout is large, and its name is known.)
    if matches!(command, Command::ImportLayout { .. }) {
        return "import_layout".to_string();
    }
    serde_json::to_value(command)
        .ok()
        .and_then(|written| {
            written
                .get("command")
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "do something".to_string())
}

/// A command that comes at each move of the pointer, each key or each
/// tick: said at trace only.
fn busy(command: &Command) -> bool {
    matches!(
        command,
        Command::MoveZone { .. }
            | Command::ResizeZone { .. }
            | Command::PointerOver { .. }
            | Command::Tick { .. }
            | Command::Search { .. }
            | Command::Filter { .. }
            | Command::Peek { .. }
            | Command::Select { .. }
            | Command::FocusZone { .. }
            | Command::FocusItem { .. }
            | Command::FocusNeighbour { .. }
            | Command::MoveItemFocus { .. }
            | Command::CheckTimedRules
    )
}

/// What the backend told, at trace: the listings and changes of folders
/// come often. (The failures are said where they are taken in.)
fn told(event: &BackendEvent) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    match event {
        BackendEvent::Listed { location, items } => {
            log::trace!("{location} was listed: {} items", items.len())
        }
        BackendEvent::ItemAdded { location, item } => log::trace!("{} came to {location}", item.id),
        BackendEvent::ItemRemoved { location, item } => log::trace!("{item} went from {location}"),
        BackendEvent::ItemRenamed {
            location,
            from,
            item,
        } => log::trace!("{from} in {location} is {} now", item.id),
        BackendEvent::ItemChanged { location, item } => {
            log::trace!("{} in {location} changed", item.id)
        }
        BackendEvent::OperationProgress {
            request,
            done,
            total,
        } => log::trace!("request {request} has come to {done} of {total}"),
        BackendEvent::IconReady { request, icon } => {
            log::trace!("the picture of request {} is {icon:?}", request.0)
        }
        _ => {}
    }
}

/// What came of a command or an event, as far as it is a state of the
/// zones that changed (a zone made, a folder gone to, an operation
/// ended), at debug; what changes at each move, at trace.
fn say(effects: &[Effect]) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    for effect in effects {
        let event = match effect {
            Effect::Notify(event) => event,
            Effect::SaveConfig => {
                log::trace!("the zones' configuration is to be written");
                continue;
            }
            Effect::SaveState => {
                log::trace!("the zones' state is to be written");
                continue;
            }
            Effect::Ask(_) => continue,
        };
        match event {
            ZonesEvent::ZoneCreated { zone } => log::debug!("the zone {zone} was made"),
            ZonesEvent::ZoneDeleted { zone } => log::debug!("the zone {zone} was taken away"),
            ZonesEvent::ZoneExpanded {
                zone,
                expanded: true,
            } => log::debug!("the zone {zone} is open"),
            ZonesEvent::ZoneExpanded {
                zone,
                expanded: false,
            } => log::debug!("the zone {zone} is rolled up again"),
            ZonesEvent::ProfileChanged { made, moved } => log::debug!(
                "another arrangement of the zones is in use{}{}",
                if *made {
                    ", made new for these displays"
                } else {
                    ""
                },
                if moved.is_empty() {
                    String::new()
                } else {
                    format!(", {} moved to another display", moved.len())
                }
            ),
            ZonesEvent::SettingsChanged => log::debug!("the zones' settings changed"),
            ZonesEvent::Navigated { zone, directory } => {
                log::debug!("the portal {zone} shows {directory} now")
            }
            ZonesEvent::ListingChanged { zone, listing } => {
                log::debug!("the listing of the zone {zone} is {listing:?}")
            }
            ZonesEvent::OperationBegun { request, operation } => {
                log::debug!("request {request} ({operation}) goes on by itself")
            }
            ZonesEvent::OperationEnded { request } => log::debug!("request {request} has ended"),
            ZonesEvent::OpenRequested { items, with } => log::debug!(
                "{} to be opened{}",
                crate::timed::count(items.len(), "item is", "items are"),
                with.as_ref()
                    .map(|with| format!(" with {with}"))
                    .unwrap_or_default()
            ),
            ZonesEvent::SnapshotSaved { name } => log::debug!("the snapshot {name} was saved"),
            ZonesEvent::SnapshotRestored { name } => {
                log::debug!("the snapshot {name} was put in force")
            }
            ZonesEvent::SnapshotDeleted { name } => {
                log::debug!("the snapshot {name} was taken away")
            }
            ZonesEvent::SnapshotRenamed { name, to } => {
                log::debug!("the snapshot {name} is called {to} now")
            }
            ZonesEvent::LayoutImported { name, applied, .. } => {
                log::debug!(
                    "a layout was taken in as {name}{}",
                    if *applied { " and put in force" } else { "" }
                )
            }
            ZonesEvent::LayoutExported { .. } => log::debug!("the zones' layout was written out"),
            ZonesEvent::RulesApplied { plan, .. } => log::debug!(
                "the rules were applied: {} steps, {} held back",
                plan.steps.len(),
                plan.held.len()
            ),
            ZonesEvent::RulesUndone => log::debug!("what the rules did was taken back"),
            ZonesEvent::AutoCreated {
                zones,
                items,
                rules,
            } => {
                log::debug!(
                    "{} were made by type, holding {items} items, with {rules} rules",
                    crate::timed::count(zones.len(), "zone", "zones")
                )
            }
            ZonesEvent::AutoUndone {
                zones,
                items,
                rules,
            } => {
                log::debug!("the zones made by type were taken back: {zones} zones, {items} items, {rules} rules")
            }
            ZonesEvent::VisibilityChanged {
                quick_hidden,
                peeking,
            } => {
                log::debug!(
                    "the zones are {}{}",
                    if *quick_hidden { "hidden" } else { "shown" },
                    if *peeking {
                        ", looked at above the windows"
                    } else {
                        ""
                    }
                )
            }
            ZonesEvent::PrivacyChanged {
                masked,
                screen_shared,
            } => {
                log::debug!(
                    "what the zones show is {} (the screen is {}shared)",
                    if *masked { "covered" } else { "uncovered" },
                    if *screen_shared { "" } else { "not " }
                )
            }
            ZonesEvent::GreyIconsChanged { on } => log::debug!(
                "the icons are drawn {}",
                if *on { "grey" } else { "in their colours" }
            ),
            ZonesEvent::RefusedByLock { zone, .. } => log::debug!("the lock of {zone} refused it"),
            _ => log::trace!("the zones tell: {}", told_name(event)),
        }
    }
}

/// What a told event is called, for the log at trace.
fn told_name(event: &ZonesEvent) -> &'static str {
    match event {
        ZonesEvent::ZoneChanged { .. } => "a zone changed",
        ZonesEvent::ZonePlaced { .. } => "a zone was placed",
        ZonesEvent::ItemsChanged { .. } => "the items of a zone changed",
        ZonesEvent::LayoutChanged => "the layout changed",
        ZonesEvent::FocusChanged { .. } => "the keyboard is elsewhere",
        ZonesEvent::SelectionChanged { .. } => "other items are chosen",
        ZonesEvent::ReservedChanged { .. } => "other room is kept",
        ZonesEvent::Found { .. } => "what was found",
        ZonesEvent::FilterChanged { .. } => "a filter changed",
        ZonesEvent::OperationProgress { .. } => "how far an operation has come",
        ZonesEvent::Refused { .. } | ZonesEvent::RefusedAbout { .. } => "a refusal",
        ZonesEvent::RulesPreviewed { .. } => "what the rules would do",
        ZonesEvent::AutoPlanned { .. } => "what making zones by type would do",
        _ => "something else",
    }
}
