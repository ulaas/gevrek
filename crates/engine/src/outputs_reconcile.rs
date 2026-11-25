//! Display topology changes (dock/undock, monitor power, resolution, DPI,
//! taskbar moves) applied to the live monitor model.
//!
//! Monitors are identified by their device interface path, which is stable
//! for a physical display on a port across reconnects. `HMONITOR` values are
//! not: Windows hands out new ones on every topology change, and workspaces
//! keyed only by handle used to be dropped (windows included) on undock.
//!
//! On detach, a monitor's workspaces are kept in a cache and its windows are
//! moved to the primary monitor's visible workspace, so none stays hidden on
//! a display that no longer exists. On reattach, the workspaces come back and
//! every evacuated window that is still where it was put returns to its
//! original workspace.

use super::{MonitorHandle, MonitorInfo, Outputs};
use crate::ring::Ring;
use crate::workspace::Workspace;
use crate::WindowHandle;
use gevrek_model::settings::ReservedArea;

/// A window moved off a detached monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvacuatedWindow {
    handle: WindowHandle,
    workspace_index: usize,
    floating: bool,
}

/// Everything needed to restore a monitor when it comes back.
#[derive(Debug, Clone)]
pub(crate) struct DetachedMonitor {
    workspaces: Ring<Workspace>,
    reserved: Option<ReservedArea>,
    previous_workspace: Option<usize>,
    evacuated: Vec<EvacuatedWindow>,
    /// Workspace (by id) that received the evacuated windows.
    host_workspace_id: Option<usize>,
}

/// What a reconciliation changed; the runtime turns this into effects.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MonitorChanges {
    /// Same display, new `HMONITOR` (old, new).
    pub remapped: Vec<(MonitorHandle, MonitorHandle)>,
    /// Displays seen for the first time.
    pub attached: Vec<MonitorHandle>,
    /// Displays restored from the detach cache.
    pub reattached: Vec<MonitorHandle>,
    /// Stable keys of displays that went away.
    pub detached: Vec<String>,
    /// A monitor rectangle, work area, DPI, or primary flag changed.
    pub geometry_changed: bool,
    /// Windows that must become visible (moved onto a visible workspace).
    pub windows_to_show: Vec<WindowHandle>,
    /// Windows that must be hidden (moved back onto a hidden workspace).
    pub windows_to_hide: Vec<WindowHandle>,
}

impl MonitorChanges {
    pub fn topology_changed(&self) -> bool {
        !self.remapped.is_empty()
            || !self.attached.is_empty()
            || !self.reattached.is_empty()
            || !self.detached.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.topology_changed() && !self.geometry_changed
    }

    /// The monitor handle that replaced `old`, if it was remapped.
    pub fn remap(&self, old: MonitorHandle) -> Option<MonitorHandle> {
        self.remapped
            .iter()
            .find(|(from, _)| *from == old)
            .map(|(_, to)| *to)
    }
}

/// Stable identity of a display.
fn monitor_key(info: &MonitorInfo) -> String {
    if info.device_path.is_empty() {
        // Test monitors and drivers without an interface path.
        info.name.clone()
    } else {
        info.device_path.to_ascii_lowercase()
    }
}

fn same_geometry(a: &MonitorInfo, b: &MonitorInfo) -> bool {
    (a.x, a.y, a.width, a.height) == (b.x, b.y, b.width, b.height)
        && (a.work_x, a.work_y, a.work_width, a.work_height)
            == (b.work_x, b.work_y, b.work_width, b.work_height)
        && a.dpi_scale == b.dpi_scale
        && a.is_primary == b.is_primary
}

impl Outputs {
    /// Apply a freshly enumerated monitor set. An empty set is ignored:
    /// Windows briefly reports no displays during some transitions, and
    /// evacuating into nothing would lose every window.
    pub fn reconcile_with(&mut self, current: Vec<MonitorInfo>) -> MonitorChanges {
        let mut changes = MonitorChanges::default();
        if current.is_empty() {
            log::warn!("Display change reported no monitors; keeping the current model");
            return changes;
        }

        let old_monitors = std::mem::take(&mut self.monitors);
        let mut old_by_key: std::collections::HashMap<String, MonitorInfo> = old_monitors
            .into_values()
            .map(|info| (monitor_key(&info), info))
            .collect();

        let mut reattach_keys = Vec::new();
        for info in &current {
            let key = monitor_key(info);
            match old_by_key.remove(&key) {
                Some(old) => {
                    if old.handle != info.handle {
                        log::debug!(
                            "the display {} has another handle now ({:?} for {:?}): its workspaces go along",
                            info.name,
                            info.handle,
                            old.handle
                        );
                        self.move_monitor_state(old.handle, info.handle);
                        changes.remapped.push((old.handle, info.handle));
                    }
                    if !same_geometry(&old, info) {
                        log::debug!(
                            "the display {} changed: {}x{} at ({}, {}), room for windows {}x{} at ({}, {}), scale {}{}",
                            info.name,
                            info.width,
                            info.height,
                            info.x,
                            info.y,
                            info.work_width,
                            info.work_height,
                            info.work_x,
                            info.work_y,
                            info.dpi_scale,
                            if info.is_primary { ", the primary one" } else { "" }
                        );
                        changes.geometry_changed = true;
                    }
                }
                None if self.detached.contains_key(&key) => reattach_keys.push((key, info.handle)),
                None => {
                    log::debug!(
                        "the display {} is new ({}x{} at ({}, {})): it gets an empty ring of workspaces",
                        info.name,
                        info.width,
                        info.height,
                        info.x,
                        info.y
                    );
                    self.workspaces.entry(info.handle).or_default();
                    changes.attached.push(info.handle);
                }
            }
        }

        self.primary_monitor = current
            .iter()
            .find(|info| info.is_primary)
            .or_else(|| current.first())
            .map(|info| info.handle);
        self.monitors = current
            .into_iter()
            .map(|info| (info.handle, info))
            .collect();

        // Detach before reattach: a dock swap can do both at once, and the
        // reattached monitor must not receive windows from the detached one.
        for (key, old) in old_by_key {
            self.detach_monitor(&key, old.handle, &mut changes);
            changes.detached.push(key);
        }
        for (key, handle) in reattach_keys {
            self.reattach_monitor(&key, handle, &mut changes);
            changes.reattached.push(handle);
        }

        if changes.topology_changed() {
            changes.geometry_changed = true;
        }
        changes
    }

    fn move_monitor_state(&mut self, from: MonitorHandle, to: MonitorHandle) {
        if let Some(ring) = self.workspaces.remove(&from) {
            self.workspaces.insert(to, ring);
        }
        if let Some(reserved) = self.reserved_areas.remove(&from) {
            self.reserved_areas.insert(to, reserved);
        }
        if let Some(previous) = self.previous_workspace.remove(&from) {
            self.previous_workspace.insert(to, previous);
        }
    }

    fn detach_monitor(&mut self, key: &str, handle: MonitorHandle, changes: &mut MonitorChanges) {
        let mut ring = self.workspaces.remove(&handle).unwrap_or_default();
        let reserved = self.reserved_areas.remove(&handle);
        let previous_workspace = self.previous_workspace.remove(&handle);

        // Where the windows go: the workspace the primary display shows.
        // A display that takes the only one's place has none yet (it is
        // given its workspaces after this): it gets the ring of the one
        // that went, windows and all, instead of the windows being taken
        // out of their workspaces and put nowhere.
        let host_ready = self.primary_monitor.is_some_and(|primary| {
            self.workspaces
                .get(&primary)
                .is_some_and(|workspaces| workspaces.focused().is_some())
        });
        if !host_ready {
            if let Some(primary) = self.primary_monitor.filter(|primary| *primary != handle) {
                if self
                    .workspaces
                    .get(&primary)
                    .is_none_or(|workspaces| workspaces.is_empty())
                    && !ring.is_empty()
                {
                    log::info!(
                        "Monitor {key} detached with no other workspace to take its windows: its workspaces go to the primary monitor"
                    );
                    self.workspaces.insert(primary, ring);
                    if let Some(reserved) = reserved {
                        self.reserved_areas.entry(primary).or_insert(reserved);
                    }
                    if let Some(previous) = previous_workspace {
                        self.previous_workspace.insert(primary, previous);
                    }
                    return;
                }
            }
        }

        let mut evacuated = Vec::new();
        for (workspace_index, workspace) in ring.iter_mut().enumerate() {
            for window in workspace.all_windows() {
                let floating = workspace
                    .floating_windows
                    .iter()
                    .any(|floating| *floating == window);
                workspace.remove_window(window);
                evacuated.push(EvacuatedWindow {
                    handle: window,
                    workspace_index,
                    floating,
                });
            }
        }

        let host_workspace_id = self.primary_monitor.and_then(|primary| {
            let host = self.workspaces.get_mut(&primary)?.focused_mut()?;
            for window in &evacuated {
                if window.floating {
                    host.add_floating_window(window.handle);
                } else {
                    host.add_window(window.handle, true);
                }
            }
            Some(host.id)
        });
        if host_workspace_id.is_some() {
            // Windows from hidden workspaces were hidden by the WM.
            changes
                .windows_to_show
                .extend(evacuated.iter().map(|window| window.handle));
        } else {
            log::warn!("No visible workspace to receive windows from detached monitor {key}");
        }

        log::info!(
            "Monitor {key} detached: {} workspace(s) cached, {} window(s) moved to the primary monitor",
            ring.len(),
            evacuated.len()
        );
        self.detached.insert(
            key.to_string(),
            DetachedMonitor {
                workspaces: ring,
                reserved,
                previous_workspace,
                evacuated,
                host_workspace_id,
            },
        );
    }

    fn reattach_monitor(&mut self, key: &str, handle: MonitorHandle, changes: &mut MonitorChanges) {
        let Some(mut cached) = self.detached.remove(key) else {
            return;
        };
        let visible_index = cached.workspaces.focused_idx();
        let mut returned = 0;
        for window in &cached.evacuated {
            // Only windows still where they were put come back; a window the
            // user moved since is left alone.
            let still_hosted = self
                .find_window(window.handle)
                .is_some_and(|(monitor, index)| {
                    self.workspaces
                        .get(&monitor)
                        .and_then(|ring| ring.get(index))
                        .is_some_and(|workspace| Some(workspace.id) == cached.host_workspace_id)
                });
            if !still_hosted {
                continue;
            }
            self.find_and_remove_window(window.handle);
            let Some(workspace) = cached.workspaces.get_mut(window.workspace_index) else {
                continue;
            };
            if window.floating {
                workspace.add_floating_window(window.handle);
            } else {
                workspace.add_window(window.handle, true);
            }
            returned += 1;
            if window.workspace_index != visible_index {
                changes.windows_to_hide.push(window.handle);
            }
        }
        // Workspaces may be parked under this handle already: the only
        // display went as this one came back, and its ring was handed
        // to the display that takes its place (`detach_monitor`). Their
        // windows join the ring that comes back: the workspace of the
        // same name, else the one that is shown. (The ring that came
        // back was put in its place, and every window of the display
        // that went was known to nobody from then on.)
        if let Some(parked) = self.workspaces.remove(&handle) {
            let mut joined = 0;
            for workspace in parked.iter() {
                let index = crate::workspace::Workspace::position_called(
                    cached.workspaces.elements(),
                    &workspace.name,
                )
                .unwrap_or(visible_index);
                for window in workspace.all_windows() {
                    let floating = workspace
                        .floating_windows
                        .iter()
                        .any(|other| *other == window);
                    let Some(target) = cached.workspaces.get_mut(index) else {
                        continue;
                    };
                    if floating {
                        target.add_floating_window(window);
                    } else {
                        target.add_window(window, true);
                    }
                    joined += 1;
                    if index == visible_index {
                        changes.windows_to_show.push(window);
                    } else {
                        changes.windows_to_hide.push(window);
                    }
                }
            }
            log::info!("Monitor {key} took {joined} window(s) of the display that went as it came");
        }
        self.workspaces.insert(handle, cached.workspaces);
        if let Some(reserved) = cached.reserved {
            self.reserved_areas.insert(handle, reserved);
        }
        if let Some(previous) = cached.previous_workspace {
            self.previous_workspace.insert(handle, previous);
        }
        log::info!("Monitor {key} reattached: {returned} window(s) returned to their workspaces");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::LayoutType;

    fn display(handle: isize, path: &str, primary: bool, x: i32) -> MonitorInfo {
        MonitorInfo {
            handle: MonitorHandle(handle),
            name: format!("Display {handle}"),
            device_path: path.to_string(),
            stable_id: String::new(),
            is_primary: primary,
            x,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: x,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            dpi_scale: 1.0,
        }
    }

    const LAPTOP: &str =
        r"\\?\DISPLAY#BOE0A1B#4&1&UID8388688#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    const EXTERNAL: &str =
        r"\\?\DISPLAY#DEL4321#5&2&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";

    /// Laptop (primary) plus an external monitor with two workspaces: a
    /// visible one holding window 1 and a hidden one holding windows 2 and 3.
    fn docked() -> Outputs {
        let mut manager = Outputs::with_workspace_count(1);
        manager.reconcile_with(vec![
            display(1, LAPTOP, true, 0),
            display(2, EXTERNAL, false, 1920),
        ]);
        manager.create_workspace(MonitorHandle(1), "laptop".into(), LayoutType::BSP);
        manager.create_workspace(MonitorHandle(2), "web".into(), LayoutType::BSP);
        manager.create_workspace(MonitorHandle(2), "chat".into(), LayoutType::BSP);
        let ring = manager.get_workspaces_mut(MonitorHandle(2)).unwrap();
        ring.get_mut(0).unwrap().add_window(WindowHandle(1), true);
        ring.get_mut(1).unwrap().add_window(WindowHandle(2), true);
        ring.get_mut(1)
            .unwrap()
            .add_floating_window(WindowHandle(3));
        manager
    }

    #[test]
    fn a_new_hmonitor_for_the_same_display_keeps_its_workspaces() {
        let mut manager = docked();
        let changes = manager.reconcile_with(vec![
            display(11, LAPTOP, true, 0),
            display(12, EXTERNAL, false, 1920),
        ]);
        assert_eq!(
            changes.remapped,
            vec![
                (MonitorHandle(1), MonitorHandle(11)),
                (MonitorHandle(2), MonitorHandle(12))
            ]
        );
        assert_eq!(
            manager.find_window(WindowHandle(2)),
            Some((MonitorHandle(12), 1))
        );
        assert!(changes.windows_to_show.is_empty());
    }

    #[test]
    fn undock_moves_every_window_to_the_visible_primary_workspace() {
        let mut manager = docked();
        let changes = manager.reconcile_with(vec![display(1, LAPTOP, true, 0)]);

        assert_eq!(changes.detached.len(), 1);
        for window in [1, 2, 3] {
            assert_eq!(
                manager.find_window(WindowHandle(window)),
                Some((MonitorHandle(1), 0)),
                "window {window} was not evacuated"
            );
        }
        assert!(manager.is_window_floating(WindowHandle(3)));
        let mut shown = changes.windows_to_show.clone();
        shown.sort_by_key(|window| window.0);
        assert_eq!(
            shown,
            vec![WindowHandle(1), WindowHandle(2), WindowHandle(3)]
        );
        assert!(manager.get_workspaces(MonitorHandle(2)).is_none());
    }

    #[test]
    fn a_display_that_comes_back_as_the_only_one_keeps_the_windows_of_the_one_that_went() {
        let mut manager = docked();
        // Undocked: the external display's windows are on the laptop's.
        manager.reconcile_with(vec![display(1, LAPTOP, true, 0)]);
        // A window of the laptop's own, on a workspace of its own.
        manager.create_workspace(MonitorHandle(1), "scratch".into(), LayoutType::BSP);
        manager
            .get_workspaces_mut(MonitorHandle(1))
            .unwrap()
            .get_mut(1)
            .unwrap()
            .add_window(WindowHandle(9), true);
        // Docked with the lid closed: the laptop's display is gone and
        // the external one is back, in the same look at the displays.
        manager.reconcile_with(vec![display(7, EXTERNAL, true, 0)]);
        for window in [1, 2, 3, 9] {
            assert_eq!(
                manager
                    .find_window(WindowHandle(window))
                    .map(|(monitor, _)| monitor),
                Some(MonitorHandle(7)),
                "window {window} is known on the display that is left"
            );
        }
    }

    #[test]
    fn redock_restores_workspaces_and_returns_windows() {
        let mut manager = docked();
        manager.reconcile_with(vec![display(1, LAPTOP, true, 0)]);
        // The user moves window 1 elsewhere while undocked; it stays put.
        manager.find_and_remove_window(WindowHandle(1));
        manager.create_workspace(MonitorHandle(1), "scratch".into(), LayoutType::BSP);
        manager
            .get_workspaces_mut(MonitorHandle(1))
            .unwrap()
            .get_mut(1)
            .unwrap()
            .add_window(WindowHandle(1), true);

        let changes = manager.reconcile_with(vec![
            display(1, LAPTOP, true, 0),
            display(7, EXTERNAL, false, 1920),
        ]);

        assert_eq!(changes.reattached, vec![MonitorHandle(7)]);
        let ring = manager.get_workspaces(MonitorHandle(7)).unwrap();
        assert_eq!(
            ring.iter()
                .map(|workspace| workspace.name.clone())
                .collect::<Vec<_>>(),
            vec!["web".to_string(), "chat".to_string()]
        );
        assert_eq!(
            manager.find_window(WindowHandle(2)),
            Some((MonitorHandle(7), 1))
        );
        assert_eq!(
            manager.find_window(WindowHandle(3)),
            Some((MonitorHandle(7), 1))
        );
        assert!(manager.is_window_floating(WindowHandle(3)));
        assert_eq!(
            manager.find_window(WindowHandle(1)),
            Some((MonitorHandle(1), 1))
        );
        // Workspace "chat" is not the visible one on the restored monitor.
        let mut hidden = changes.windows_to_hide.clone();
        hidden.sort_by_key(|window| window.0);
        assert_eq!(hidden, vec![WindowHandle(2), WindowHandle(3)]);
    }

    #[test]
    fn an_empty_enumeration_changes_nothing() {
        let mut manager = docked();
        assert!(manager.reconcile_with(Vec::new()).is_empty());
        assert_eq!(
            manager.find_window(WindowHandle(2)),
            Some((MonitorHandle(2), 1))
        );
    }

    #[test]
    fn a_taskbar_or_dpi_change_is_a_geometry_change_only() {
        let mut manager = docked();
        let mut external = display(2, EXTERNAL, false, 1920);
        external.work_height = 1000;
        let changes = manager.reconcile_with(vec![display(1, LAPTOP, true, 0), external]);
        assert!(changes.geometry_changed);
        assert!(!changes.topology_changed());
    }

    #[test]
    fn a_new_display_gets_an_empty_workspace_ring() {
        let mut manager = docked();
        let changes = manager.reconcile_with(vec![
            display(1, LAPTOP, true, 0),
            display(2, EXTERNAL, false, 1920),
            display(3, r"\\?\DISPLAY#NEW0001#x", false, 3840),
        ]);
        assert_eq!(changes.attached, vec![MonitorHandle(3)]);
        assert!(manager.get_workspaces(MonitorHandle(3)).unwrap().is_empty());
    }
}
