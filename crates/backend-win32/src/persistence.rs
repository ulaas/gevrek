#[cfg(test)]
use crate::dwm::MonitorHandle;
use crate::dwm::WindowHandle;
use anyhow::Result;
#[cfg(test)]
use chrono::Utc;
use std::path::{Path, PathBuf};

pub use gevrek_engine::persistence::{
    PersistentWindowSize, ResizeDimensionsState, WindowManagerState, WindowState,
    WorkspaceResizeState,
};

/// Where the session state file lives: in this instance's state folder
/// (`gevrek_model::paths`), so two gevreks on one machine (the one in use
/// and one being worked on) never read or write each other's.
pub fn default_state_path() -> PathBuf {
    let folder = gevrek_model::paths::state_dir();
    std::fs::create_dir_all(&folder).ok();
    let path = folder.join("window_state.json");
    bring_over_legacy_state(&path, legacy_state_path());
    path
}

/// Where the file was before there were instances: beside the
/// configuration (`%APPDATA%\gevrek`).
fn legacy_state_path() -> Option<PathBuf> {
    (gevrek_model::paths::instance() == gevrek_model::paths::MAIN_INSTANCE)
        .then(dirs::config_dir)
        .flatten()
        .map(|folder| folder.join("gevrek").join("window_state.json"))
}

/// The state saved at the old place is the main instance's: copied once,
/// when there is none at the new place yet. The old file stays (an older
/// gevrek may still be started).
fn bring_over_legacy_state(path: &Path, legacy: Option<PathBuf>) {
    let Some(legacy) = legacy else {
        return;
    };
    if !path.exists() && legacy.is_file() {
        match std::fs::copy(&legacy, path) {
            Ok(_) => log::info!(
                "The saved session was brought over from {} to {}",
                legacy.display(),
                path.display()
            ),
            Err(error) => log::warn!("{} could not be copied: {error}", legacy.display()),
        }
    }
}

/// Persistence manager for automatic save/load
pub struct PersistenceManager {
    state_path: PathBuf,
    auto_save_enabled: bool,
}

impl PersistenceManager {
    pub fn new() -> Self {
        Self {
            state_path: default_state_path(),
            auto_save_enabled: true,
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            state_path: path,
            auto_save_enabled: true,
        }
    }

    pub fn disable_auto_save(&mut self) {
        self.auto_save_enabled = false;
    }

    pub fn enable_auto_save(&mut self) {
        self.auto_save_enabled = true;
    }

    pub fn get_path(&self) -> &Path {
        &self.state_path
    }

    /// Save state to disk
    pub fn save_state(&self, state: &WindowManagerState) -> Result<()> {
        if self.auto_save_enabled {
            state.save(&self.state_path)?;
        }
        Ok(())
    }

    /// Load state from disk
    pub fn load_state(&self) -> Result<WindowManagerState> {
        if self.state_path.exists() {
            let mut state = WindowManagerState::load(&self.state_path)?;
            // The saved ids are from the run that wrote the file; only the
            // HWND value in them means something now.
            for window in &mut state.windows {
                window.handle = WindowHandle(crate::dwm::hwnd_map::id_for(
                    crate::dwm::hwnd_map::hwnd_of(window.handle.0),
                ));
            }
            Ok(state)
        } else {
            log::warn!(
                "State file not found at {}, returning empty state",
                self.state_path.display()
            );
            Ok(WindowManagerState::new())
        }
    }

    /// Check if state file exists
    pub fn has_saved_state(&self) -> bool {
        self.state_path.exists()
    }

    /// Delete saved state
    pub fn clear_state(&self) -> Result<()> {
        if self.state_path.exists() {
            std::fs::remove_file(&self.state_path)?;
            log::info!("Deleted state file at {}", self.state_path.display());
        }
        Ok(())
    }
}

impl Default for PersistenceManager {
    fn default() -> Self {
        Self::new()
    }
}

// Note: chrono dependency needed for timestamps
// Add to Cargo.toml: chrono = { version = "0.4", features = ["serde"] }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_manager_state_serialization() {
        let mut state = WindowManagerState::new();
        state.windows.push(WindowState {
            handle: WindowHandle(123),
            title: "Test Window".to_string(),
            exe: "test.exe".to_string(),
            class: "TestClass".to_string(),
            x: 100,
            y: 100,
            width: 800,
            height: 600,
            is_minimized: false,
            is_maximized: false,
            workspace_id: Some(0),
            workspace_name: None,
            monitor: Some(MonitorHandle(1)),
            container_index: Some(0),
            is_floating: false,
        });
        state.persistent_window_sizes.push(PersistentWindowSize {
            exe: "test.exe".to_string(),
            initial_class: "TestClass".to_string(),
            initial_title: "Test Window".to_string(),
            width: 960,
            height: 640,
        });

        let json = serde_json::to_string_pretty(&state).unwrap();
        assert!(json.contains("Test Window"));
        assert!(json.contains("test.exe"));

        let deserialized: WindowManagerState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.windows.len(), 1);
        assert_eq!(deserialized.windows[0].title, "Test Window");
        assert_eq!(deserialized.persistent_window_sizes.len(), 1);
        assert_eq!(deserialized.persistent_window_sizes[0].width, 960);
    }

    #[test]
    fn the_state_saved_at_the_old_place_is_brought_over_once() {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-legacy-state-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let (legacy, path) = (folder.join("old.json"), folder.join("window_state.json"));

        // Nothing at the old place: nothing happens.
        bring_over_legacy_state(&path, Some(legacy.clone()));
        assert!(!path.exists());

        std::fs::write(&legacy, b"old").unwrap();
        bring_over_legacy_state(&path, Some(legacy.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert!(legacy.exists(), "the old file stays for an older gevrek");

        // What was saved since is not written over.
        std::fs::write(&path, b"new").unwrap();
        bring_over_legacy_state(&path, Some(legacy));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");

        // Another instance than the main one has no old place.
        let other = folder.join("other.json");
        bring_over_legacy_state(&other, None);
        assert!(!other.exists());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn persistent_sizes_survive_save_and_reload() {
        let path = std::env::temp_dir().join(format!(
            "gevrek-persistent-size-{}-{}.json",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let manager = PersistenceManager::with_path(path.clone());
        let mut state = WindowManagerState::new();
        state.persistent_window_sizes.push(PersistentWindowSize {
            exe: "credential-ui.exe".to_string(),
            initial_class: "CredentialDialog".to_string(),
            initial_title: "Sign in".to_string(),
            width: 720,
            height: 520,
        });

        manager.save_state(&state).unwrap();
        let reloaded = manager.load_state().unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(
            reloaded.persistent_window_sizes,
            state.persistent_window_sizes
        );
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gevrek-{name}-{}-{}.json",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn resize_dimensions_state_round_trips_through_json() {
        let mut state = ResizeDimensionsState::new(75);
        state.workspaces.push(WorkspaceResizeState {
            monitor_index: 0,
            workspace_index: 1,
            name: "code".to_string(),
            layout: crate::dwm::layouts::LayoutType::BSP,
            master_ratio: 0.6,
            master_count: 2,
            bsp_split_ratios: vec![0.65, 0.4],
        });
        state.workspaces.push(WorkspaceResizeState {
            monitor_index: 1,
            workspace_index: 0,
            name: "web".to_string(),
            layout: crate::dwm::layouts::LayoutType::Columns,
            master_ratio: 0.5,
            master_count: 1,
            bsp_split_ratios: Vec::new(),
        });

        let path = unique_temp_path("resize-roundtrip");
        state.save(&path).unwrap();
        let reloaded = ResizeDimensionsState::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(reloaded, state);
        assert_eq!(reloaded.resize_delta, 75);
        assert_eq!(reloaded.workspaces[0].bsp_split_ratios, vec![0.65, 0.4]);
    }

    #[test]
    fn resize_dimensions_state_missing_bsp_ratios_default_to_empty() {
        let json = r#"{
            "version": "0.1.0",
            "timestamp": "2026-08-01T00:00:00Z",
            "resize_delta": 50,
            "workspaces": [{
                "monitor_index": 0,
                "workspace_index": 0,
                "name": "code",
                "layout": "BSP",
                "master_ratio": 0.5,
                "master_count": 1
            }]
        }"#;
        let state: ResizeDimensionsState = serde_json::from_str(json).unwrap();
        assert!(state.workspaces[0].bsp_split_ratios.is_empty());
    }

    #[test]
    fn resize_dimensions_load_rejects_malformed_json() {
        let path = unique_temp_path("resize-malformed");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(ResizeDimensionsState::load(&path).is_err());
        let _ = std::fs::remove_file(&path);
        assert!(ResizeDimensionsState::load(&path).is_err());
    }

    #[test]
    fn resize_dimensions_quick_path_is_a_temp_file() {
        let path = ResizeDimensionsState::quick_path();
        assert_eq!(path.file_name().unwrap(), "gevrek.resize.json");
        assert!(path.starts_with(std::env::temp_dir()));
    }
}
