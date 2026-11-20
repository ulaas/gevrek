//! The command table: what each `DispatchCommand` does to engine state
//! (docs/PHASE2_PLAN.md, step 4.9). Bindings, the CLI, gRPC and the
//! automation graph all send `DispatchCommand`s, so each command is decided
//! in one place.
//!
//! Ported so far: the commands that change one workspace's layout settings.
//! The caller picks the workspace (`layout_scope`), applies the command, and
//! lays the workspace out.

use crate::layouts::LayoutType;
use crate::workspace::Workspace;
use gevrek_model::command::DispatchCommand;
use gevrek_model::geometry::Padding;
use gevrek_model::settings::ReservedArea;

/// Which workspace a layout command acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutScope {
    /// The workspace of the active window (the command needs one, and it
    /// keeps focus).
    ActiveWindow,
    /// The visible workspace of the active output.
    FocusedWorkspace,
}

/// The scope of a layout-setting command; `None` for every other command.
pub fn layout_scope(command: &DispatchCommand) -> Option<LayoutScope> {
    use DispatchCommand::*;
    match command {
        SetLayout { .. }
        | SetLayoutRatio { .. }
        | AdjustLayoutRatio { .. }
        | SetMasterCount { .. }
        | AdjustMasterCount { .. }
        | RotateLayout
        | SetLayoutGap { .. }
        | AdjustLayoutGap { .. }
        | SetDwindleDirection { .. }
        | CycleDwindleDirection
        | ToggleDwindlePreserveSplit => Some(LayoutScope::ActiveWindow),
        CycleNextLayout
        | CyclePreviousLayout
        | FlipLayout { .. }
        | ToggleMonocle
        | ToggleLayoutLock
        | SetWorkspacePadding { .. }
        | AdjustWorkspacePadding { .. }
        | SetContainerPadding { .. }
        | AdjustContainerPadding { .. }
        | SetWorkspaceWorkAreaOffset { .. } => Some(LayoutScope::FocusedWorkspace),
        _ => None,
    }
}

/// Apply a layout-setting command to a workspace. Values are clamped to
/// their valid ranges. Returns `Ok(false)` when `command` is not a layout
/// command (nothing changed).
pub fn apply_layout_command(
    workspace: &mut Workspace,
    command: &DispatchCommand,
) -> Result<bool, String> {
    use DispatchCommand::*;
    match command {
        CycleNextLayout | CyclePreviousLayout => {
            let layouts = LayoutType::all();
            let current = layouts
                .iter()
                .position(|layout| *layout == workspace.layout)
                .unwrap_or(0);
            let step = if matches!(command, CycleNextLayout) {
                1
            } else {
                layouts.len() - 1
            };
            workspace.layout = layouts[(current + step) % layouts.len()];
            log::debug!(
                "Cycled to layout {:?} on workspace {}",
                workspace.layout,
                workspace.name
            );
        }
        SetLayout { layout } => workspace.layout = *layout,
        SetLayoutRatio { ratio } => workspace.master_ratio = ratio.clamp(0.1, 0.9),
        AdjustLayoutRatio { delta } => {
            workspace.master_ratio = (workspace.master_ratio + delta).clamp(0.1, 0.9)
        }
        SetMasterCount { count } => workspace.master_count = (*count).clamp(1, 8),
        AdjustMasterCount { delta } => {
            workspace.master_count = (workspace.master_count as i32).saturating_add(*delta).clamp(1, 8) as usize
        }
        RotateLayout => workspace.layout_rotation = (workspace.layout_rotation + 1) % 4,
        FlipLayout { axis } => {
            workspace.toggle_layout_flip(*axis);
            log::debug!(
                "Layout flip: horizontal {}, vertical {}",
                on_off(workspace.layout_flip_horizontal),
                on_off(workspace.layout_flip_vertical),
            );
        }
        ToggleMonocle => {
            let active = workspace.toggle_monocle()?;
            log::debug!(
                "Monocle {} for workspace '{}'",
                enabled(active),
                workspace.name
            );
        }
        ToggleLayoutLock => {
            workspace.layout_locked = !workspace.layout_locked;
            log::debug!(
                "Layout resize lock {} for workspace '{}'",
                enabled(workspace.layout_locked),
                workspace.name,
            );
        }
        SetLayoutGap { gap } => {
            workspace.container_padding = Padding::uniform((*gap).clamp(0, 500))
        }
        AdjustLayoutGap { delta } => {
            let gap = workspace.container_padding.left.saturating_add(*delta).clamp(0, 500);
            workspace.container_padding = Padding::uniform(gap);
        }
        SetWorkspacePadding { padding } => {
            workspace.padding = Padding::uniform((*padding).clamp(0, 500))
        }
        AdjustWorkspacePadding { delta } => {
            let padding = workspace.padding.left.saturating_add(*delta).clamp(0, 500);
            workspace.padding = Padding::uniform(padding);
        }
        SetContainerPadding { padding } => {
            workspace.container_padding = Padding::uniform((*padding).clamp(0, 500))
        }
        AdjustContainerPadding { delta } => {
            let padding = workspace.container_padding.left.saturating_add(*delta).clamp(0, 500);
            workspace.container_padding = Padding::uniform(padding);
        }
        SetWorkspaceWorkAreaOffset {
            left,
            top,
            right,
            bottom,
        } => {
            workspace.work_area_offset = ReservedArea {
                left: (*left).clamp(0, 10_000),
                top: (*top).clamp(0, 10_000),
                right: (*right).clamp(0, 10_000),
                bottom: (*bottom).clamp(0, 10_000),
            };
            log::debug!(
                "Work-area offset for workspace '{}' set to {},{},{},{}",
                workspace.name,
                workspace.work_area_offset.left,
                workspace.work_area_offset.top,
                workspace.work_area_offset.right,
                workspace.work_area_offset.bottom,
            );
        }
        SetDwindleDirection { direction } => {
            workspace.dwindle_direction = *direction;
            forget_splits_unless_preserved(workspace);
        }
        CycleDwindleDirection => {
            let direction = workspace.cycle_dwindle_direction();
            forget_splits_unless_preserved(workspace);
            log::debug!("Dwindle split policy is now {}", direction.display_name());
        }
        ToggleDwindlePreserveSplit => {
            workspace.dwindle_preserve_split = !workspace.dwindle_preserve_split;
            forget_splits_unless_preserved(workspace);
            log::debug!(
                "Dwindle preserve split {}",
                enabled(workspace.dwindle_preserve_split)
            );
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn forget_splits_unless_preserved(workspace: &mut Workspace) {
    if !workspace.dwindle_preserve_split {
        workspace.dwindle_split_history.clear();
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn enabled(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> Workspace {
        Workspace::new(0, "one".into())
    }

    fn apply(workspace: &mut Workspace, command: DispatchCommand) {
        assert_eq!(
            apply_layout_command(workspace, &command),
            Ok(true),
            "{command:?}"
        );
    }

    #[test]
    fn ratios_counts_and_gaps_are_clamped() {
        let mut ws = workspace();
        apply(&mut ws, DispatchCommand::SetLayoutRatio { ratio: 5.0 });
        assert_eq!(ws.master_ratio, 0.9);
        apply(&mut ws, DispatchCommand::AdjustLayoutRatio { delta: -2.0 });
        assert_eq!(ws.master_ratio, 0.1);
        apply(&mut ws, DispatchCommand::SetMasterCount { count: 99 });
        assert_eq!(ws.master_count, 8);
        apply(&mut ws, DispatchCommand::AdjustMasterCount { delta: -20 });
        assert_eq!(ws.master_count, 1);
        apply(&mut ws, DispatchCommand::SetLayoutGap { gap: 9999 });
        assert_eq!(ws.container_padding, Padding::uniform(500));
        apply(
            &mut ws,
            DispatchCommand::AdjustWorkspacePadding { delta: -9999 },
        );
        assert_eq!(ws.padding, Padding::uniform(0));
    }

    #[test]
    fn layout_rotation_wraps_and_the_layout_is_set() {
        let mut ws = workspace();
        for _ in 0..5 {
            apply(&mut ws, DispatchCommand::RotateLayout);
        }
        assert_eq!(ws.layout_rotation, 1);
        apply(
            &mut ws,
            DispatchCommand::SetLayout {
                layout: LayoutType::Columns,
            },
        );
        assert_eq!(ws.layout, LayoutType::Columns);
        apply(&mut ws, DispatchCommand::ToggleLayoutLock);
        assert!(ws.layout_locked);
    }

    #[test]
    fn changing_the_dwindle_policy_forgets_splits_unless_they_are_preserved() {
        let mut ws = workspace();
        ws.dwindle_split_history = vec![
            gevrek_model::layout::DwindleDirection::Horizontal,
            gevrek_model::layout::DwindleDirection::Vertical,
        ];
        apply(&mut ws, DispatchCommand::ToggleDwindlePreserveSplit);
        assert!(ws.dwindle_preserve_split);
        apply(&mut ws, DispatchCommand::CycleDwindleDirection);
        assert_eq!(ws.dwindle_split_history.len(), 2, "preserved");
        apply(&mut ws, DispatchCommand::ToggleDwindlePreserveSplit);
        assert!(ws.dwindle_split_history.is_empty());
    }

    #[test]
    fn cycling_walks_the_layout_list_both_ways_and_wraps() {
        let mut ws = workspace();
        let layouts = LayoutType::all();
        assert_eq!(ws.layout, layouts[0]);
        apply(&mut ws, DispatchCommand::CyclePreviousLayout);
        assert_eq!(ws.layout, *layouts.last().unwrap());
        apply(&mut ws, DispatchCommand::CycleNextLayout);
        apply(&mut ws, DispatchCommand::CycleNextLayout);
        assert_eq!(ws.layout, layouts[1]);
    }

    #[test]
    fn other_commands_are_left_to_their_own_handlers() {
        let mut ws = workspace();
        assert_eq!(
            apply_layout_command(&mut ws, &DispatchCommand::StackAll),
            Ok(false)
        );
        assert_eq!(layout_scope(&DispatchCommand::StackAll), None);
        assert_eq!(
            layout_scope(&DispatchCommand::ToggleMonocle),
            Some(LayoutScope::FocusedWorkspace)
        );
        assert_eq!(
            layout_scope(&DispatchCommand::RotateLayout),
            Some(LayoutScope::ActiveWindow)
        );
    }
}
