//! The zones' backends: what `gevrek_zones::backend::ZonesBackend` asks,
//! answered by the system gevrek runs on. Each system has its own file.
//!
//! Both give the same names for what the shell uses beside the trait, so
//! that the shell is written once: `SystemBackend` (made with
//! `SystemBackend::new(wake)`, where `wake` may be called from any thread
//! and means "call `drain_events`"), `SystemBackend::picture(handle)`
//! (the pixels of an `IconHandle`, as a `crate::icons::Picture`), and
//! `pixels(IconSize)` (the side of a picture in pixels).
//!
//! What both do alike is written once, beside them:
//!
//! - [`pictures`]: icons and small pictures by handle, with a bound on
//!   how many bytes of them are kept.
//! - [`reach`]: how a folder a portal shows stands (being read, there,
//!   away), and how long one may take to answer.
//! - [`sources`], [`smart`], [`programs`]: what smart zones look through
//!   that is no plain folder (recent files, downloads, launchers, the
//!   programs that run).
//!
//! And these names are the same on both, for the shell:
//! `set_wm_state(&json)` and `set_commander(|command| ...)` (the programs
//! that run, and giving one the keyboard), `pin_launcher`,
//! `pinned_launchers`, `set_pinned_launchers`, `installed_programs`,
//! `unreachable()` and `retry_unreachable()` (folders that are away),
//! `set_picture_budget(bytes)`.

pub mod operations;
pub mod pictures;
pub mod programs;
pub mod reach;
pub mod smart;
pub mod sources;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;
