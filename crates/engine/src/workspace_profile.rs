//! Applying a workspace profile from the configuration to a live workspace.

use gevrek_model::settings::{AppearanceProfile, WorkspaceProfile};

/// Apply a workspace profile to a live workspace.
pub trait WorkspaceProfileApply {
    fn apply_to_workspace(
        &self,
        workspace: &mut crate::workspace::Workspace,
        appearance: &AppearanceProfile,
    );
}

impl WorkspaceProfileApply for WorkspaceProfile {
    /// Apply the complete profile to an already-created runtime workspace.
    ///
    /// Keeping this mapping in one place ensures eagerly-created persistent
    /// workspaces and lazily-created non-persistent workspaces behave exactly
    /// the same.
    fn apply_to_workspace(
        &self,
        workspace: &mut crate::workspace::Workspace,
        appearance: &AppearanceProfile,
    ) {
        workspace.also_called = self.also_called.clone();
        workspace.layout = self.layout;
        workspace.padding = crate::container::Padding::uniform(self.gaps_out);
        workspace.container_padding = crate::container::Padding::uniform(self.gaps_in);
        workspace.work_area_offset = self.work_area_offset;
        workspace.master_ratio = self.master_ratio;
        workspace.master_count = self.master_count;
        workspace.master_position = self.master_position;
        workspace.new_window_placement = self.new_window_placement;
        workspace.window_container_behaviour = self.window_container_behaviour;
        workspace.dwindle_direction = self.dwindle_direction;
        workspace.dwindle_preserve_split = self.dwindle_preserve_split;
        // (Floating windows: the configuration's look for them, on every
        // workspace.)
        workspace.floating_look = appearance.floating.clone();
        workspace.border_enabled = if self.appearance_override {
            self.border_enabled
        } else {
            appearance.border_enabled
        };
        workspace.border_width = if self.appearance_override {
            self.border_width
        } else {
            appearance.border_width
        };
        workspace.border_offset = if self.appearance_override {
            self.border_offset
        } else {
            appearance.border_offset
        };
        workspace.border_style = if self.appearance_override {
            self.border_style
        } else {
            appearance.border_style
        };
        workspace.active_border_color = if self.appearance_override {
            self.active_border_color
        } else {
            appearance.active_border_color
        };
        workspace.inactive_border_color = if self.appearance_override {
            self.inactive_border_color
        } else {
            appearance.inactive_border_color
        };
        workspace.active_border_alpha = if self.appearance_override {
            self.active_border_alpha
        } else {
            appearance.active_border_alpha
        };
        workspace.inactive_border_alpha = if self.appearance_override {
            self.inactive_border_alpha
        } else {
            appearance.inactive_border_alpha
        };
        // The stackbar is a global appearance feature; there
        // is no per-workspace appearance override for it.
        workspace.stackbar = appearance.stackbar.clone();
        // Invisible borders are likewise a global appearance feature.
        workspace.invisible_borders = appearance.invisible_borders;
    }
}
