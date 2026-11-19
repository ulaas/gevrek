//! gevrek-engine: backend-neutral window-management logic. Depends only on
//! `gevrek-model`: no Windows API, no UI, no async runtime, no threads.
//! See docs/PHASE2_PLAN.md. `contract` is the backend interface (`Desktop`,
//! `WindowQueries`, `Effect`): a backend answers the queries, carries out
//! the effects and feeds the core `events::WindowEvent`s. `core::WmCore`
//! owns the WM's state and `executor` holds the logic a host drives.
//! `testing::fake_desktop` is a desktop without a windowing system; the
//! scenarios in `core_scenarios` run the real core on it.

pub mod animation_scheduler;
pub mod animations;
pub mod appearance;
pub mod commands;
pub mod config_apply;
pub mod constraints;
pub mod container;
pub mod contract;
pub mod core;
pub mod decoration;
pub mod decoration_render;
pub mod event_coalescing;
pub mod events;
pub mod executor;
pub mod expose;
pub mod focus;
pub mod focus_manager;
pub mod gesture;
pub mod groups;
pub mod hot_corners;
pub mod input;
pub mod layouts;
pub mod lifecycle;
pub mod matching;
pub mod move_suppression;
pub mod outputs;
pub mod overlay_data;
pub mod persistence;
pub mod reload;
pub mod ring;
pub mod rule_state;
pub mod rules;
pub mod scratchpads;
pub mod shell_room;
pub mod workspace;
pub mod workspace_profile;
pub mod workspaces;
pub mod snap_zones;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod core_scenarios;

#[cfg(test)]
mod outside_input;
#[cfg(test)]
mod spoil;

#[cfg(test)]
mod random_runs;

pub use crate::core::WmCore;
pub use contract::{Desktop, Effect, WindowQueries};
pub use gevrek_model::geometry::{Padding, Rect};
/// The window id under the name the engine code was written with.
pub use gevrek_model::ids::WindowId as WindowHandle;
pub use gevrek_model::ids::{OutputId, WindowId};
