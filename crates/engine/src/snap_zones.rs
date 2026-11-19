use crate::container::Rect;
use crate::outputs::{MonitorHandle, Outputs as MonitorManager};
use crate::WindowHandle;
pub use gevrek_model::snap_zones::{
    presets, SnapZone, SnapZoneConfig, SnapZoneLayout, SnapZoneModifier,
};
use std::collections::HashMap;

/// Result of zone detection
#[derive(Debug, Clone)]
pub struct SnapZoneHit {
    /// The zone that was hit
    pub zone: SnapZone,
    /// The actual pixel rect of the zone
    pub rect: Rect,
    /// Overlap percentage (0.0 to 1.0)
    pub overlap: f32,
    /// Monitor this zone belongs to
    pub monitor: MonitorHandle,
}

/// Zone manager for handling zone layouts per monitor
pub struct SnapZoneManager {
    /// Configuration
    config: SnapZoneConfig,
    /// Layout per monitor (monitor handle -> layout)
    layouts: HashMap<MonitorHandle, SnapZoneLayout>,
    /// Default layout for monitors without specific assignment
    default_layout: SnapZoneLayout,
    /// Currently highlighted zone during drag
    active_zone: Option<SnapZoneHit>,
    /// Windows assigned to zones (window -> (monitor, zone_id))
    window_zones: HashMap<WindowHandle, (MonitorHandle, usize)>,
    /// Asks the backend whether a modifier key is held. Without one, a
    /// configured activation modifier counts as not held.
    modifier_probe: fn(SnapZoneModifier) -> bool,
}

impl SnapZoneManager {
    pub fn new(config: SnapZoneConfig) -> Self {
        Self {
            config,
            layouts: HashMap::new(),
            default_layout: presets::columns_2(),
            active_zone: None,
            window_zones: HashMap::new(),
            modifier_probe: |_| false,
        }
    }

    /// Give the manager the backend's way to read modifier keys.
    pub fn with_modifier_probe(mut self, probe: fn(SnapZoneModifier) -> bool) -> Self {
        self.modifier_probe = probe;
        self
    }

    pub fn with_default_layout(mut self, layout: SnapZoneLayout) -> Self {
        self.default_layout = layout;
        self
    }

    pub fn set_config(&mut self, config: SnapZoneConfig) {
        self.config = config;
    }

    pub fn config(&self) -> &SnapZoneConfig {
        &self.config
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled && self.activation_modifier_pressed()
    }

    fn activation_modifier_pressed(&self) -> bool {
        let Some(modifier) = self.config.activation_modifier else {
            return true;
        };
        (self.modifier_probe)(modifier)
    }

    /// Set layout for a specific monitor
    pub fn set_monitor_layout(&mut self, monitor: MonitorHandle, layout: SnapZoneLayout) {
        self.layouts.insert(monitor, layout);
    }

    /// Get layout for a monitor (returns default if not set)
    pub fn get_monitor_layout(&self, monitor: MonitorHandle) -> &SnapZoneLayout {
        self.layouts.get(&monitor).unwrap_or(&self.default_layout)
    }

    /// Set the default layout
    pub fn set_default_layout(&mut self, layout: SnapZoneLayout) {
        self.default_layout = layout;
    }

    /// Find which zone a window should snap to based on cursor position
    pub fn find_zone_at_cursor(
        &self,
        cursor_x: i32,
        cursor_y: i32,
        monitor_manager: &MonitorManager,
    ) -> Option<SnapZoneHit> {
        if !self.is_enabled() {
            return None;
        }

        // Find which monitor the cursor is on
        let monitors = monitor_manager.get_all_monitors();
        let current_monitor = monitors.iter().find(|m| {
            cursor_x >= m.work_x
                && cursor_x < m.work_x + m.work_width
                && cursor_y >= m.work_y
                && cursor_y < m.work_y + m.work_height
        })?;

        let work_area = Rect::new(
            current_monitor.work_x,
            current_monitor.work_y,
            current_monitor.work_width,
            current_monitor.work_height,
        );

        let layout = self.get_monitor_layout(current_monitor.handle);

        // Find zone containing cursor (prefer higher priority)
        let mut best_zone: Option<&SnapZone> = None;
        for zone in &layout.zones {
            if zone.contains_point(&work_area, cursor_x, cursor_y)
                && (best_zone.is_none() || zone.priority > best_zone.unwrap().priority)
            {
                best_zone = Some(zone);
            }
        }

        best_zone.map(|z| SnapZoneHit {
            zone: z.clone(),
            rect: z.to_rect(&work_area),
            overlap: 1.0,
            monitor: current_monitor.handle,
        })
    }

    /// Find which zone a window rect best overlaps with
    pub fn find_zone_for_rect(
        &self,
        window_rect: &Rect,
        monitor_manager: &MonitorManager,
    ) -> Option<SnapZoneHit> {
        if !self.is_enabled() {
            return None;
        }

        let window_center_x = window_rect.x + window_rect.width / 2;
        let window_center_y = window_rect.y + window_rect.height / 2;

        // Find which monitor the window center is on
        let monitors = monitor_manager.get_all_monitors();
        let current_monitor = monitors.iter().find(|m| {
            window_center_x >= m.work_x
                && window_center_x < m.work_x + m.work_width
                && window_center_y >= m.work_y
                && window_center_y < m.work_y + m.work_height
        })?;

        let work_area = Rect::new(
            current_monitor.work_x,
            current_monitor.work_y,
            current_monitor.work_width,
            current_monitor.work_height,
        );

        let layout = self.get_monitor_layout(current_monitor.handle);

        // Find zone with best overlap (considering priority)
        let mut best_hit: Option<SnapZoneHit> = None;

        for zone in &layout.zones {
            let overlap = zone.overlaps(&work_area, window_rect);
            if overlap >= self.config.snap_threshold {
                let is_better = match &best_hit {
                    None => true,
                    Some(current) => {
                        // Prefer higher priority, then higher overlap
                        zone.priority > current.zone.priority
                            || (zone.priority == current.zone.priority && overlap > current.overlap)
                    }
                };

                if is_better {
                    best_hit = Some(SnapZoneHit {
                        zone: zone.clone(),
                        rect: zone.to_rect(&work_area),
                        overlap,
                        monitor: current_monitor.handle,
                    });
                }
            }
        }

        best_hit
    }

    /// Get all zones for a monitor (for visualization)
    pub fn get_zones_for_monitor(
        &self,
        monitor: MonitorHandle,
        monitor_manager: &MonitorManager,
    ) -> Vec<(SnapZone, Rect)> {
        let monitors = monitor_manager.get_all_monitors();
        let monitor_info = monitors.iter().find(|m| m.handle == monitor);

        match monitor_info {
            Some(m) => {
                let work_area = Rect::new(m.work_x, m.work_y, m.work_width, m.work_height);
                let layout = self.get_monitor_layout(monitor);
                layout
                    .zones
                    .iter()
                    .map(|z| (z.clone(), z.to_rect(&work_area)))
                    .collect()
            }
            None => Vec::new(),
        }
    }

    /// Set the currently active (highlighted) zone during drag
    pub fn set_active_zone(&mut self, zone: Option<SnapZoneHit>) {
        self.active_zone = zone;
    }

    /// Get the currently active zone
    pub fn get_active_zone(&self) -> Option<&SnapZoneHit> {
        self.active_zone.as_ref()
    }

    /// Assign a window to a zone
    pub fn assign_window_to_zone(
        &mut self,
        window: WindowHandle,
        monitor: MonitorHandle,
        zone_id: usize,
    ) {
        self.window_zones.insert(window, (monitor, zone_id));
    }

    /// Remove window from zone tracking
    pub fn remove_window(&mut self, window: WindowHandle) {
        self.window_zones.remove(&window);
    }

    /// Get zone assignment for a window
    pub fn get_window_zone(&self, window: WindowHandle) -> Option<(MonitorHandle, usize)> {
        self.window_zones.get(&window).copied()
    }

    /// Snap a window to a specific zone by index
    pub fn snap_to_zone_index(
        &self,
        monitor: MonitorHandle,
        zone_index: usize,
        monitor_manager: &MonitorManager,
    ) -> Option<Rect> {
        let monitors = monitor_manager.get_all_monitors();
        let monitor_info = monitors.iter().find(|m| m.handle == monitor)?;

        let work_area = Rect::new(
            monitor_info.work_x,
            monitor_info.work_y,
            monitor_info.work_width,
            monitor_info.work_height,
        );

        let layout = self.get_monitor_layout(monitor);
        layout.zones.get(zone_index).map(|z| z.to_rect(&work_area))
    }

    /// Get zone count for a monitor
    pub fn zone_count(&self, monitor: MonitorHandle) -> usize {
        self.get_monitor_layout(monitor).zones.len()
    }
}

impl Default for SnapZoneManager {
    fn default() -> Self {
        Self::new(SnapZoneConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_to_rect() {
        let zone = SnapZone::new(0, 0.0, 0.0, 0.5, 1.0).with_gap(0);
        let work_area = Rect::new(0, 0, 1920, 1080);
        let rect = zone.to_rect(&work_area);

        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 960);
        assert_eq!(rect.height, 1080);
    }

    #[test]
    fn test_zone_with_gap() {
        let zone = SnapZone::new(0, 0.0, 0.0, 0.5, 1.0).with_gap(10);
        let work_area = Rect::new(0, 0, 1920, 1080);
        let rect = zone.to_rect(&work_area);

        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 10);
        assert_eq!(rect.width, 940);
        assert_eq!(rect.height, 1060);
    }

    #[test]
    fn test_zone_contains_point() {
        let zone = SnapZone::new(0, 0.0, 0.0, 0.5, 1.0).with_gap(0);
        let work_area = Rect::new(0, 0, 1920, 1080);

        assert!(zone.contains_point(&work_area, 100, 100));
        assert!(zone.contains_point(&work_area, 959, 500));
        assert!(!zone.contains_point(&work_area, 960, 500));
        assert!(!zone.contains_point(&work_area, 1500, 500));
    }

    #[test]
    fn test_preset_columns_2() {
        let layout = presets::columns_2();
        assert_eq!(layout.zones.len(), 2);
        assert_eq!(layout.zones[0].width_percent, 0.5);
        assert_eq!(layout.zones[1].width_percent, 0.5);
    }

    #[test]
    fn test_zone_overlap() {
        let zone = SnapZone::new(0, 0.0, 0.0, 0.5, 1.0).with_gap(0);
        let work_area = Rect::new(0, 0, 1920, 1080);

        // Window fully inside zone
        let window = Rect::new(100, 100, 400, 400);
        let overlap = zone.overlaps(&work_area, &window);
        assert_eq!(overlap, 1.0);

        // Window partially inside zone
        let window = Rect::new(800, 100, 400, 400);
        let overlap = zone.overlaps(&work_area, &window);
        assert!(overlap > 0.0 && overlap < 1.0);

        // Window outside zone
        let window = Rect::new(1000, 100, 400, 400);
        let overlap = zone.overlaps(&work_area, &window);
        assert_eq!(overlap, 0.0);
    }
}
