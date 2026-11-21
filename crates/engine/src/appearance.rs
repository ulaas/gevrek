//! How windows look, as far as the WM decides it: the appearance profile
//! applied to a new window, opacity and dimming by focus, shadows, and the
//! per-workspace state snapshot that reports the result.

use crate::contract::{Desktop, Effect, PlatformEffect, WindowsEffect};
use crate::core::WmCore;
use crate::overlay_data::{DetailedWindowInfo, MonitorData, ScratchpadStateData, WorkspaceData};
use crate::rules::RuleResolutionScope;
use crate::workspace::Workspace;
use crate::WindowHandle;
use gevrek_model::settings::ShadowPolicy;
use std::collections::HashMap;

/// The opacity a window gets: its base opacity, or the inactive opacity if
/// it is dimmed for not having the focus.
pub fn effective_window_alpha(
    transparency_enabled: bool,
    base_alpha: u8,
    focused: bool,
    dim_inactive: bool,
    inactive_alpha: u8,
    fullscreen: bool,
    dim_fullscreen: bool,
) -> u8 {
    // With the transparency system off,
    // every window renders fully opaque regardless of configured opacity.
    if !transparency_enabled {
        return 255;
    }
    if focused || !dim_inactive || (fullscreen && !dim_fullscreen) {
        base_alpha
    } else {
        base_alpha.min(inactive_alpha)
    }
}

fn try_effect(desktop: &mut dyn Desktop, what: &str, id: WindowHandle, effect: Effect) {
    if let Err(error) = desktop.apply(&effect) {
        log::debug!("Could not apply {what} to {id:?}: {error}");
    }
}

impl WmCore {
    /// Give a window the configured look when it appears.
    pub fn apply_appearance_to_window(
        &mut self,
        desktop: &mut dyn Desktop,
        config: &gevrek_model::settings::DesktopConfig,
        id: WindowHandle,
    ) {
        let appearance = &config.appearance;
        if desktop.is_decoration(id) {
            return;
        }
        // Only a window that is gevrek's has gevrek's look. This is asked
        // for every window that appears, and every menu, tooltip and
        // flyout is one: given round corners, a menu (whose window is
        // larger than what is seen of it, for its shadow) had the frame
        // Windows draws around a rounded window, a thin line well outside
        // the menu itself; and a backdrop, and gevrek's opacity.
        if desktop.ignored_by_rules(id) {
            return;
        }
        try_effect(
            desktop,
            "backdrop",
            id,
            Effect::Platform(PlatformEffect::Windows(WindowsEffect::Backdrop {
                id,
                backdrop: config.platform.windows.backdrop,
            })),
        );
        try_effect(
            desktop,
            "corner preference",
            id,
            Effect::Platform(PlatformEffect::Windows(WindowsEffect::Corner {
                id,
                corner: config.platform.windows.corners,
            })),
        );
        // (As solid as a rule has it for this window, else as the
        // appearance has every window: set here without the rule, a
        // rule's see-through window stood solid until the keyboard next
        // went to another.)
        let (workspace_name, workspace_index) = self.workspace_of(id);
        let resolution = self.executor.resolve_rules(
            desktop,
            id,
            workspace_name,
            workspace_index,
            RuleResolutionScope::All,
        );
        let alpha = if appearance.transparency_enabled {
            resolution.opacity.unwrap_or(appearance.window_opacity)
        } else {
            255
        };
        try_effect(desktop, "opacity", id, Effect::Opacity { id, alpha });
        try_effect(
            desktop,
            "shadow policy",
            id,
            Effect::Platform(PlatformEffect::Windows(WindowsEffect::Shadow {
                id,
                policy: config.platform.windows.shadow,
            })),
        );
    }

    /// The workspace a window is on: its name and index.
    fn workspace_of(&self, id: WindowHandle) -> (Option<String>, Option<usize>) {
        match self.monitor_manager.find_window(id) {
            Some((monitor, workspace_index)) => {
                let name = self
                    .monitor_manager
                    .get_workspaces(monitor)
                    .and_then(|workspaces| workspaces.get(workspace_index))
                    .map(|workspace| workspace.name.clone());
                (name, Some(workspace_index))
            }
            None => (None, None),
        }
    }

    /// Opacity and shadow of every window, by focus, the appearance profile
    /// and the window rules. Only what differs from the window's current
    /// look is sent.
    pub fn apply_focus_effects(
        &mut self,
        desktop: &mut dyn Desktop,
        config: &gevrek_model::settings::DesktopConfig,
    ) {
        let appearance = &config.appearance;
        let active = desktop.active_window();
        let handles: Vec<WindowHandle> = desktop
            .windows()
            .iter()
            .map(|window| window.handle)
            // (Gevrek's own windows only: one that a rule says to leave
            // alone is not made fainter for not having the keyboard.)
            .filter(|handle| !desktop.is_decoration(*handle) && !desktop.ignored_by_rules(*handle))
            .collect();

        for handle in handles {
            let (workspace_name, workspace_index) = self.workspace_of(handle);
            let resolution = self.executor.resolve_rules(
                desktop,
                handle,
                workspace_name,
                workspace_index,
                RuleResolutionScope::All,
            );
            let base_alpha = resolution.opacity.unwrap_or(appearance.window_opacity);
            let allow_dim = resolution.dim_inactive.unwrap_or(true);
            let effective_alpha = effective_window_alpha(
                appearance.transparency_enabled,
                base_alpha,
                active == Some(handle),
                appearance.dim_inactive && allow_dim,
                appearance.inactive_window_opacity,
                desktop.is_fullscreen(handle),
                appearance.dim_fullscreen,
            );
            if desktop.opacity(handle) != effective_alpha {
                try_effect(
                    desktop,
                    "focus opacity",
                    handle,
                    Effect::Opacity {
                        id: handle,
                        alpha: effective_alpha,
                    },
                );
            }

            let shadow = resolution
                .shadow
                .map_or(config.platform.windows.shadow, |enabled| {
                    if enabled {
                        ShadowPolicy::Enabled
                    } else {
                        ShadowPolicy::Disabled
                    }
                });
            if desktop.shadow_policy(handle) != shadow {
                try_effect(
                    desktop,
                    "shadow policy",
                    handle,
                    Effect::Platform(PlatformEffect::Windows(WindowsEffect::Shadow {
                        id: handle,
                        policy: shadow,
                    })),
                );
            }
        }
    }

    /// For the state snapshot: a window's opacity now, whether that is
    /// dimmed below its base opacity, and its shadow policy.
    pub fn window_effect_state(
        &self,
        desktop: &dyn Desktop,
        config: &gevrek_model::settings::DesktopConfig,
        id: WindowHandle,
        workspace_name: &str,
        workspace_index: usize,
    ) -> (u8, bool, String) {
        let appearance = &config.appearance;
        let resolution = self.executor.resolve_rules(
            desktop,
            id,
            Some(workspace_name.to_string()),
            Some(workspace_index),
            RuleResolutionScope::All,
        );
        let base_alpha = resolution.opacity.unwrap_or(appearance.window_opacity);
        let effective_alpha = desktop.opacity(id);
        (
            effective_alpha,
            effective_alpha < base_alpha,
            format!("{:?}", desktop.shadow_policy(id)),
        )
    }

    /// The state of every output with its workspaces, and how many windows
    /// they hold in all.
    pub fn monitor_data(
        &self,
        desktop: &dyn Desktop,
        config: &gevrek_model::settings::DesktopConfig,
    ) -> (Vec<MonitorData>, usize) {
        let focused_monitor = self
            .executor
            .focused_monitor()
            .or_else(|| {
                desktop
                    .active_window()
                    .and_then(|window| self.monitor_manager.find_window(window))
                    .map(|(monitor, _)| monitor)
            })
            .or_else(|| self.monitor_manager.get_primary_monitor_handle());
        let mut monitors = Vec::new();
        let mut total_windows = 0;
        for info in self.monitor_manager.get_all_monitors() {
            let Some(ring) = self.monitor_manager.get_workspaces(info.handle) else {
                continue;
            };
            if ring.is_empty() {
                continue;
            }
            let workspaces = ring
                .elements()
                .iter()
                .enumerate()
                .map(|(index, workspace)| self.workspace_data(desktop, config, workspace, index))
                .collect::<Vec<_>>();
            let Some(active_workspace) = workspaces.get(ring.focused_idx()).cloned() else {
                continue;
            };
            total_windows += workspaces
                .iter()
                .map(|workspace| workspace.total_windows)
                .sum::<usize>();
            let work_area = self
                .monitor_manager
                .get_adjusted_work_area(info.handle, desktop)
                .unwrap_or_else(|| info.work_area_rect());
            let reserved = self.monitor_manager.get_reserved_area(info.handle);
            monitors.push(MonitorData {
                handle: info.handle,
                name: info.name.clone(),
                connector: info.device_path.clone(),
                stable_id: info.stable_id.clone(),
                is_primary: info.is_primary,
                is_focused: focused_monitor == Some(info.handle),
                workspace: active_workspace,
                workspaces,
                x: info.x,
                y: info.y,
                width: info.width,
                height: info.height,
                work_x: work_area.x,
                work_y: work_area.y,
                work_width: work_area.width,
                work_height: work_area.height,
                reserved_left: reserved.left,
                reserved_top: reserved.top,
                reserved_right: reserved.right,
                reserved_bottom: reserved.bottom,
                zones_room: self.monitor_manager.shell_room_taken(info.handle, desktop),
                bar: self
                    .monitor_manager
                    .bar_rect(info.handle)
                    .map(|rect| (rect.x, rect.y, rect.width, rect.height)),
                scale: f64::from(info.dpi_scale),
                detail: self
                    .monitor_manager
                    .detail(info.handle)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        (monitors, total_windows)
    }

    /// The state of every configured scratchpad.
    pub fn scratchpad_data(&self) -> Vec<ScratchpadStateData> {
        self.executor
            .scratchpad_runtime_states()
            .into_iter()
            .map(|state| ScratchpadStateData {
                id: state.id,
                enabled: state.enabled,
                handle: state.handle,
                visible: state.visible,
                pending_spawn: state.pending_spawn,
            })
            .collect()
    }

    /// The state of one workspace and its windows, for `state`, the overlay
    /// and the editor's inspector.
    pub fn workspace_data(
        &self,
        desktop: &dyn Desktop,
        config: &gevrek_model::settings::DesktopConfig,
        workspace: &Workspace,
        workspace_index: usize,
    ) -> WorkspaceData {
        let tiled_windows = workspace
            .containers
            .iter()
            .map(|container| container.windows.len())
            .sum::<usize>();
        let floating_windows = workspace.floating_windows.len();
        let focused_window = workspace.focused_window();
        let known: HashMap<WindowHandle, gevrek_model::window::WindowInfo> = desktop
            .windows()
            .into_iter()
            .map(|window| (window.handle, window))
            .collect();
        let mut windows = Vec::new();
        let mut detailed_windows = Vec::new();

        let states = &self.executor.window_state;
        // (`place`: where among the others it is listed; at the end
        // without one.)
        let mut collect_window = |handle: WindowHandle, is_tiled: bool, place: Option<usize>| {
            let Some(window_info) = known.get(&handle) else {
                return;
            };
            let at = place.map_or(detailed_windows.len(), |place| {
                place.min(detailed_windows.len())
            });
            let state = states.get(handle);
            // Minimized as the panel tells it: by the person (or their
            // program) while the window could be seen. One the system
            // minimized while its workspace was put away is not: it comes
            // back with its workspace (`switch_workspace_on`).
            let is_minimized = state.minimized || (window_info.is_minimized && !state.hidden_by_wm);
            windows.insert(at, window_info.clone());
            let (effective_opacity, is_dimmed, shadow_policy) =
                self.window_effect_state(desktop, config, handle, &workspace.name, workspace_index);
            let detailed = DetailedWindowInfo {
                handle,
                title: window_info.title.clone(),
                class: desktop.class(handle),
                exe: desktop.exe(handle),
                app_id: desktop.app_id(handle),
                icon: desktop.icon(handle),
                x: window_info.x,
                y: window_info.y,
                width: window_info.width,
                height: window_info.height,
                is_focused: focused_window == Some(handle),
                is_minimized,
                is_maximized: window_info.is_maximized,
                is_managed: !desktop.ignored_by_rules(handle),
                is_tiled,
                is_floating: !is_tiled,
                is_always_on_top: desktop.is_always_on_top(handle),
                has_border: desktop.has_border(handle),
                is_border_window: desktop.is_decoration(handle),
                workspace_id: workspace.id,
                tags: self
                    .executor
                    .rule_metadata
                    .tags(handle)
                    .into_iter()
                    .collect(),
                matched_rules: self.executor.rule_metadata.matched_rules(handle),
                effective_rule_effects: self.executor.rule_metadata.effects(handle),
                effective_opacity,
                is_dimmed,
                shadow_policy,
            };
            detailed_windows.insert(at, detailed);
        };

        for container in workspace.containers.iter() {
            for handle in container.windows.iter().copied() {
                collect_window(handle, true, None);
            }
        }
        for handle in workspace.floating_windows.iter().copied() {
            collect_window(handle, false, None);
        }
        // A tiled window the person minimized left the layout, and is
        // still this workspace's: it is listed (so a panel can give it
        // back) where it stood among the others when it was minimized,
        // under the workspace it goes back to when it is restored. Only
        // here, once: workspace ids are not shared, and one that is a
        // member again is listed as one. (Those that stood first are put
        // back first, so each comes to its own place.)
        let outputs = &self.monitor_manager;
        let mut minimized: Vec<(usize, WindowHandle)> = states
            .iter()
            .filter(|(handle, state)| {
                state.minimized
                    && state.minimized_origin == Some(workspace.id)
                    && outputs.find_window(*handle).is_none()
            })
            .map(|(handle, state)| (state.minimized_place.unwrap_or(usize::MAX), handle))
            .collect();
        minimized.sort();
        for (place, handle) in minimized {
            collect_window(handle, true, Some(place));
        }

        WorkspaceData {
            id: workspace.id,
            name: workspace.name.clone(),
            layout_type: format!("{:?}", workspace.layout),
            total_windows: tiled_windows + floating_windows,
            tiled_windows,
            floating_windows,
            container_count: workspace.containers.len(),
            dwindle_direction: workspace.dwindle_direction.config_name().to_string(),
            dwindle_preserve_split: workspace.dwindle_preserve_split,
            pseudotiled_windows: workspace.pseudotile_sizes.len(),
            master_position: format!("{:?}", workspace.master_position),
            new_window_placement: format!("{:?}", workspace.new_window_placement),
            tiling_enabled: workspace.tiling_enabled,
            float_override: workspace.float_override,
            title: workspace.label().to_string(),
            configured: workspace.configured,
            persistent: workspace.persistent,
            window_container_behaviour: workspace
                .window_container_behaviour
                .display_name()
                .to_string(),
            windows,
            detailed_windows,
        }
    }
}
