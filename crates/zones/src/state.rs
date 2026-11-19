//! What the zones remember that is not configuration: which item is
//! where, where each zone is on each set of displays, the saved copies,
//! and how to take back the last time the rules were applied. It changes
//! with every drag, so it lives in a file of its own (`store`), not in
//! `config.json`.

use crate::config::{Rule, RuleAction, ZoneDefinition};
use crate::ids::ZoneId;
use crate::placement::Placements;
use crate::profile::Profile;
use crate::rules::UndoRecord;
use gevrek_model::Extra;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}
