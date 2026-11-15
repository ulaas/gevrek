//! State the WM saves to files and reads back: remembered window sizes and
//! the per-workspace resize dimensions. Plain data and file I/O; where the
//! whole-session state file lives is the host's business.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::outputs::MonitorHandle;
use crate::WindowHandle;

/// Floating-window size remembered independently of a currently open window.
///
/// The initial title and class are used because live titles can change during
/// a session. Executable identity prevents unrelated classes with the same
/// caption from sharing geometry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistentWindowSize {
    pub exe: String,
    pub initial_class: String,
    pub initial_title: String,
    pub width: i32,
    pub height: i32,
}

/// Resize-relevant layout state of one workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceResizeState {
    /// Index of the monitor in the runtime's ordered monitor list
    pub monitor_index: usize,
    /// Index of the workspace in that monitor's workspace ring
    pub workspace_index: usize,
    /// Workspace name at save time (diagnostics; matching is by index)
    pub name: String,
    /// Layout type in effect when the snapshot was taken
    pub layout: crate::layouts::LayoutType,
    /// Master area ratio (master-family layouts)
    pub master_ratio: f32,
    /// Number of master windows (master-family layouts)
    pub master_count: usize,
    /// Per-split BSP ratios; empty means even splits
    #[serde(default)]
    pub bsp_split_ratios: Vec<f32>,
}

/// Snapshot of every workspace's resize dimensions plus the session resize
/// delta, written by `save-resize` and consumed by `load-resize`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResizeDimensionsState {
    pub version: String,
    pub timestamp: String,
    /// Session resize delta in pixels (1-1000)
    pub resize_delta: i32,
    pub workspaces: Vec<WorkspaceResizeState>,
}

impl ResizeDimensionsState {
    pub fn new(resize_delta: i32) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now().to_rfc3339(),
            resize_delta,
            workspaces: Vec::new(),
        }
    }

    /// Save the snapshot to a JSON file, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        log::info!(
            "Saved {} workspace resize snapshot(s) to {}",
            self.workspaces.len(),
            path.display()
        );
        Ok(())
    }

    /// Load a snapshot from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let state: ResizeDimensionsState = serde_json::from_str(&content)?;
        log::info!(
            "Loaded {} workspace resize snapshot(s) from {}",
            state.workspaces.len(),
            path.display()
        );
        Ok(state)
    }

    /// Standard temp-file location used by `quick-save-resize` and
    /// `quick-load-resize`.
    pub fn quick_path() -> PathBuf {
        std::env::temp_dir().join("gevrek.resize.json")
    }
}

/// Persisted state of a window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub handle: WindowHandle,
    pub title: String,
    pub exe: String,
    pub class: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_minimized: bool,
    pub is_maximized: bool,
    pub workspace_id: Option<usize>,
    /// The workspace's name; a restore looks the workspace up by it, so a
    /// changed order of workspaces does not send the window elsewhere.
    #[serde(default)]
    pub workspace_name: Option<String>,
    pub monitor: Option<MonitorHandle>,
    /// Whether this window was floating (true) or tiled (false)
    #[serde(default)]
    pub is_floating: bool,
    /// Container index within workspace (for tiled windows)
    #[serde(default)]
    pub container_index: Option<usize>,
}

/// Complete window manager state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowManagerState {
    pub version: String,
    pub timestamp: String,
    pub windows: Vec<WindowState>,
    /// Monitor handle -> focused workspace index
    pub focused_workspaces: HashMap<MonitorHandle, usize>,
    /// Size cache for windows matched by a persistent-size rule. This survives
    /// application closure and graceful WM restarts.
    #[serde(default)]
    pub persistent_window_sizes: Vec<PersistentWindowSize>,
}

impl WindowManagerState {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now().to_rfc3339(),
            windows: Vec::new(),
            focused_workspaces: HashMap::new(),
            persistent_window_sizes: Vec::new(),
        }
    }

    /// Save state to JSON file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        log::info!("Saved window manager state to {}", path.display());
        log::debug!(
            "Saved {} windows, {} monitors",
            self.windows.len(),
            self.focused_workspaces.len()
        );
        Ok(())
    }

    /// Load state from JSON file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let state: WindowManagerState = serde_json::from_str(&content)?;
        log::info!("Loaded window manager state from {}", path.display());
        log::debug!(
            "Loaded {} windows from {}",
            state.windows.len(),
            state.timestamp
        );
        Ok(state)
    }
}

impl Default for WindowManagerState {
    fn default() -> Self {
        Self::new()
    }
}
