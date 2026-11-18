// Conversion utilities between gevrek-core types and protobuf types
use crate::proto;
use gevrek_model::graph::{self as node_editor, NodeGraph, NodeType};

pub fn dispatch_command_to_proto(
    command: gevrek_model::command::DispatchCommand,
) -> proto::DispatchRequest {
    use gevrek_model::command::{Direction, DispatchCommand};

    let mut request = proto::DispatchRequest {
        command: proto::DispatchType::Unspecified as i32,
        direction: proto::Direction::Unspecified as i32,
        x: 0,
        y: 0,
        argument: String::new(),
        secondary_argument: String::new(),
    };
    let proto_direction = |direction| match direction {
        Direction::Left => proto::Direction::Left,
        Direction::Right => proto::Direction::Right,
        Direction::Up => proto::Direction::Up,
        Direction::Down => proto::Direction::Down,
    };

    match command {
        // Stays "unspecified": the server refuses it.
        DispatchCommand::Unknown(_) => {}
        // Commands newer than the typed request have no number in it: they
        // are sent as JSON (`WMClient::dispatch_command`).
        DispatchCommand::Quit
        | DispatchCommand::KeyboardLayout { .. }
        | DispatchCommand::ScrollView { .. }
        | DispatchCommand::ExpectWindow { .. }
        | DispatchCommand::WorkspaceOverview
        | DispatchCommand::PickWindow
        | DispatchCommand::NightLightPreview { .. }
        | DispatchCommand::ShortcutsInhibit
        | DispatchCommand::OutputTrial { .. }
        | DispatchCommand::Shell { .. }
        | DispatchCommand::Notify { .. }
        | DispatchCommand::AppShortcut { .. }
        | DispatchCommand::Osd { .. } => {}
        DispatchCommand::FocusNext => request.command = proto::DispatchType::FocusNext as i32,
        DispatchCommand::FocusPrevious => {
            request.command = proto::DispatchType::FocusPrevious as i32
        }
        DispatchCommand::FocusLast => request.command = proto::DispatchType::FocusLast as i32,
        DispatchCommand::FocusWindow { selector } => {
            request.command = proto::DispatchType::FocusWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ForceFocusWindow { selector } => {
            request.command = proto::DispatchType::ForceFocusWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::FocusDirection { direction } => {
            request.command = proto::DispatchType::FocusDirection as i32;
            request.direction = proto_direction(direction) as i32;
        }
        DispatchCommand::SwapNext => request.command = proto::DispatchType::SwapNext as i32,
        DispatchCommand::SwapPrevious => request.command = proto::DispatchType::SwapPrevious as i32,
        DispatchCommand::SwapDirection { direction } => {
            request.command = proto::DispatchType::SwapDirection as i32;
            request.direction = proto_direction(direction) as i32;
        }
        DispatchCommand::MoveActive { delta_x, delta_y } => {
            request.command = proto::DispatchType::MoveActive as i32;
            request.x = delta_x;
            request.y = delta_y;
        }
        DispatchCommand::MoveWindow {
            selector,
            x,
            y,
            relative,
        } => {
            request.command = proto::DispatchType::MoveWindow as i32;
            request.argument = selector.expression();
            request.x = x;
            request.y = y;
            request.secondary_argument = if relative { "relative" } else { "absolute" }.to_string();
        }
        DispatchCommand::ResizeActive {
            delta_width,
            delta_height,
        } => {
            request.command = proto::DispatchType::ResizeActive as i32;
            request.x = delta_width;
            request.y = delta_height;
        }
        DispatchCommand::ResizeWindow {
            selector,
            width,
            height,
            relative,
        } => {
            request.command = proto::DispatchType::ResizeWindow as i32;
            request.argument = selector.expression();
            request.x = width;
            request.y = height;
            request.secondary_argument = if relative { "relative" } else { "absolute" }.to_string();
        }
        DispatchCommand::CloseActive => request.command = proto::DispatchType::CloseActive as i32,
        DispatchCommand::CloseWindow { selector } => {
            request.command = proto::DispatchType::CloseWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::MinimizeActive => {
            request.command = proto::DispatchType::MinimizeActive as i32
        }
        DispatchCommand::ToggleMaximized => {
            request.command = proto::DispatchType::ToggleMaximized as i32
        }
        DispatchCommand::ToggleFullscreen => {
            request.command = proto::DispatchType::ToggleFullscreen as i32
        }
        DispatchCommand::ToggleWindowFullscreen { selector } => {
            request.command = proto::DispatchType::ToggleWindowFullscreen as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::TogglePinned => request.command = proto::DispatchType::TogglePinned as i32,
        DispatchCommand::ToggleWindowPinned { selector } => {
            request.command = proto::DispatchType::ToggleWindowPinned as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ToggleFloating => {
            request.command = proto::DispatchType::ToggleFloating as i32
        }
        DispatchCommand::ToggleWindowFloating { selector } => {
            request.command = proto::DispatchType::ToggleWindowFloating as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::SetPinned { enabled } => {
            request.command = proto::DispatchType::SetPinned as i32;
            request.x = i32::from(enabled);
        }
        DispatchCommand::SetWindowPinned { selector, enabled } => {
            request.command = proto::DispatchType::SetWindowPinned as i32;
            request.argument = selector.expression();
            request.x = i32::from(enabled);
        }
        DispatchCommand::SetFloating { enabled } => {
            request.command = proto::DispatchType::SetFloating as i32;
            request.x = i32::from(enabled);
        }
        DispatchCommand::SetWindowFloating { selector, enabled } => {
            request.command = proto::DispatchType::SetWindowFloating as i32;
            request.argument = selector.expression();
            request.x = i32::from(enabled);
        }
        DispatchCommand::CenterActive => request.command = proto::DispatchType::CenterActive as i32,
        DispatchCommand::ToggleFloatingAside => {
            request.command = proto::DispatchType::ToggleFloatingAside as i32
        }
        DispatchCommand::CenterWindow { selector } => {
            request.command = proto::DispatchType::CenterWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::RaiseActive => request.command = proto::DispatchType::RaiseActive as i32,
        DispatchCommand::RaiseWindow { selector } => {
            request.command = proto::DispatchType::RaiseWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::LowerActive => request.command = proto::DispatchType::LowerActive as i32,
        DispatchCommand::LowerWindow { selector } => {
            request.command = proto::DispatchType::LowerWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ClearWindowTags => {
            request.command = proto::DispatchType::ClearWindowTags as i32
        }
        DispatchCommand::ClearSelectedWindowTags { selector } => {
            request.command = proto::DispatchType::ClearSelectedWindowTags as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::AddWindowTag { tag } => {
            request.command = proto::DispatchType::AddWindowTag as i32;
            request.argument = tag;
        }
        DispatchCommand::AddSelectedWindowTag { selector, tag } => {
            request.command = proto::DispatchType::AddSelectedWindowTag as i32;
            request.argument = selector.expression();
            request.secondary_argument = tag;
        }
        DispatchCommand::RemoveWindowTag { tag } => {
            request.command = proto::DispatchType::RemoveWindowTag as i32;
            request.argument = tag;
        }
        DispatchCommand::RemoveSelectedWindowTag { selector, tag } => {
            request.command = proto::DispatchType::RemoveSelectedWindowTag as i32;
            request.argument = selector.expression();
            request.secondary_argument = tag;
        }
        DispatchCommand::ToggleWindowTag { tag } => {
            request.command = proto::DispatchType::ToggleWindowTag as i32;
            request.argument = tag;
        }
        DispatchCommand::ToggleSelectedWindowTag { selector, tag } => {
            request.command = proto::DispatchType::ToggleSelectedWindowTag as i32;
            request.argument = selector.expression();
            request.secondary_argument = tag;
        }
        DispatchCommand::EnableRule { name } => {
            request.command = proto::DispatchType::EnableRule as i32;
            request.argument = name;
        }
        DispatchCommand::DisableRule { name } => {
            request.command = proto::DispatchType::DisableRule as i32;
            request.argument = name;
        }
        DispatchCommand::ToggleRule { name } => {
            request.command = proto::DispatchType::ToggleRule as i32;
            request.argument = name;
        }
        DispatchCommand::SwitchWorkspaceSelector { selector } => {
            request.command = proto::DispatchType::SwitchWorkspace as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::MoveToWorkspaceSelector { selector } => {
            request.command = proto::DispatchType::MoveToWorkspace as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::MoveToWorkspaceAndFollow { selector } => {
            request.command = proto::DispatchType::MoveToWorkspaceAndFollow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::MoveWindowToWorkspace { window, workspace } => {
            request.command = proto::DispatchType::MoveSelectedToWorkspace as i32;
            request.argument = window.expression();
            request.secondary_argument = workspace.expression();
        }
        DispatchCommand::MoveWindowToWorkspaceAndFollow { window, workspace } => {
            request.command = proto::DispatchType::MoveSelectedToWorkspaceAndFollow as i32;
            request.argument = window.expression();
            request.secondary_argument = workspace.expression();
        }
        DispatchCommand::RenameWorkspace { selector, name } => {
            request.command = proto::DispatchType::RenameWorkspace as i32;
            request.argument = selector.expression();
            request.secondary_argument = name;
        }
        DispatchCommand::FocusMonitor { target } => {
            request.command = proto::DispatchType::FocusMonitor as i32;
            request.argument = target.expression();
        }
        DispatchCommand::MoveWorkspaceToMonitor { target } => {
            request.command = proto::DispatchType::MoveWorkspaceToMonitor as i32;
            request.argument = target.expression();
        }
        DispatchCommand::SwapActiveWorkspaces { target } => {
            request.command = proto::DispatchType::SwapActiveWorkspaces as i32;
            request.argument = target.expression();
        }
        DispatchCommand::MoveWindowToMonitor { window, target } => {
            request.command = proto::DispatchType::MoveWindowToMonitor as i32;
            request.argument = window.expression();
            request.secondary_argument = target.expression();
        }
        DispatchCommand::ToggleWindowCentered { window, target } => {
            request.command = proto::DispatchType::ToggleWindowCentered as i32;
            request.argument = window.expression();
            request.secondary_argument = target.expression();
        }
        DispatchCommand::MoveWindowToMonitorAndFollow { window, target } => {
            request.command = proto::DispatchType::MoveWindowToMonitorAndFollow as i32;
            request.argument = window.expression();
            request.secondary_argument = target.expression();
        }
        DispatchCommand::Retile => request.command = proto::DispatchType::Retile as i32,
        DispatchCommand::ToggleTiling => request.command = proto::DispatchType::ToggleTiling as i32,
        DispatchCommand::ToggleWorkspaceTiling { selector } => {
            request.command = proto::DispatchType::ToggleWorkspaceTiling as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ToggleWorkspaceFloatOverride { selector } => {
            request.command = proto::DispatchType::ToggleWorkspaceFloatOverride as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::NewWorkspace { name } => {
            request.command = proto::DispatchType::NewWorkspace as i32;
            if let Some(name) = name {
                request.argument = name;
            }
        }
        DispatchCommand::CloseWorkspace => {
            request.command = proto::DispatchType::CloseWorkspace as i32
        }
        DispatchCommand::EnsureWorkspaces { target, count } => {
            request.command = proto::DispatchType::EnsureWorkspaces as i32;
            request.argument = target.expression();
            request.x = count as i32;
        }
        // (The layout goes with the command as JSON only.)
        DispatchCommand::EnsureNamedWorkspace { target, name, .. } => {
            request.command = proto::DispatchType::EnsureNamedWorkspace as i32;
            request.argument = target.expression();
            request.secondary_argument = name;
        }
        DispatchCommand::ToggleWindowContainerBehaviour => {
            request.command = proto::DispatchType::ToggleWindowContainerBehaviour as i32
        }
        DispatchCommand::ToggleWorkspaceWindowContainerBehaviour { selector } => {
            request.command = proto::DispatchType::ToggleWorkspaceWindowContainerBehaviour as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ToggleTitleBar => {
            request.command = proto::DispatchType::ToggleTitleBar as i32
        }
        DispatchCommand::ToggleWindowTitleBar { selector } => {
            request.command = proto::DispatchType::ToggleWindowTitleBar as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::FocusMonitorAtCursor => {
            request.command = proto::DispatchType::FocusMonitorAtCursor as i32
        }
        DispatchCommand::FocusMonitorWorkspace { target, workspace } => {
            request.command = proto::DispatchType::FocusMonitorWorkspace as i32;
            request.argument = target.expression();
            request.secondary_argument = workspace.expression();
        }
        DispatchCommand::ToggleMouseFollowsFocus => {
            request.command = proto::DispatchType::ToggleMouseFollowsFocus as i32
        }
        DispatchCommand::ManageWindow { selector } => {
            request.command = proto::DispatchType::ManageWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::UnmanageWindow { selector } => {
            request.command = proto::DispatchType::UnmanageWindow as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::RestoreWindows => {
            request.command = proto::DispatchType::RestoreWindows as i32
        }
        DispatchCommand::ResizeEdge { edge, sizing } => {
            request.command = proto::DispatchType::ResizeEdge as i32;
            request.direction = proto_direction(edge) as i32;
            request.argument = sizing.config_name().to_string();
        }
        DispatchCommand::ResizeAxis { axis, sizing } => {
            request.command = proto::DispatchType::ResizeAxis as i32;
            request.argument = axis.config_name().to_string();
            request.secondary_argument = sizing.config_name().to_string();
        }
        DispatchCommand::SetResizeDelta { pixels } => {
            request.command = proto::DispatchType::SetResizeDelta as i32;
            request.x = pixels;
        }
        DispatchCommand::FlipLayout { axis } => {
            request.command = proto::DispatchType::FlipLayout as i32;
            request.argument = axis.config_name().to_string();
        }
        DispatchCommand::ToggleMonocle => {
            request.command = proto::DispatchType::ToggleMonocle as i32
        }
        DispatchCommand::ToggleLayoutLock => {
            request.command = proto::DispatchType::ToggleLayoutLock as i32
        }
        DispatchCommand::ToggleStackbar => {
            request.command = proto::DispatchType::ToggleStackbar as i32
        }
        DispatchCommand::SaveResize { path } => {
            request.command = proto::DispatchType::SaveResize as i32;
            request.argument = path;
        }
        DispatchCommand::LoadResize { path } => {
            request.command = proto::DispatchType::LoadResize as i32;
            request.argument = path;
        }
        DispatchCommand::QuickSaveResize => {
            request.command = proto::DispatchType::QuickSaveResize as i32
        }
        DispatchCommand::QuickLoadResize => {
            request.command = proto::DispatchType::QuickLoadResize as i32
        }
        DispatchCommand::WatchConfiguration { enabled } => {
            request.command = proto::DispatchType::WatchConfiguration as i32;
            request.x = i32::from(enabled);
        }
        DispatchCommand::ToggleTransparency => {
            request.command = proto::DispatchType::ToggleTransparency as i32
        }
        DispatchCommand::SetTransparencyAlpha { alpha } => {
            request.command = proto::DispatchType::SetTransparencyAlpha as i32;
            request.x = i32::from(alpha);
        }
        DispatchCommand::SetGlobalWorkAreaOffset {
            left,
            top,
            right,
            bottom,
        } => {
            request.command = proto::DispatchType::SetGlobalWorkAreaOffset as i32;
            request.argument = format!("{left},{top},{right},{bottom}");
        }
        DispatchCommand::SetWorkspaceWorkAreaOffset {
            left,
            top,
            right,
            bottom,
        } => {
            request.command = proto::DispatchType::SetWorkspaceWorkAreaOffset as i32;
            request.argument = format!("{left},{top},{right},{bottom}");
        }
        DispatchCommand::SetWorkspacePadding { padding } => {
            request.command = proto::DispatchType::SetWorkspacePadding as i32;
            request.x = padding;
        }
        DispatchCommand::AdjustWorkspacePadding { delta } => {
            request.command = proto::DispatchType::AdjustWorkspacePadding as i32;
            request.x = delta;
        }
        DispatchCommand::SetContainerPadding { padding } => {
            request.command = proto::DispatchType::SetContainerPadding as i32;
            request.x = padding;
        }
        DispatchCommand::AdjustContainerPadding { delta } => {
            request.command = proto::DispatchType::AdjustContainerPadding as i32;
            request.x = delta;
        }
        DispatchCommand::StackAll => request.command = proto::DispatchType::StackAll as i32,
        DispatchCommand::UnstackAll => request.command = proto::DispatchType::UnstackAll as i32,
        DispatchCommand::Promote => request.command = proto::DispatchType::Promote as i32,
        DispatchCommand::PromoteFocus => request.command = proto::DispatchType::PromoteFocus as i32,
        DispatchCommand::PromoteWindow { direction } => {
            request.command = proto::DispatchType::PromoteWindow as i32;
            request.direction = proto_direction(direction) as i32;
        }
        DispatchCommand::CycleNextLayout => {
            request.command = proto::DispatchType::CycleNextLayout as i32
        }
        DispatchCommand::CyclePreviousLayout => {
            request.command = proto::DispatchType::CyclePreviousLayout as i32
        }
        DispatchCommand::SetLayout { layout } => {
            request.command = proto::DispatchType::SetLayout as i32;
            request.argument = layout.config_name().to_string();
        }
        DispatchCommand::SetLayoutRatio { ratio } => {
            request.command = proto::DispatchType::SetLayoutRatio as i32;
            request.x = (ratio * 1000.0).round() as i32;
        }
        DispatchCommand::AdjustLayoutRatio { delta } => {
            request.command = proto::DispatchType::AdjustLayoutRatio as i32;
            request.x = (delta * 1000.0).round() as i32;
        }
        DispatchCommand::SetMasterCount { count } => {
            request.command = proto::DispatchType::SetMasterCount as i32;
            request.x = count as i32;
        }
        DispatchCommand::AdjustMasterCount { delta } => {
            request.command = proto::DispatchType::AdjustMasterCount as i32;
            request.x = delta;
        }
        DispatchCommand::RotateLayout => request.command = proto::DispatchType::RotateLayout as i32,
        DispatchCommand::SetLayoutGap { gap } => {
            request.command = proto::DispatchType::SetLayoutGap as i32;
            request.x = gap;
        }
        DispatchCommand::AdjustLayoutGap { delta } => {
            request.command = proto::DispatchType::AdjustLayoutGap as i32;
            request.x = delta;
        }
        DispatchCommand::PromoteToMaster => {
            request.command = proto::DispatchType::PromoteToMaster as i32
        }
        DispatchCommand::SetDwindleDirection { direction } => {
            request.command = proto::DispatchType::SetDwindleDirection as i32;
            request.argument = direction.config_name().to_string();
        }
        DispatchCommand::CycleDwindleDirection => {
            request.command = proto::DispatchType::CycleDwindleDirection as i32
        }
        DispatchCommand::ToggleDwindlePreserveSplit => {
            request.command = proto::DispatchType::ToggleDwindlePreserveSplit as i32
        }
        DispatchCommand::TogglePseudotile => {
            request.command = proto::DispatchType::TogglePseudotile as i32
        }
        DispatchCommand::ToggleWindowPseudotile { selector } => {
            request.command = proto::DispatchType::ToggleWindowPseudotile as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ToggleGroup => request.command = proto::DispatchType::ToggleGroup as i32,
        DispatchCommand::ToggleWindowGroup { selector } => {
            request.command = proto::DispatchType::ToggleWindowGroup as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::ToggleGroupLock => {
            request.command = proto::DispatchType::ToggleGroupLock as i32
        }
        DispatchCommand::ToggleWindowGroupLock { selector } => {
            request.command = proto::DispatchType::ToggleWindowGroupLock as i32;
            request.argument = selector.expression();
        }
        DispatchCommand::CycleGroupNext => {
            request.command = proto::DispatchType::CycleGroupNext as i32
        }
        DispatchCommand::CycleGroupPrevious => {
            request.command = proto::DispatchType::CycleGroupPrevious as i32
        }
        DispatchCommand::CycleWindowGroup { selector, forward } => {
            request.command = proto::DispatchType::CycleWindowGroup as i32;
            request.argument = selector.expression();
            request.x = i32::from(forward);
        }
        DispatchCommand::FocusGroupIndex { index } => {
            request.command = proto::DispatchType::FocusGroupIndex as i32;
            request.x = index as i32;
        }
        DispatchCommand::FocusWindowGroupIndex { selector, index } => {
            request.command = proto::DispatchType::FocusWindowGroupIndex as i32;
            request.argument = selector.expression();
            request.x = index as i32;
        }
        DispatchCommand::MoveGroupWindow { forward } => {
            request.command = proto::DispatchType::MoveGroupWindow as i32;
            request.x = i32::from(forward);
        }
        DispatchCommand::MoveWindowInGroup { selector, forward } => {
            request.command = proto::DispatchType::MoveWindowInGroup as i32;
            request.argument = selector.expression();
            request.x = i32::from(forward);
        }
        DispatchCommand::MergeGroupNext => {
            request.command = proto::DispatchType::MergeGroupNext as i32
        }
        DispatchCommand::MergeGroupPrevious => {
            request.command = proto::DispatchType::MergeGroupPrevious as i32
        }
        DispatchCommand::UngroupActive => {
            request.command = proto::DispatchType::UngroupActive as i32
        }
        DispatchCommand::ToggleExpose => request.command = proto::DispatchType::ToggleExpose as i32,
        DispatchCommand::ShowShortcuts => {
            request.command = proto::DispatchType::ShowShortcuts as i32
        }
        DispatchCommand::ToggleScratchpad { id } => {
            request.command = proto::DispatchType::ToggleScratchpad as i32;
            request.argument = id;
        }
        DispatchCommand::Exec { program, args } => {
            request.command = proto::DispatchType::Exec as i32;
            request.argument = program;
            if !args.is_empty() {
                request.secondary_argument = serde_json::to_string(&args).unwrap_or_default();
            }
        }
        DispatchCommand::LaunchStartup { id } => {
            request.command = proto::DispatchType::LaunchStartup as i32;
            request.argument = id;
        }
        DispatchCommand::StopStartup { id } => {
            request.command = proto::DispatchType::StopStartup as i32;
            request.argument = id;
        }
        DispatchCommand::RestartStartup { id } => {
            request.command = proto::DispatchType::RestartStartup as i32;
            request.argument = id;
        }
        DispatchCommand::SnapActiveToZone { index } => {
            request.command = proto::DispatchType::SnapActiveToZone as i32;
            request.x = index as i32;
        }
        DispatchCommand::CycleActiveZoneNext => {
            request.command = proto::DispatchType::CycleActiveZoneNext as i32
        }
        DispatchCommand::CycleActiveZonePrevious => {
            request.command = proto::DispatchType::CycleActiveZonePrevious as i32
        }
        DispatchCommand::SetBindingSubmap { name } => {
            request.command = proto::DispatchType::SetBindingSubmap as i32;
            request.argument = name;
        }
        DispatchCommand::ResetBindingSubmap => {
            request.command = proto::DispatchType::ResetBindingSubmap as i32
        }
    }
    request
}

// ============================================================================
// Core → Proto Conversions
// ============================================================================

/// The graph for the wire: its serde JSON, as in the configuration file.
pub fn node_graph_to_proto(core_graph: &NodeGraph) -> proto::NodeGraph {
    let (next_node_id, next_pin_id) = core_graph.id_counters();
    proto::NodeGraph {
        json: serde_json::to_string(core_graph).unwrap_or_default(),
        node_count: core_graph.nodes.len() as u32,
        connection_count: core_graph.connections.len() as u32,
        version: env!("CARGO_PKG_VERSION").to_string(),
        editor_id: String::new(), // Will be set by the caller
        next_node_id: next_node_id as u64,
        next_pin_id: next_pin_id as u64,
        ..Default::default()
    }
}

pub fn node_type_to_proto(node_type: &NodeType) -> proto::NodeType {
    use node_editor::NodeType as NT;
    use proto::NodeType as PT;

    match node_type {
        NT::Unknown(_) => PT::Unspecified,
        NT::OnStartup => PT::OnStartup,
        NT::OnStartupCompleted => PT::OnStartupCompleted,
        NT::OnWindowCreated => PT::OnWindowCreated,
        NT::OnWindowDestroyed => PT::OnWindowDestroyed,
        NT::OnWindowFocused => PT::OnWindowFocused,
        NT::OnWindowMoved => PT::OnWindowMoved,
        NT::OnWindowResized => PT::OnWindowResized,
        NT::OnKeyPress { .. } => PT::OnKeyPress,
        NT::ManualTrigger => PT::ManualTrigger,
        NT::LaunchProcess { .. } => PT::LaunchProcess,
        NT::MoveWindow { .. } => PT::MoveWindow,
        NT::ResizeWindow { .. } => PT::ResizeWindow,
        NT::CloseWindow => PT::CloseWindow,
        NT::SetWindowTitle { .. } => PT::SetWindowTitle,
        NT::AnimateWindow { .. } => PT::AnimateWindow,
        NT::SetWindowArrangement { .. } => PT::SetWindowArrangement,
        NT::BringToFront => PT::BringToFront,
        NT::SendToBack => PT::SendToBack,
        NT::SetAlwaysOnTop { .. } => PT::SetAlwaysOnTop,
        NT::WindowExists => PT::WindowExists,
        NT::WindowHasTitle { .. } => PT::WindowHasTitle,
        NT::MatchWindow { .. } => PT::MatchWindow,
        NT::GetWindowPosition => PT::GetWindowPosition,
        NT::GetWindowSize => PT::GetWindowSize,
        NT::GetActiveWindow => PT::GetActiveWindow,
        NT::GetAllWindows => PT::GetAllWindows,
        NT::GetWindowInfo => PT::GetWindowInfo,
        NT::GetMonitorForWindow => PT::GetMonitorForWindow,
        NT::Add => PT::Add,
        NT::Subtract => PT::Subtract,
        NT::Multiply => PT::Multiply,
        NT::Divide => PT::Divide,
        NT::NumberValue { .. } => PT::NumberValue,
        NT::StringValue { .. } => PT::StringValue,
        NT::BoolValue { .. } => PT::BoolValue,
        NT::ColorValue { .. } => PT::ColorValue,
        NT::BorderStyleValue { .. } => PT::BorderStyleValue,
        NT::ForEach => PT::ForEach,
        NT::WindowListToSingle { .. } => PT::WindowListToSingle,
        NT::SwitchToWorkspace { .. } => PT::SwitchToWorkspace,
        NT::MoveWindowToWorkspace { .. } => PT::MoveWindowToWorkspace,
        NT::GetCurrentWorkspace => PT::GetCurrentWorkspace,
        NT::NextWorkspace => PT::NextWorkspace,
        NT::PreviousWorkspace => PT::PreviousWorkspace,
        NT::FocusWorkspace { .. } => PT::FocusWorkspace,
        NT::GetMonitors => PT::GetMonitors,
        NT::GetMonitorInfo => PT::GetMonitorInfo,
        NT::CreateWorkspace { .. } => PT::CreateWorkspace,
        NT::CreateFloatRule { .. } => PT::CreateFloatRule,
        NT::CreateIgnoreRule { .. } => PT::CreateIgnoreRule,
        NT::CreateManageRule { .. } => PT::CreateManageRule,
        NT::WindowModeSelector { .. } => PT::WindowModeSelector,
        NT::ApplySystemFilters { .. } => PT::ApplySystemFilters,
        NT::CreateRule { .. } => PT::CreateRule,
        NT::CombineRules => PT::CombineRules,
        NT::LogMessage { .. } => PT::LogMessage,
        // Newer core-only variants are carried losslessly in NodeData.core_type
        // until the wire enum is expanded.
        NT::ForLoop { .. }
        | NT::GetWorkspace { .. }
        | NT::Retile { .. }
        | NT::SetLayoutRatio { .. }
        | NT::IncrementLayoutRatio { .. }
        | NT::DecrementLayoutRatio { .. }
        | NT::SetMasterCount { .. }
        | NT::IncrementMasterCount
        | NT::DecrementMasterCount
        | NT::SetLayoutGap { .. }
        | NT::CycleNextLayout
        | NT::CyclePreviousLayout
        | NT::RotateLayout
        | NT::ToString
        | NT::FromString => PT::Unspecified,
        // Window navigation nodes
        NT::FocusNextWindow => PT::FocusNextWindow,
        NT::FocusPreviousWindow => PT::FocusPreviousWindow,
        NT::CycleFocusNext | NT::CycleFocusPrevious => PT::Unspecified,
        NT::SwapWithNext => PT::SwapWithNext,
        NT::SwapWithPrevious => PT::SwapWithPrevious,
        NT::MoveToNextWorkspace => PT::MoveToNextWorkspace,
        NT::MoveToPreviousWorkspace => PT::MoveToPreviousWorkspace,
        NT::ToggleFloating => PT::ToggleFloating,
        NT::ToggleExposeMode
        | NT::ToggleScratchpad { .. }
        | NT::ShowScratchpad { .. }
        | NT::HideScratchpad { .. }
        | NT::HideAllScratchpads
        | NT::CloakWindow { .. }
        | NT::SetNativeBorderColor { .. }
        | NT::SetCaptionColor { .. }
        | NT::SetSystemBackdrop { .. }
        | NT::SetWindowCorner { .. } => PT::Unspecified,
    }
}

// ============================================================================
// Proto → Core Conversions
// ============================================================================

/// Error type for conversion failures
#[derive(Debug)]
pub enum ConversionError {
    /// The graph JSON does not parse as a node graph.
    InvalidGraphJson(String),
    /// The message uses the per-node encoding, which is no longer read.
    LegacyEncoding,
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionError::InvalidGraphJson(error) => write!(f, "Invalid graph JSON: {error}"),
            ConversionError::LegacyEncoding => write!(
                f,
                "The graph uses the old per-node encoding; update the client"
            ),
        }
    }
}

impl std::error::Error for ConversionError {}

/// The graph from the wire. A message without a graph (empty `json`) gives
/// an empty graph.
pub fn proto_to_node_graph(proto_graph: &proto::NodeGraph) -> Result<NodeGraph, ConversionError> {
    if proto_graph.json.is_empty() {
        if !proto_graph.nodes.is_empty() || !proto_graph.connections.is_empty() {
            return Err(ConversionError::LegacyEncoding);
        }
        return Ok(NodeGraph::new());
    }
    let mut graph: NodeGraph = serde_json::from_str(&proto_graph.json)
        .map_err(|error| ConversionError::InvalidGraphJson(error.to_string()))?;
    graph.rebuild_connection_indices();
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::graph::{
        BorderStyle, NodeGraph, NodeType, RuleMatchKind, RuleMatchStrategy, ScratchpadPosition,
        SystemBackdropType, TileLayout, WindowCornerType,
    };

    fn round_trip_node_type(node_type: NodeType) -> NodeType {
        let mut graph = NodeGraph::new();
        graph.add_node(node_type, [10.0, 20.0]);

        let proto = node_graph_to_proto(&graph);
        let converted = proto_to_node_graph(&proto).expect("graph should convert back to core");

        converted
            .nodes
            .get(&0)
            .expect("converted graph should contain node 0")
            .node_type
            .clone()
    }

    #[test]
    fn built_in_example_graphs_survive_protocol_round_trip() {
        for example in gevrek_model::examples::built_in_examples() {
            let proto = node_graph_to_proto(&example.config.node_graph);
            let converted =
                proto_to_node_graph(&proto).expect("built-in graph should convert back to core");
            assert_eq!(
                serde_json::to_value(&example.config.node_graph).unwrap(),
                serde_json::to_value(converted).unwrap(),
                "{} changed while crossing the editor/WM protocol",
                example.name
            );
        }
    }

    #[test]
    fn typed_dispatch_conversion_preserves_parameters() {
        use gevrek_model::command::{Direction, DispatchCommand, MonitorTarget};

        let request = dispatch_command_to_proto(DispatchCommand::ResizeActive {
            delta_width: 120,
            delta_height: -30,
        });
        assert_eq!(request.command, proto::DispatchType::ResizeActive as i32);
        assert_eq!(request.x, 120);
        assert_eq!(request.y, -30);

        let request = dispatch_command_to_proto(DispatchCommand::FocusDirection {
            direction: Direction::Up,
        });
        assert_eq!(request.command, proto::DispatchType::FocusDirection as i32);
        assert_eq!(request.direction, proto::Direction::Up as i32);

        let request = dispatch_command_to_proto(DispatchCommand::SetDwindleDirection {
            direction: gevrek_engine::layouts::DwindleDirection::Vertical,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::SetDwindleDirection as i32
        );
        assert_eq!(request.argument, "vertical");

        let request = dispatch_command_to_proto(DispatchCommand::ToggleWindowTag {
            tag: "communication".to_string(),
        });
        assert_eq!(request.command, proto::DispatchType::ToggleWindowTag as i32);
        assert_eq!(request.argument, "communication");

        let request = dispatch_command_to_proto(DispatchCommand::SnapActiveToZone { index: 3 });
        assert_eq!(
            request.command,
            proto::DispatchType::SnapActiveToZone as i32
        );
        assert_eq!(request.x, 3);

        let request = dispatch_command_to_proto(DispatchCommand::SetBindingSubmap {
            name: "resize".to_string(),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::SetBindingSubmap as i32
        );
        assert_eq!(request.argument, "resize");

        let request = dispatch_command_to_proto(DispatchCommand::ResetBindingSubmap);
        assert_eq!(
            request.command,
            proto::DispatchType::ResetBindingSubmap as i32
        );

        let request = dispatch_command_to_proto(DispatchCommand::RestartStartup {
            id: "status-bar".to_string(),
        });
        assert_eq!(request.command, proto::DispatchType::RestartStartup as i32);
        assert_eq!(request.argument, "status-bar");

        let request = dispatch_command_to_proto(DispatchCommand::FocusMonitor {
            target: MonitorTarget::Name("Side display".to_string()),
        });
        assert_eq!(request.command, proto::DispatchType::FocusMonitor as i32);
        assert_eq!(request.argument, "name:Side display");

        let request = dispatch_command_to_proto(DispatchCommand::MoveWorkspaceToMonitor {
            target: MonitorTarget::Next,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveWorkspaceToMonitor as i32
        );
        assert_eq!(request.argument, "next");

        let request = dispatch_command_to_proto(DispatchCommand::SwapActiveWorkspaces {
            target: MonitorTarget::Previous,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::SwapActiveWorkspaces as i32
        );
        assert_eq!(request.argument, "previous");

        let request = dispatch_command_to_proto(DispatchCommand::MoveWindowToMonitor {
            window: gevrek_model::command::WindowSelector::Active,
            target: MonitorTarget::Next,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveWindowToMonitor as i32
        );
        assert_eq!(request.argument, "active");
        assert_eq!(request.secondary_argument, "next");

        let request = dispatch_command_to_proto(DispatchCommand::MoveWindowToMonitorAndFollow {
            window: gevrek_model::command::WindowSelector::Handle(42),
            target: MonitorTarget::Primary,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveWindowToMonitorAndFollow as i32
        );
        assert_eq!(request.argument, "handle:42");
        assert_eq!(request.secondary_argument, "primary");

        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::Retile).command,
            proto::DispatchType::Retile as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleTiling).command,
            proto::DispatchType::ToggleTiling as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::ToggleWorkspaceTiling {
            selector: gevrek_model::command::WorkspaceSelector::Index(1),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleWorkspaceTiling as i32
        );
        assert_eq!(request.argument, "1");
        let request = dispatch_command_to_proto(DispatchCommand::ToggleWorkspaceFloatOverride {
            selector: gevrek_model::command::WorkspaceSelector::Name("code".to_string()),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleWorkspaceFloatOverride as i32
        );
        assert_eq!(request.argument, "name:code");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleTitleBar).command,
            proto::DispatchType::ToggleTitleBar as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::ToggleWindowTitleBar {
            selector: gevrek_model::command::WindowSelector::Class("Notepad".to_string()),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleWindowTitleBar as i32
        );
        assert_eq!(request.argument, "class:Notepad");

        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::FocusMonitorAtCursor).command,
            proto::DispatchType::FocusMonitorAtCursor as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::FocusMonitorWorkspace {
            target: MonitorTarget::Index(1),
            workspace: gevrek_model::command::WorkspaceSelector::Name("code".to_string()),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::FocusMonitorWorkspace as i32
        );
        assert_eq!(request.argument, "1");
        assert_eq!(request.secondary_argument, "name:code");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleMouseFollowsFocus).command,
            proto::DispatchType::ToggleMouseFollowsFocus as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::ManageWindow {
            selector: gevrek_model::command::WindowSelector::Handle(42),
        });
        assert_eq!(request.command, proto::DispatchType::ManageWindow as i32);
        assert_eq!(request.argument, "handle:42");
        let request = dispatch_command_to_proto(DispatchCommand::UnmanageWindow {
            selector: gevrek_model::command::WindowSelector::Active,
        });
        assert_eq!(request.command, proto::DispatchType::UnmanageWindow as i32);
        assert_eq!(request.argument, "active");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::RestoreWindows).command,
            proto::DispatchType::RestoreWindows as i32
        );

        let request = dispatch_command_to_proto(DispatchCommand::ResizeEdge {
            edge: gevrek_model::command::Direction::Right,
            sizing: gevrek_engine::layouts::Sizing::Increase,
        });
        assert_eq!(request.command, proto::DispatchType::ResizeEdge as i32);
        assert_eq!(request.direction, proto::Direction::Right as i32);
        assert_eq!(request.argument, "increase");

        let request = dispatch_command_to_proto(DispatchCommand::ResizeAxis {
            axis: gevrek_engine::layouts::LayoutAxis::HorizontalAndVertical,
            sizing: gevrek_engine::layouts::Sizing::Decrease,
        });
        assert_eq!(request.command, proto::DispatchType::ResizeAxis as i32);
        assert_eq!(request.argument, "horizontal-and-vertical");
        assert_eq!(request.secondary_argument, "decrease");

        let request = dispatch_command_to_proto(DispatchCommand::SetResizeDelta { pixels: 75 });
        assert_eq!(request.command, proto::DispatchType::SetResizeDelta as i32);
        assert_eq!(request.x, 75);

        let request = dispatch_command_to_proto(DispatchCommand::FlipLayout {
            axis: gevrek_engine::layouts::LayoutAxis::Vertical,
        });
        assert_eq!(request.command, proto::DispatchType::FlipLayout as i32);
        assert_eq!(request.argument, "vertical");

        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleMonocle).command,
            proto::DispatchType::ToggleMonocle as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleLayoutLock).command,
            proto::DispatchType::ToggleLayoutLock as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::StackAll).command,
            proto::DispatchType::StackAll as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::UnstackAll).command,
            proto::DispatchType::UnstackAll as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleStackbar).command,
            proto::DispatchType::ToggleStackbar as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::SaveResize {
            path: "C:\\layouts\\main.json".to_string(),
        });
        assert_eq!(request.command, proto::DispatchType::SaveResize as i32);
        assert_eq!(request.argument, "C:\\layouts\\main.json");
        let request = dispatch_command_to_proto(DispatchCommand::LoadResize {
            path: "snapshots/resume.json".to_string(),
        });
        assert_eq!(request.command, proto::DispatchType::LoadResize as i32);
        assert_eq!(request.argument, "snapshots/resume.json");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::QuickSaveResize).command,
            proto::DispatchType::QuickSaveResize as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::QuickLoadResize).command,
            proto::DispatchType::QuickLoadResize as i32
        );
        let request =
            dispatch_command_to_proto(DispatchCommand::WatchConfiguration { enabled: true });
        assert_eq!(
            request.command,
            proto::DispatchType::WatchConfiguration as i32
        );
        assert_eq!(request.x, 1);

        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleTransparency).command,
            proto::DispatchType::ToggleTransparency as i32
        );
        let request =
            dispatch_command_to_proto(DispatchCommand::SetTransparencyAlpha { alpha: 180 });
        assert_eq!(
            request.command,
            proto::DispatchType::SetTransparencyAlpha as i32
        );
        assert_eq!(request.x, 180);
        let request = dispatch_command_to_proto(DispatchCommand::SetGlobalWorkAreaOffset {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::SetGlobalWorkAreaOffset as i32
        );
        assert_eq!(request.argument, "10,20,30,40");
        let request = dispatch_command_to_proto(DispatchCommand::SetWorkspaceWorkAreaOffset {
            left: 1,
            top: 2,
            right: 3,
            bottom: 4,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::SetWorkspaceWorkAreaOffset as i32
        );
        assert_eq!(request.argument, "1,2,3,4");
        let request =
            dispatch_command_to_proto(DispatchCommand::SetWorkspacePadding { padding: 12 });
        assert_eq!(
            request.command,
            proto::DispatchType::SetWorkspacePadding as i32
        );
        assert_eq!(request.x, 12);
        let request =
            dispatch_command_to_proto(DispatchCommand::AdjustWorkspacePadding { delta: -5 });
        assert_eq!(
            request.command,
            proto::DispatchType::AdjustWorkspacePadding as i32
        );
        assert_eq!(request.x, -5);
        let request =
            dispatch_command_to_proto(DispatchCommand::SetContainerPadding { padding: 8 });
        assert_eq!(
            request.command,
            proto::DispatchType::SetContainerPadding as i32
        );
        assert_eq!(request.x, 8);
        let request =
            dispatch_command_to_proto(DispatchCommand::AdjustContainerPadding { delta: 3 });
        assert_eq!(
            request.command,
            proto::DispatchType::AdjustContainerPadding as i32
        );
        assert_eq!(request.x, 3);

        let request = dispatch_command_to_proto(DispatchCommand::ForceFocusWindow {
            selector: gevrek_model::command::WindowSelector::Active,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ForceFocusWindow as i32
        );
        assert_eq!(request.argument, "active");

        let request = dispatch_command_to_proto(DispatchCommand::NewWorkspace {
            name: Some("mail".to_string()),
        });
        assert_eq!(request.command, proto::DispatchType::NewWorkspace as i32);
        assert_eq!(request.argument, "mail");
        let request = dispatch_command_to_proto(DispatchCommand::NewWorkspace { name: None });
        assert_eq!(request.command, proto::DispatchType::NewWorkspace as i32);
        assert!(request.argument.is_empty());
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::CloseWorkspace).command,
            proto::DispatchType::CloseWorkspace as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::EnsureWorkspaces {
            target: gevrek_model::command::MonitorTarget::Index(1),
            count: 4,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::EnsureWorkspaces as i32
        );
        assert_eq!(request.argument, "1");
        assert_eq!(request.x, 4);
        let request = dispatch_command_to_proto(DispatchCommand::EnsureNamedWorkspace {
            target: gevrek_model::command::MonitorTarget::Primary,
            name: "chat".to_string(),
            layout: None,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::EnsureNamedWorkspace as i32
        );
        assert_eq!(request.argument, "primary");
        assert_eq!(request.secondary_argument, "chat");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ToggleWindowContainerBehaviour).command,
            proto::DispatchType::ToggleWindowContainerBehaviour as i32
        );
        let request =
            dispatch_command_to_proto(DispatchCommand::ToggleWorkspaceWindowContainerBehaviour {
                selector: gevrek_model::command::WorkspaceSelector::Index(2),
            });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleWorkspaceWindowContainerBehaviour as i32
        );
        assert_eq!(request.argument, "2");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::Promote).command,
            proto::DispatchType::Promote as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::PromoteFocus).command,
            proto::DispatchType::PromoteFocus as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::PromoteWindow {
            direction: gevrek_model::command::Direction::Down,
        });
        assert_eq!(request.command, proto::DispatchType::PromoteWindow as i32);
        assert_eq!(request.direction, proto::Direction::Down as i32);

        let request = dispatch_command_to_proto(DispatchCommand::MoveToWorkspaceAndFollow {
            selector: gevrek_model::command::WorkspaceSelector::Name("web".to_string()),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveToWorkspaceAndFollow as i32
        );
        assert_eq!(request.argument, "name:web");

        let request = dispatch_command_to_proto(DispatchCommand::RenameWorkspace {
            selector: gevrek_model::command::WorkspaceSelector::Index(2),
            name: "research".to_string(),
        });
        assert_eq!(request.command, proto::DispatchType::RenameWorkspace as i32);
        assert_eq!(request.argument, "2");
        assert_eq!(request.secondary_argument, "research");

        let request = dispatch_command_to_proto(DispatchCommand::SetFloating { enabled: false });
        assert_eq!(request.command, proto::DispatchType::SetFloating as i32);
        assert_eq!(request.x, 0);
        let request = dispatch_command_to_proto(DispatchCommand::SetPinned { enabled: true });
        assert_eq!(request.command, proto::DispatchType::SetPinned as i32);
        assert_eq!(request.x, 1);
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::CenterActive).command,
            proto::DispatchType::CenterActive as i32
        );
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::ClearWindowTags).command,
            proto::DispatchType::ClearWindowTags as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::FocusWindow {
            selector: gevrek_model::command::WindowSelector::Tag("communication".to_string()),
        });
        assert_eq!(request.command, proto::DispatchType::FocusWindow as i32);
        assert_eq!(request.argument, "tag:communication");
        assert_eq!(
            dispatch_command_to_proto(DispatchCommand::FocusLast).command,
            proto::DispatchType::FocusLast as i32
        );
        let request = dispatch_command_to_proto(DispatchCommand::MoveWindow {
            selector: gevrek_model::command::WindowSelector::Tag("scratch".to_string()),
            x: 40,
            y: -20,
            relative: true,
        });
        assert_eq!(request.command, proto::DispatchType::MoveWindow as i32);
        assert_eq!(request.argument, "tag:scratch");
        assert_eq!((request.x, request.y), (40, -20));
        assert_eq!(request.secondary_argument, "relative");
        let request = dispatch_command_to_proto(DispatchCommand::SetWindowPinned {
            selector: gevrek_model::command::WindowSelector::Class("Tool".to_string()),
            enabled: true,
        });
        assert_eq!(request.command, proto::DispatchType::SetWindowPinned as i32);
        assert_eq!(request.argument, "class:Tool");
        assert_eq!(request.x, 1);
        let request = dispatch_command_to_proto(DispatchCommand::ToggleWindowFullscreen {
            selector: gevrek_model::command::WindowSelector::ProcessId(4242),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleWindowFullscreen as i32
        );
        assert_eq!(request.argument, "pid:4242");
        let request = dispatch_command_to_proto(DispatchCommand::ToggleSelectedWindowTag {
            selector: gevrek_model::command::WindowSelector::Tag("communication".to_string()),
            tag: "urgent".to_string(),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::ToggleSelectedWindowTag as i32
        );
        assert_eq!(request.argument, "tag:communication");
        assert_eq!(request.secondary_argument, "urgent");
        let request = dispatch_command_to_proto(DispatchCommand::MoveWindowToWorkspaceAndFollow {
            window: gevrek_model::command::WindowSelector::ProcessId(4242),
            workspace: gevrek_model::command::WorkspaceSelector::Name("chat".to_string()),
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveSelectedToWorkspaceAndFollow as i32
        );
        assert_eq!(request.argument, "pid:4242");
        assert_eq!(request.secondary_argument, "name:chat");
        let request = dispatch_command_to_proto(DispatchCommand::CycleWindowGroup {
            selector: gevrek_model::command::WindowSelector::Tag("tabs".to_string()),
            forward: true,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::CycleWindowGroup as i32
        );
        assert_eq!(request.argument, "tag:tabs");
        assert_eq!(request.x, 1);
        let request = dispatch_command_to_proto(DispatchCommand::FocusWindowGroupIndex {
            selector: gevrek_model::command::WindowSelector::Class("Browser".to_string()),
            index: 2,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::FocusWindowGroupIndex as i32
        );
        assert_eq!(request.argument, "class:Browser");
        assert_eq!(request.x, 2);
        let request = dispatch_command_to_proto(DispatchCommand::MoveWindowInGroup {
            selector: gevrek_model::command::WindowSelector::ProcessId(4242),
            forward: false,
        });
        assert_eq!(
            request.command,
            proto::DispatchType::MoveWindowInGroup as i32
        );
        assert_eq!(request.argument, "pid:4242");
        assert_eq!(request.x, 0);
    }

    #[test]
    fn create_workspace_preserves_border_colors() {
        let original = NodeType::CreateWorkspace {
            workspace_name: "main".to_string(),
            layout: TileLayout::BSP,
            padding_x: 3,
            padding_y: 4,
            workspace_padding_x: 5,
            workspace_padding_y: 6,
            border_enabled: true,
            border_width: 7,
            border_offset: -2,
            border_style: BorderStyle::Rounded,
            active_border_color: (12, 34, 56),
            inactive_border_color: (78, 90, 123),
            active_border_alpha: 222,
            inactive_border_alpha: 111,
        };

        assert_eq!(round_trip_node_type(original.clone()), original);
    }

    #[test]
    fn core_only_node_types_round_trip_without_placeholder_semantics() {
        let cases = vec![
            NodeType::ForLoop {
                start: 1,
                end: 10,
                step: 2,
            },
            NodeType::GetWorkspace { workspace_index: 3 },
            NodeType::Retile {
                layout: TileLayout::Dwindle,
            },
            NodeType::SetLayoutRatio { ratio: 0.65 },
            NodeType::IncrementLayoutRatio { step: 0.1 },
            NodeType::DecrementLayoutRatio { step: 0.15 },
            NodeType::SetMasterCount { count: 2 },
            NodeType::IncrementMasterCount,
            NodeType::DecrementMasterCount,
            NodeType::SetLayoutGap { gap: 9 },
            NodeType::CycleNextLayout,
            NodeType::CyclePreviousLayout,
            NodeType::RotateLayout,
            NodeType::ToString,
            NodeType::FromString,
            NodeType::CycleFocusNext,
            NodeType::CycleFocusPrevious,
            NodeType::ToggleExposeMode,
            NodeType::ToggleScratchpad {
                scratchpad_id: "terminal".to_string(),
                pattern: "WindowsTerminal".to_string(),
                match_kind: RuleMatchKind::Exe,
                match_strategy: RuleMatchStrategy::Contains,
                spawn_command: Some("wt.exe".to_string()),
                width_percent: 80,
                height_percent: 55,
                position: ScratchpadPosition::BottomRight,
            },
            NodeType::ShowScratchpad {
                scratchpad_id: "terminal".to_string(),
            },
            NodeType::HideScratchpad {
                scratchpad_id: "terminal".to_string(),
            },
            NodeType::HideAllScratchpads,
            NodeType::CloakWindow { cloak: true },
            NodeType::SetNativeBorderColor { r: 1, g: 2, b: 3 },
            NodeType::SetCaptionColor { r: 4, g: 5, b: 6 },
            NodeType::SetSystemBackdrop {
                backdrop_type: SystemBackdropType::MicaAlt,
            },
            NodeType::SetWindowCorner {
                corner_type: WindowCornerType::RoundedSmall,
            },
        ];

        for original in cases {
            assert_eq!(round_trip_node_type(original.clone()), original);
        }
    }

    #[test]
    fn id_counters_survive_round_trip_after_deletions() {
        let mut graph = NodeGraph::new();
        graph.add_node(NodeType::OnStartup, [0.0, 0.0]);
        let last = graph.add_node(NodeType::ApplySystemFilters { enabled: true }, [1.0, 1.0]);
        graph.remove_node(last);

        let converted = proto_to_node_graph(&node_graph_to_proto(&graph)).unwrap();
        assert_eq!(converted.id_counters(), graph.id_counters());
    }

    #[test]
    fn the_wire_graph_is_the_configuration_file_json() {
        let mut graph = NodeGraph::new();
        graph.add_node(NodeType::ApplySystemFilters { enabled: false }, [3.0, 4.0]);
        let proto = node_graph_to_proto(&graph);
        assert_eq!(proto.json, serde_json::to_string(&graph).unwrap());
        assert_eq!((proto.node_count, proto.connection_count), (1, 0));
        assert!(
            proto.nodes.is_empty(),
            "the per-node encoding is not written"
        );
    }

    #[test]
    fn an_empty_message_is_an_empty_graph_and_bad_json_is_refused() {
        let empty = proto_to_node_graph(&proto::NodeGraph::default()).unwrap();
        assert!(empty.nodes.is_empty());

        let broken = proto::NodeGraph {
            json: "{not json".to_string(),
            ..Default::default()
        };
        assert!(matches!(
            proto_to_node_graph(&broken),
            Err(ConversionError::InvalidGraphJson(_))
        ));

        let legacy = proto::NodeGraph {
            nodes: vec![proto::Node::default()],
            ..Default::default()
        };
        assert!(matches!(
            proto_to_node_graph(&legacy),
            Err(ConversionError::LegacyEncoding)
        ));
    }
}
