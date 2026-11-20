//! What the zones remember that is not configuration: which item is
//! where, where each zone is on each set of displays, the saved copies,
//! and how to take back the last time the rules were applied. It changes
//! with every drag, so it lives in a file of its own (`store`), not in
//! `config.json`.

use crate::auto::AutoUndo;
use crate::config::{Rule, RuleAction, ZoneDefinition};
use crate::ids::{ItemId, ZoneId};
use crate::placement::Placements;
use crate::profile::Profile;
use crate::rules::UndoRecord;
use gevrek_model::Extra;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The layout of the state file. A file with a lower number is brought up
/// step by step (`store::migrate`); one with a higher number is from a
/// newer gevrek and is left alone.
pub const STATE_VERSION: u32 = 1;

/// Everything that is remembered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZonesState {
    pub version: u32,
    pub placements: Placements,
    /// One arrangement for each set of displays that was seen.
    pub profiles: Vec<Profile>,
    /// Which tab of a group is in front, by the group's name.
    pub active_tabs: BTreeMap<String, ZoneId>,
    pub snapshots: Vec<Snapshot>,
    /// How to take back the last application of the rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<UndoRecord>,
    /// Counts up with every zone made, for ids that are never used twice.
    pub zones_made: u64,
    /// Counts up with every choice of an arrangement.
    pub choices: u64,
    /// The zones and loose items are put away with the one key. Kept, so
    /// that a desktop the person cleared is still clear after a restart.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub quick_hidden: bool,
    /// The items that stay when the rest is put away with the one key,
    /// whichever zone they are in.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub always_visible_items: BTreeSet<ItemId>,
    /// Every icon is drawn grey. A way of looking at the desktop that the
    /// person switched on, kept like the putting away: it is no setting
    /// of any zone's look, and changes none.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grey_icons: bool,
    /// How to take back the last making of zones by type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_undo: Option<AutoUndo>,
    /// The icons that came to the desktop by themselves (a program's
    /// installation put them there) while no zone could take them and the
    /// system's own desktop was put away: nobody asked for them on a
    /// desktop that was cleared, so they are not drawn. Each is an item of
    /// the desktop like any other (listed, found, sorted by the rules),
    /// and is drawn from the moment it is put into a zone or at a place.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub withheld: BTreeSet<ItemId>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for ZonesState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            placements: Placements::default(),
            profiles: Vec::new(),
            active_tabs: BTreeMap::new(),
            snapshots: Vec::new(),
            undo: None,
            zones_made: 0,
            choices: 0,
            quick_hidden: false,
            always_visible_items: BTreeSet::new(),
            grey_icons: false,
            auto_undo: None,
            withheld: BTreeSet::new(),
            extra: Extra::new(),
        }
    }
}

/// A named copy of everything that makes up a layout: the zones and rules
/// of the configuration as they were, and the placements and
/// arrangements.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub name: String,
    /// When it was taken, in seconds since 1970.
    pub taken_at: u64,
    pub zones: Vec<ZoneDefinition>,
    pub rules: Vec<Rule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catch_all: Option<RuleAction>,
    pub placements: Placements,
    pub profiles: Vec<Profile>,
    pub active_tabs: BTreeMap<String, ZoneId>,
    /// Taken by the zones themselves, before something replaced the whole
    /// layout (a snapshot restored, a layout brought in), so that what
    /// was there is never lost. Only the last few of these are kept.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Snapshot {
    /// Whether two copies hold the same layout, whatever they are called
    /// and whenever they were taken.
    pub fn same_layout(&self, other: &Snapshot) -> bool {
        // (When an arrangement was last used is no part of where things
        // are: looking at a layout does not make it another.)
        let arranged = |snapshot: &Snapshot| -> Vec<Profile> {
            snapshot.profiles.iter().map(|profile| Profile { last_used: 0, ..profile.clone() }).collect()
        };
        self.zones == other.zones
            && self.rules == other.rules
            && self.catch_all == other.catch_all
            && self.placements == other.placements
            && arranged(self) == arranged(other)
            && self.active_tabs == other.active_tabs
    }
}

/// How many copies the zones keep that they took themselves.
pub const AUTOMATIC_KEPT: usize = 5;
