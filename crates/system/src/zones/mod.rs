//! The zones' backends: what `gevrek_zones::backend::ZonesBackend` asks,
//! answered by the system gevrek runs on. Each system has its own file.
//!
//! Both give the same names for what the shell uses beside the trait, so
//! that the shell is written once: `SystemBackend` (made with
//! `SystemBackend::new(wake)`, where `wake` may be called from any thread
//! and means "call `drain_events`"), `SystemBackend::picture(handle)`
//! (the pixels of an `IconHandle`, as a `crate::icons::Picture`), and
//! `pixels(IconSize)` (the side of a picture in pixels).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;
