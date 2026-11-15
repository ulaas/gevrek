use super::ring::Ring;
use crate::WindowHandle;
pub use gevrek_model::focus::FocusConfig;

/// Focus Manager - tracks focus history and provides focus cycling
///
/// Features:
/// - Maintains a history ring of focused windows
/// - Supports Alt-Tab style cycling (forward/backward)
/// - Focus follows mouse (optional)
/// - Focus stealing prevention
pub struct FocusManager {
    /// Configuration
    config: FocusConfig,
    /// History of focused windows (most recent first)
    history: Ring<WindowHandle>,
    /// Currently focused window
    current: Option<WindowHandle>,
    /// Index for cycling through history (for Alt-Tab behavior)
    cycle_index: usize,
    /// Whether we're currently in cycling mode
    cycling: bool,
}

impl FocusManager {
    pub fn new() -> Self {
        Self::with_config(FocusConfig::default())
    }

    pub fn with_config(config: FocusConfig) -> Self {
        Self {
            history: Ring::with_capacity(config.history_size),
            config,
            current: None,
            cycle_index: 0,
            cycling: false,
        }
    }

    pub fn config(&self) -> &FocusConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut FocusConfig {
        &mut self.config
    }

    pub fn set_config(&mut self, config: FocusConfig) {
        self.config = config;
        // Resize history if needed
        while self.history.len() > self.config.history_size {
            self.history.pop_back();
        }
    }

    /// Get the currently focused window
    pub fn current(&self) -> Option<WindowHandle> {
        self.current
    }

    /// Get the focus history (most recent first)
    pub fn history(&self) -> impl Iterator<Item = &WindowHandle> {
        self.history.iter()
    }

    /// Get the last N focused windows
    pub fn recent(&self, n: usize) -> Vec<WindowHandle> {
        self.history.iter().take(n).copied().collect()
    }

    /// Record a focus change
    ///
    /// Returns true if the focus was actually changed
    pub fn record_focus(&mut self, handle: WindowHandle) -> bool {
        // Don't record if it's the same as current
        if self.current == Some(handle) {
            return false;
        }

        // Reset cycling state when a new focus is recorded
        self.cycling = false;
        self.cycle_index = 0;

        // Remove from history if already present (to move to front)
        self.remove_from_history(handle);

        // Add to front of history
        self.history.push_front(handle);

        // Trim history to max size
        while self.history.len() > self.config.history_size {
            self.history.pop_back();
        }

        self.current = Some(handle);
        true
    }

    /// Remove a window from tracking (when destroyed)
    pub fn remove_window(&mut self, handle: WindowHandle) {
        self.remove_from_history(handle);

        if self.current == Some(handle) {
            // Focus the next window in history
            self.current = self.history.focused().copied();
        }
    }

    /// Get the previous window in focus history
    pub fn previous(&self) -> Option<WindowHandle> {
        self.history.get(1).copied()
    }

    /// Start focus cycling (Alt-Tab behavior)
    ///
    /// Call this when the user initiates cycling (e.g., presses Alt+Tab)
    pub fn start_cycle(&mut self) {
        self.cycling = true;
        self.cycle_index = 0;
    }

    /// Move to next window in cycle
    ///
    /// Returns the window to focus, or None if history is empty
    pub fn cycle_next(&mut self) -> Option<WindowHandle> {
        if self.history.is_empty() {
            return None;
        }

        if !self.cycling {
            self.start_cycle();
        }

        self.cycle_index = (self.cycle_index + 1) % self.history.len();
        self.history.get(self.cycle_index).copied()
    }

    /// Move to previous window in cycle
    ///
    /// Returns the window to focus, or None if history is empty
    pub fn cycle_previous(&mut self) -> Option<WindowHandle> {
        if self.history.is_empty() {
            return None;
        }

        if !self.cycling {
            self.start_cycle();
        }

        if self.cycle_index == 0 {
            self.cycle_index = self.history.len() - 1;
        } else {
            self.cycle_index -= 1;
        }
        self.history.get(self.cycle_index).copied()
    }

    /// Confirm the current cycle selection
    ///
    /// Call this when the user releases the modifier key (e.g., releases Alt)
    /// This commits the cycled window as the new focus
    pub fn confirm_cycle(&mut self) {
        if self.cycling {
            if let Some(handle) = self.history.get(self.cycle_index).copied() {
                // Move the selected window to front of history
                self.remove_from_history(handle);
                self.history.push_front(handle);
                self.current = Some(handle);
            }
        }
        self.cycling = false;
        self.cycle_index = 0;
    }

    /// Cancel cycling and return to original window
    pub fn cancel_cycle(&mut self) -> Option<WindowHandle> {
        self.cycling = false;
        self.cycle_index = 0;
        self.current
    }

    /// Check if we're currently in cycling mode
    pub fn is_cycling(&self) -> bool {
        self.cycling
    }

    /// Get the window at the current cycle position
    pub fn cycle_current(&self) -> Option<WindowHandle> {
        if self.cycling {
            self.history.get(self.cycle_index).copied()
        } else {
            self.current
        }
    }

    /// Check if a window is allowed to steal focus
    pub fn can_steal_focus(&self, exe_name: &str) -> bool {
        if !self.config.prevent_focus_steal {
            return true;
        }
        self.config.focus_steal_whitelist.contains(exe_name)
    }

    /// Add an app to focus steal whitelist
    pub fn allow_focus_steal(&mut self, exe_name: String) {
        self.config.focus_steal_whitelist.insert(exe_name);
    }

    /// Remove an app from focus steal whitelist
    pub fn disallow_focus_steal(&mut self, exe_name: &str) {
        self.config.focus_steal_whitelist.remove(exe_name);
    }

    /// Get focus follows mouse setting
    pub fn focus_follows_mouse(&self) -> bool {
        self.config.focus_follows_mouse
    }

    /// Get mouse-follows-focus (cursor warping) setting
    pub fn mouse_follows_focus(&self) -> bool {
        self.config.mouse_follows_focus
    }

    /// Set mouse-follows-focus (cursor warping)
    pub fn set_mouse_follows_focus(&mut self, enabled: bool) {
        self.config.mouse_follows_focus = enabled;
    }

    /// Set focus follows mouse
    pub fn set_focus_follows_mouse(&mut self, enabled: bool) {
        self.config.focus_follows_mouse = enabled;
    }

    /// Helper to remove a window from history
    fn remove_from_history(&mut self, handle: WindowHandle) {
        // Find and remove the window from history
        let mut idx_to_remove = None;
        for (i, h) in self.history.iter().enumerate() {
            if *h == handle {
                idx_to_remove = Some(i);
                break;
            }
        }
        if let Some(idx) = idx_to_remove {
            self.history.remove_at(idx);
        }
    }
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_history() {
        let mut fm = FocusManager::new();

        fm.record_focus(WindowHandle(1));
        fm.record_focus(WindowHandle(2));
        fm.record_focus(WindowHandle(3));

        assert_eq!(fm.current(), Some(WindowHandle(3)));
        assert_eq!(fm.previous(), Some(WindowHandle(2)));

        let history: Vec<_> = fm.recent(3);
        assert_eq!(
            history,
            vec![WindowHandle(3), WindowHandle(2), WindowHandle(1)]
        );
    }

    #[test]
    fn test_focus_cycle() {
        let mut fm = FocusManager::new();

        fm.record_focus(WindowHandle(1));
        fm.record_focus(WindowHandle(2));
        fm.record_focus(WindowHandle(3));

        // Start cycling
        fm.start_cycle();
        assert!(fm.is_cycling());

        // Cycle forward
        let next = fm.cycle_next();
        assert_eq!(next, Some(WindowHandle(2)));

        let next = fm.cycle_next();
        assert_eq!(next, Some(WindowHandle(1)));

        // Wrap around
        let next = fm.cycle_next();
        assert_eq!(next, Some(WindowHandle(3)));

        // Confirm selection
        fm.confirm_cycle();
        assert!(!fm.is_cycling());
    }

    #[test]
    fn test_remove_window() {
        let mut fm = FocusManager::new();

        fm.record_focus(WindowHandle(1));
        fm.record_focus(WindowHandle(2));
        fm.record_focus(WindowHandle(3));

        fm.remove_window(WindowHandle(2));

        let history: Vec<_> = fm.recent(3);
        assert_eq!(history, vec![WindowHandle(3), WindowHandle(1)]);
    }

    #[test]
    fn test_no_duplicate_history() {
        let mut fm = FocusManager::new();

        fm.record_focus(WindowHandle(1));
        fm.record_focus(WindowHandle(2));
        fm.record_focus(WindowHandle(1)); // Re-focus window 1

        assert_eq!(fm.current(), Some(WindowHandle(1)));

        // Window 1 should be at front, 2 behind
        let history: Vec<_> = fm.recent(3);
        assert_eq!(history, vec![WindowHandle(1), WindowHandle(2)]);
    }

    #[test]
    fn mouse_follows_focus_defaults_off_and_round_trips() {
        let config = FocusConfig::default();
        assert!(!config.mouse_follows_focus);

        // Config files written before the flag existed still parse.
        let legacy: FocusConfig = serde_json::from_str(
            r#"{
                "history_size": 20,
                "focus_follows_mouse": true,
                "focus_delay_ms": 100,
                "prevent_focus_steal": true,
                "focus_steal_whitelist": []
            }"#,
        )
        .unwrap();
        assert!(legacy.focus_follows_mouse);
        assert!(!legacy.mouse_follows_focus);

        let mut fm = FocusManager::new();
        assert!(!fm.mouse_follows_focus());
        fm.set_mouse_follows_focus(true);
        assert!(fm.mouse_follows_focus());
        assert!(fm.config().mouse_follows_focus);
    }
}
