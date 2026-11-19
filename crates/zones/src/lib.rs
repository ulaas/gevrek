//! gevrek-zones: containers on the desktop that hold its icons. This is
//! the half that is the same on every system: no system API, no drawing,
//! no async runtime, no threads, and no file touched but the state file a
//! caller names. See docs/zones/design.md.
//!
//! - `config`: what the person sets (zones, looks, rules), made to be a
//!   section of the configuration.
//! - `item`, `ids`: what lies on the desktop and what things are known by.
//! - `placement`, `geometry`, `profile`, `state`: which item is where,
//!   where a zone is on a display, one arrangement for each set of
//!   displays, and all of it together as what is remembered.
//! - `store`: the state on the disk, with backups, never losing a layout.
//! - `snap`, `layout`: where a dragged zone comes to rest, and where
//!   items are inside one.
//! - `rules`: sorting items by rules: a dry run, applying, taking back.
//! - `search`: finding an item by some of its name.
//! - `backend`: what is asked of a system (`ZonesBackend`), and what
//!   passes between it and the core.
//! - `core`: the state machine. Events and commands in, effects out.
//! - `host`: carries events and requests between a core and a backend.
//! - `ipc`: what is asked from outside, what is answered, and the zones
//!   as they are told to other programs.
//! - `testing`: a backend in memory.

pub mod backend;
pub mod config;
pub mod core;
pub mod geometry;
pub mod host;
pub mod ids;
pub mod ipc;
pub mod item;
pub mod layout;
pub mod paths;
pub mod placement;
pub mod profile;
pub mod rules;
pub mod search;
pub mod snap;
pub mod state;
pub mod store;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod core_scenarios;
#[cfg(test)]
mod random_runs;

pub use crate::core::{Command, Effect, Scene, ZonesCore, ZonesEvent};
pub use backend::{BackendEvent, BackendRequest, ZonesBackend};
pub use config::ZonesConfig;
pub use gevrek_model::geometry::Rect;
pub use ids::{DisplayId, ItemId, ZoneId};
pub use item::Item;
pub use state::ZonesState;
pub use store::Store;
