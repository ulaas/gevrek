//! gevrek-host: what every WM host process needs and no windowing system
//! provides: the configuration store the IPC server and the WM loop share,
//! the watcher of the configuration file, the programs started for the
//! user, the commands the server hands to the loop, and the heartbeat of
//! the loop.
//!
//! Portable: the Windows host (`gevrek-win32`) and the Wayland host
//! (`gevrek-smithay`) both use it.

pub mod backend_info;
pub mod command;
pub mod config_store;
pub mod config_watch;
pub mod heartbeat;
pub mod input_injection;
pub mod lock;
pub mod process_manager;
pub mod shortcuts;
pub mod wake;
