use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::graph::NodeGraph;
use crate::settings::{ConfigIssue, DesktopConfig};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Config {
    /// The gevrek version that wrote the file. Informational.
    #[serde(default = "current_version")]
    pub version: String,
    #[serde(default)]
    pub desktop: DesktopConfig,
    #[serde(default = "NodeGraph::new")]
    pub node_graph: NodeGraph,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// What `Config::normalize` had to remove from the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GraphCleanup {
    pub removed_nodes: usize,
    pub removed_connections: usize,
}

impl GraphCleanup {
    /// Nothing was removed.
    pub fn is_clean(&self) -> bool {
        self.removed_nodes == 0 && self.removed_connections == 0
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            extra: Default::default(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            desktop: DesktopConfig::default(),
            node_graph: NodeGraph::new(),
        }
    }

    pub fn validate(&self) -> Vec<ConfigIssue> {
        let mut issues = self.desktop.validate();
        // What this version does not know is kept and does nothing: say so.
        const NEWER: &str =
            "is not known to this version of gevrek; it is kept in the file and does nothing";
        let unknown_command = |command: &crate::command::DispatchCommand| match command {
            crate::command::DispatchCommand::Unknown(raw) => {
                Some(crate::settings::unknown_summary(raw))
            }
            _ => None,
        };
        for (index, binding) in self.desktop.bindings.iter().enumerate() {
            if let Some(raw) = unknown_command(&binding.command) {
                issues.push(ConfigIssue::warning(
                    format!("desktop.bindings[{index}].command"),
                    format!("The command {raw} {NEWER}"),
                ));
            }
        }
        for (index, profile) in self.desktop.gestures.profiles.iter().enumerate() {
            if let Some(raw) = unknown_command(&profile.command) {
                issues.push(ConfigIssue::warning(
                    format!("desktop.gestures.profiles[{index}].command"),
                    format!("The command {raw} {NEWER}"),
                ));
            }
        }
        for (index, rule) in self.desktop.window_rules.iter().enumerate() {
            for (effect_index, effect) in rule.effects.iter().enumerate() {
                if let crate::settings::RuleEffect::Unknown(raw) = effect {
                    issues.push(ConfigIssue::warning(
                        format!("desktop.window_rules[{index}].effects[{effect_index}]"),
                        format!(
                            "The effect {} {NEWER}",
                            crate::settings::unknown_summary(raw)
                        ),
                    ));
                }
            }
        }
        let mut unknown_nodes: Vec<_> = self
            .node_graph
            .nodes
            .iter()
            .filter_map(|(id, node)| match &node.node_type {
                crate::graph::NodeType::Unknown(raw) => Some((*id, raw)),
                _ => None,
            })
            .collect();
        unknown_nodes.sort_by_key(|(id, _)| *id);
        for (id, raw) in unknown_nodes {
            issues.push(ConfigIssue::warning(
                format!("node_graph.nodes[{id}]"),
                format!("The node {} {NEWER}", crate::settings::unknown_summary(raw)),
            ));
        }
        let mut chords = std::collections::HashMap::new();
        for (index, binding) in self.desktop.bindings.iter().enumerate() {
            if binding.enabled && !binding.key.trim().is_empty() {
                chords.insert(binding.chord_key(), index);
            }
        }
        for (node_id, node) in &self.node_graph.nodes {
            if let crate::graph::NodeType::OnKeyPress {
                key,
                ctrl,
                alt,
                shift,
                win,
            } = &node.node_type
            {
                if !key.trim().is_empty() && crate::keys::key_name_to_vk(key).is_none() {
                    issues.push(ConfigIssue::error(
                        format!("node_graph.nodes[{node_id}]"),
                        format!("'{}' is no key the keyboard has", key.trim()),
                    ));
                }
                let chord = crate::settings::BindingProfile {
                    key: key.clone(),
                    ctrl: *ctrl,
                    alt: *alt,
                    shift: *shift,
                    win: *win,
                    ..crate::settings::BindingProfile::default()
                }
                .chord_key();
                if let Some(index) = chords.get(&chord) {
                    issues.push(ConfigIssue::error(
                        format!("desktop.bindings[{index}]"),
                        format!(
                            "Binding conflicts with automation hotkey node {node_id}; keep the action in one configuration surface"
                        ),
                    ));
                }
            }
        }
        issues
    }

    /// Create a config with the default node graph.
    /// New windows are tiled on every monitor unless a rule floats them.
    pub fn with_default_graph() -> Self {
        use crate::graph::{NodeType, WindowMode};

        let mut config = Self::new();

        log::debug!("Creating the default configuration");

        // --- OnStartup Logic (Phase 1: Apply system filters, then create workspaces) ---
        let on_startup = config
            .node_graph
            .add_node(NodeType::OnStartup, [50.0, 100.0]);

        // Apply system filters to ignore tooltips, taskbar, etc.
        let apply_filters = config.node_graph.add_node(
            NodeType::ApplySystemFilters { enabled: true },
            [300.0, 100.0],
        );

        // --- Dynamic Rule Creation Example (demonstrates new CreateRule + CombineRules system) ---
        // Create individual float rules dynamically
        use crate::graph::{RuleMatchKind, RuleMatchStrategy};

        let create_rule_1 = config.node_graph.add_node(
            NodeType::CreateRule {
                pattern: String::new(),
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
            },
            [550.0, 50.0],
        );
        let pattern_1 = config.node_graph.add_node(
            NodeType::StringValue {
                value: "Settings".to_string(),
            },
            [550.0, 0.0],
        );

        let create_rule_2 = config.node_graph.add_node(
            NodeType::CreateRule {
                pattern: String::new(),
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
            },
            [550.0, 180.0],
        );
        let pattern_2 = config.node_graph.add_node(
            NodeType::StringValue {
                value: "Properties".to_string(),
            },
            [550.0, 130.0],
        );

        // Combine the two rules into a RuleList
        let combine_rules = config
            .node_graph
            .add_node(NodeType::CombineRules, [800.0, 100.0]);

        // Apply the combined rules as float rules
        let create_float_rules = config.node_graph.add_node(
            NodeType::CreateFloatRule {
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                pattern: String::new(),
                rules: Vec::new(),
            },
            [1050.0, 100.0],
        );

        // Wire up dynamic rule creation flow
        let _ = config.node_graph.add_connection_by_name(
            apply_filters,
            "Then",
            create_rule_1,
            "Trigger",
        );
        let _ =
            config
                .node_graph
                .add_connection_by_name(pattern_1, "Value", create_rule_1, "Pattern");
        let _ = config.node_graph.add_connection_by_name(
            create_rule_1,
            "Then",
            create_rule_2,
            "Trigger",
        );
        let _ =
            config
                .node_graph
                .add_connection_by_name(pattern_2, "Value", create_rule_2, "Pattern");
        let _ = config.node_graph.add_connection_by_name(
            create_rule_2,
            "Then",
            combine_rules,
            "Trigger",
        );
        let _ = config.node_graph.add_connection_by_name(
            create_rule_1,
            "Rule",
            combine_rules,
            "Rule 1",
        );
        let _ = config.node_graph.add_connection_by_name(
            create_rule_2,
            "Rule",
            combine_rules,
            "Rule 2",
        );
        let _ = config.node_graph.add_connection_by_name(
            combine_rules,
            "Then",
            create_float_rules,
            "Trigger",
        );
        let _ = config.node_graph.add_connection_by_name(
            combine_rules,
            "Rules",
            create_float_rules,
            "Rules",
        );

        // Create a columns workspace for each monitor. Auto arrangement below keeps
        // the primary monitor floating-only and tiles the first secondary monitor.
        let get_monitors = config
            .node_graph
            .add_node(NodeType::GetMonitors, [1300.0, 100.0]);
        let monitor_loop = config
            .node_graph
            .add_node(NodeType::ForEach, [1550.0, 100.0]);
        let create_workspace = config.node_graph.add_node(
            NodeType::CreateWorkspace {
                workspace_name: "Workspace".to_string(),
                layout: crate::layout::LayoutType::Columns,
                padding_x: 5,
                padding_y: 5,
                workspace_padding_x: 10,
                workspace_padding_y: 10,
                border_enabled: true,
                border_width: 4,
                border_offset: -1,
                border_style: crate::graph::BorderStyle::Rounded,
                active_border_color: (255, 0, 0),
                inactive_border_color: (0, 0, 255),
                active_border_alpha: 255,
                inactive_border_alpha: 200,
            },
            [1800.0, 100.0],
        );

        // Wire up the flow: OnStartup → ApplySystemFilters → [Dynamic Rules] → GetMonitors → ForEach → CreateWorkspace
        let _ =
            config
                .node_graph
                .add_connection_by_name(on_startup, "Event", apply_filters, "Trigger");
        let _ = config.node_graph.add_connection_by_name(
            create_float_rules,
            "Then",
            get_monitors,
            "Trigger",
        );
        let _ =
            config
                .node_graph
                .add_connection_by_name(get_monitors, "Then", monitor_loop, "Trigger");
        let _ = config.node_graph.add_connection_by_name(
            get_monitors,
            "Monitors",
            monitor_loop,
            "List",
        );
        let _ = config.node_graph.add_connection_by_name(
            monitor_loop,
            "Loop Body",
            create_workspace,
            "Trigger",
        );
        let _ = config.node_graph.add_connection_by_name(
            monitor_loop,
            "Item",
            create_workspace,
            "Monitor",
        );

        // Existing windows are intentionally not arranged on startup. Starting
        // the WM should be non-destructive; new windows are handled below.

        // --- OnWindowCreated Logic (auto-detects monitor per window and floating vs tiled) ---
        let on_window_created = config
            .node_graph
            .add_node(NodeType::OnWindowCreated, [50.0, 500.0]);
        let set_new_window_arrangement = config.node_graph.add_node(
            NodeType::SetWindowArrangement {
                mode: WindowMode::Auto,
            },
            [300.0, 500.0],
        );

        let _ = config.node_graph.add_connection_by_name(
            on_window_created,
            "Event",
            set_new_window_arrangement,
            "Trigger",
        );
        let _ = config.node_graph.add_connection_by_name(
            on_window_created,
            "Window",
            set_new_window_arrangement,
            "Window",
        );

        // --- Window Navigation Hotkeys ---

        // Ctrl+Alt+J: Focus next window
        let on_key_focus_next = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "J".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 700.0],
        );
        let focus_next = config
            .node_graph
            .add_node(NodeType::FocusNextWindow, [300.0, 700.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_focus_next,
            "Event",
            focus_next,
            "Trigger",
        );

        // Ctrl+Alt+K: Focus previous window
        let on_key_focus_prev = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "K".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 800.0],
        );
        let focus_prev = config
            .node_graph
            .add_node(NodeType::FocusPreviousWindow, [300.0, 800.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_focus_prev,
            "Event",
            focus_prev,
            "Trigger",
        );

        // --- Window Position Swapping ---

        // Ctrl+Shift+J: Swap with next window
        let on_key_swap_next = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "J".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            [50.0, 900.0],
        );
        let swap_next = config
            .node_graph
            .add_node(NodeType::SwapWithNext, [300.0, 900.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_swap_next,
            "Event",
            swap_next,
            "Trigger",
        );

        // Ctrl+Shift+K: Swap with previous window
        let on_key_swap_prev = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "K".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            [50.0, 1000.0],
        );
        let swap_prev = config
            .node_graph
            .add_node(NodeType::SwapWithPrevious, [300.0, 1000.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_swap_prev,
            "Event",
            swap_prev,
            "Trigger",
        );

        // --- Workspace Navigation ---

        // Ctrl+Alt+L: Switch to next workspace
        let on_key_next_ws = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "L".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 1100.0],
        );
        let next_ws = config
            .node_graph
            .add_node(NodeType::NextWorkspace, [300.0, 1100.0]);
        let _ =
            config
                .node_graph
                .add_connection_by_name(on_key_next_ws, "Event", next_ws, "Trigger");

        // Ctrl+Alt+H: Switch to previous workspace
        let on_key_prev_ws = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "H".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 1200.0],
        );
        let prev_ws = config
            .node_graph
            .add_node(NodeType::PreviousWorkspace, [300.0, 1200.0]);
        let _ =
            config
                .node_graph
                .add_connection_by_name(on_key_prev_ws, "Event", prev_ws, "Trigger");

        // --- Move Window to Workspace ---

        // Ctrl+Shift+L: Move active window to next workspace
        let on_key_move_next = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "L".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            [50.0, 1300.0],
        );
        let move_next_ws = config
            .node_graph
            .add_node(NodeType::MoveToNextWorkspace, [300.0, 1300.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_move_next,
            "Event",
            move_next_ws,
            "Trigger",
        );

        // Ctrl+Shift+H: Move active window to previous workspace
        let on_key_move_prev = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "H".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            [50.0, 1400.0],
        );
        let move_prev_ws = config
            .node_graph
            .add_node(NodeType::MoveToPreviousWorkspace, [300.0, 1400.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_move_prev,
            "Event",
            move_prev_ws,
            "Trigger",
        );

        // --- Toggle Floating ---

        // Ctrl+Alt+F: Toggle active window between floating and tiled
        let on_key_toggle_floating = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "F".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 1500.0],
        );
        let toggle_floating = config
            .node_graph
            .add_node(NodeType::ToggleFloating, [300.0, 1500.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_toggle_floating,
            "Event",
            toggle_floating,
            "Trigger",
        );

        // --- Layout Cycling ---

        // Ctrl+Alt+Space: Cycle to next layout
        let on_key_cycle_next_layout = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "Space".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 1600.0],
        );
        let cycle_next_layout = config
            .node_graph
            .add_node(NodeType::CycleNextLayout, [300.0, 1600.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_cycle_next_layout,
            "Event",
            cycle_next_layout,
            "Trigger",
        );

        // Ctrl+Shift+Space: Cycle to previous layout
        let on_key_cycle_prev_layout = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "Space".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                win: false,
            },
            [50.0, 1700.0],
        );
        let cycle_prev_layout = config
            .node_graph
            .add_node(NodeType::CyclePreviousLayout, [300.0, 1700.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_cycle_prev_layout,
            "Event",
            cycle_prev_layout,
            "Trigger",
        );

        // Ctrl+Alt+E: Toggle Expose Mode
        let on_key_expose = config.node_graph.add_node(
            NodeType::OnKeyPress {
                key: "E".to_string(),
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            [50.0, 1800.0],
        );
        let toggle_expose = config
            .node_graph
            .add_node(NodeType::ToggleExposeMode, [300.0, 1800.0]);
        let _ = config.node_graph.add_connection_by_name(
            on_key_expose,
            "Event",
            toggle_expose,
            "Trigger",
        );

        // The nodes in order: from left to right the way things happen.
        crate::graph_layout::tidy(&mut config.node_graph);

        // The list of the shortcuts themselves, so that they can be found.
        config
            .desktop
            .bindings
            .push(crate::settings::BindingProfile {
                name: "Show shortcuts".to_string(),
                key: "F1".to_string(),
                command: crate::command::DispatchCommand::ShowShortcuts,
                ..crate::settings::BindingProfile::default()
            });

        log::info!(
            "Default configuration created with {} nodes and {} connections",
            config.node_graph.nodes.len(),
            config.node_graph.connections.len()
        );

        config
    }

    /// Check a configuration that was just parsed (from the file, the wire,
    /// or the editor) and bring its graph into its canonical in-memory form.
    /// This is the one validation every part of gevrek uses.
    ///
    /// - A settings error (`validate`) is returned and nothing is changed.
    /// - The graph's connection indices are rebuilt and node pins refreshed
    ///   to the current node definitions (older files keep working).
    /// - Nodes and connections that are still invalid are removed; the
    ///   result says how many, so a caller can refuse or warn.
    pub fn normalize(&mut self) -> std::result::Result<GraphCleanup, crate::settings::ConfigIssue> {
        if let Some(issue) = self
            .validate()
            .into_iter()
            .find(|issue| issue.severity == crate::settings::ConfigIssueSeverity::Error)
        {
            return Err(issue);
        }
        // Counted across all three steps: the pin refresh drops connections
        // it cannot place, and those must be reported like any other.
        let (nodes, connections) = (
            self.node_graph.nodes.len(),
            self.node_graph.connections.len(),
        );
        self.node_graph.rebuild_connection_indices();
        self.node_graph.refresh_node_pins();
        self.node_graph.validate_and_clean();
        Ok(GraphCleanup {
            removed_nodes: nodes.saturating_sub(self.node_graph.nodes.len()),
            removed_connections: connections.saturating_sub(self.node_graph.connections.len()),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config = Config::from_json(&content)?;

        // Loading never rewrites the file. Invalid graph data is dropped from
        // the configuration in memory only; the file changes when the user
        // saves or applies.
        let cleaned = config.normalize().map_err(|issue| {
            anyhow::anyhow!("Invalid configuration at {}: {}", issue.path, issue.message)
        })?;
        if !cleaned.is_clean() {
            log::warn!(
                "{}: ignored {} invalid node(s) and {} invalid connection(s); the file was left as it is",
                path.display(),
                cleaned.removed_nodes,
                cleaned.removed_connections
            );
        }

        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let issues = self.validate();
        if let Some(issue) = issues
            .iter()
            .find(|issue| issue.severity == crate::settings::ConfigIssueSeverity::Error)
        {
            anyhow::bail!("Invalid configuration at {}: {}", issue.path, issue.message);
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        // What is on disk now is kept as a backup first (not when it is
        // the same text, and never a reason for the save to fail).
        if let Ok(previous) = std::fs::read_to_string(path) {
            if previous != content {
                if let Err(error) = crate::files::backup(path, &previous) {
                    log::warn!("No backup of {} was kept: {error}", path.display());
                }
            }
        }
        crate::files::write_atomic(path, content.as_bytes())?;
        Ok(())
    }
}

impl Config {
    /// Read a configuration from JSON text, whatever schema version wrote
    /// it: an older layout is brought to the current one first (see
    /// `crate::migrate`). Every place that takes a configuration from a
    /// file or from another program goes through here.
    pub fn from_json(text: &str) -> serde_json::Result<Self> {
        // (A mark of the byte order at the start, as some editors on
        // Windows put before UTF-8, is no part of the text.)
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut value: serde_json::Value = serde_json::from_str(text)?;
        crate::migrate::migrate(&mut value);
        // (Snap zones are `desktop.snap_zones` since schema 26. A file of
        // that schema with them under `desktop.zones` is not guessed at.)
        if crate::migrate::has_snap_zones_under_zones(&value) {
            return Err(serde::de::Error::custom(
                "desktop.zones holds snap zones (behavior, default_layout, monitor_layouts); \
                 since schema 26 their place is desktop.snap_zones",
            ));
        }
        serde_json::from_value(value)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// The configuration file programs use when none is named: see
/// [`crate::paths::config_file`].
pub fn get_config_path() -> std::path::PathBuf {
    crate::paths::config_file()
}

#[cfg(test)]
mod compatibility_tests {
    //! One file for every system and every version: what a program does
    //! not know it keeps (docs/DESKTOP_PLAN.md, 4.2).
    use super::*;
    use serde_json::json;

    /// A configuration with keys and values no version knows, at every
    /// level a newer version could add them.
    fn from_the_future() -> serde_json::Value {
        let mut document = serde_json::to_value(crate::examples::starting_configuration()).unwrap();
        document["future_top_level"] = json!({ "a": 1 });
        document["desktop"]["future_section"] = json!({ "enabled": true, "list": [1, 2, 3] });
        document["desktop"]["appearance"]["future_look"] = json!("glass");
        document["desktop"]["appearance"]["effects"]["shadow"]["future_spread"] = json!(4);
        document["desktop"]["focus"]["future_focus"] = json!(false);
        document["desktop"]["platform"]["macos"] = json!({ "stage_manager": false });
        document["desktop"]["platform"]["wayland"]["future_wayland"] = json!(1.5);
        document["desktop"]["workspaces"][0]["future_workspace"] = json!("x");
        document["desktop"]["bindings"][0]["allow_when_locked"] = json!(true);
        document["desktop"]["bindings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "name": "From the future",
                "key": "F13",
                "command": { "command": "teleport_window", "to": "mars" }
            }));
        document["desktop"]["window_rules"] = json!([{
            "name": "Future rule",
            "predicates": [{ "kind": "Title", "strategy": "Contains", "value": "x", "future_predicate": 1 }],
            "effects": ["Float", { "Hologram": { "depth": 3 } }]
        }]);
        let nodes = document["node_graph"]["nodes"].as_object_mut().unwrap();
        nodes.insert(
            "9000".to_string(),
            json!({
                "id": 9000,
                "node_type": { "SummonWindow": { "from": "elsewhere" } },
                "position": [10.0, 20.0],
                "inputs": [{ "id": 90001, "name": "Trigger", "pin_type": "Execution" }],
                "outputs": []
            }),
        );
        document
    }

    /// Everything in `expected` is in `actual` with the same value (objects
    /// may have more keys; arrays have the same length).
    fn assert_contains(actual: &serde_json::Value, expected: &serde_json::Value, path: &str) {
        match (actual, expected) {
            (serde_json::Value::Object(actual), serde_json::Value::Object(expected)) => {
                for (key, value) in expected {
                    let found = actual
                        .get(key)
                        .unwrap_or_else(|| panic!("{path}.{key} was lost"));
                    assert_contains(found, value, &format!("{path}.{key}"));
                }
            }
            (serde_json::Value::Array(actual), serde_json::Value::Array(expected)) => {
                assert_eq!(actual.len(), expected.len(), "{path}: length");
                for (index, (found, value)) in actual.iter().zip(expected).enumerate() {
                    assert_contains(found, value, &format!("{path}[{index}]"));
                }
            }
            _ => assert_eq!(actual, expected, "{path}"),
        }
    }

    #[test]
    fn what_this_version_does_not_know_survives_reading_and_writing() {
        let original = from_the_future();
        let config = Config::from_json(&original.to_string()).expect("a newer file still loads");
        let written = serde_json::to_value(&config).unwrap();
        assert_contains(&written, &original, "config");
        // And again: nothing drifts on a second pass.
        let again = Config::from_json(&written.to_string()).unwrap();
        assert_eq!(serde_json::to_value(&again).unwrap(), written);
    }

    #[test]
    fn what_this_version_does_not_know_is_reported_and_does_not_block() {
        let config = Config::from_json(&from_the_future().to_string()).unwrap();
        let issues = config.validate();
        let warnings: Vec<&str> = issues
            .iter()
            .filter(|issue| issue.severity == crate::settings::ConfigIssueSeverity::Warning)
            .map(|issue| issue.path.as_str())
            .collect();
        assert!(
            warnings
                .iter()
                .any(|path| path.starts_with("desktop.bindings[")),
            "{warnings:?}"
        );
        assert!(
            warnings.contains(&"desktop.window_rules[0].effects[1]"),
            "{warnings:?}"
        );
        assert!(warnings.contains(&"node_graph.nodes[9000]"), "{warnings:?}");
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity != crate::settings::ConfigIssueSeverity::Error),
            "{issues:?}"
        );
        // It can be normalized (as on load) and the unknown node keeps its pins.
        let mut config = config;
        config.normalize().expect("warnings do not stop a load");
        let node = &config.node_graph.nodes[&9000];
        assert!(matches!(node.node_type, crate::graph::NodeType::Unknown(_)));
        assert_eq!(node.inputs.len(), 1);
    }

    #[test]
    fn a_nearly_empty_file_is_a_configuration() {
        for text in [
            "{}",
            r#"{"desktop":{}}"#,
            r#"{"desktop":{"focus":{},"zones":{"behavior":{}}}}"#,
        ] {
            let config = Config::from_json(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(
                config.desktop.schema_version,
                crate::settings::DESKTOP_CONFIG_SCHEMA_VERSION
            );
        }
    }

    #[test]
    fn every_built_in_example_is_written_the_same_twice() {
        // The bytes a save produces do not depend on the run or the system
        // (CI compares them between Linux and Windows).
        for example in crate::examples::built_in_examples() {
            let first = serde_json::to_string_pretty(&example.config).unwrap();
            let second = serde_json::to_string_pretty(&Config::from_json(&first).unwrap()).unwrap();
            assert_eq!(first, second, "{}", example.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_a_file_with_invalid_graph_data_never_rewrites_it() {
        let dir = std::env::temp_dir().join(format!(
            "gevrek-load-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        // A connection that points at a node that does not exist.
        let mut config = Config::with_default_graph();
        let mut json = serde_json::to_value(&config).unwrap();
        json["node_graph"]["connections"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "from_node": 999_999, "from_pin": 1, "to_node": 999_998, "to_pin": 2
            }));
        let on_disk = serde_json::to_string_pretty(&json).unwrap();
        std::fs::write(&path, &on_disk).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.node_graph.connections.len(),
            config.node_graph.connections.len(),
            "the dangling connection is ignored in memory"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            on_disk,
            "file untouched"
        );
        assert!(
            !path.with_extension("json.backup").exists(),
            "and no backup is written"
        );

        // The same check reports the cleanup to callers that refuse it.
        config = serde_json::from_str(&on_disk).unwrap();
        let cleanup = config.normalize().unwrap();
        assert_eq!((cleanup.removed_nodes, cleanup.removed_connections), (0, 1));
        assert!(
            config.normalize().unwrap().is_clean(),
            "normalizing is idempotent"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalizing_a_current_configuration_changes_nothing() {
        // Pin and node ids are the graph's identity for the editor and on
        // the wire; a valid, current graph must pass through untouched.
        for example in crate::examples::built_in_examples() {
            let mut config = example.config;
            let before = serde_json::to_string(&config).unwrap();
            assert!(config.normalize().unwrap().is_clean());
            assert_eq!(serde_json::to_string(&config).unwrap(), before);
        }
    }

    #[test]
    fn a_settings_error_is_reported_and_nothing_is_changed() {
        // A focus history of zero entries is a settings error.
        let mut config = Config::with_default_graph();
        config.desktop.focus.history_size = 0;
        let before = serde_json::to_string(&config).unwrap();
        let issue = config.normalize().unwrap_err();
        assert!(issue.path.contains("history_size"), "{}", issue.path);
        assert_eq!(serde_json::to_string(&config).unwrap(), before);
    }

    #[test]
    fn legacy_graph_only_config_receives_safe_desktop_defaults() {
        let mut value = serde_json::to_value(Config::new()).unwrap();
        value.as_object_mut().unwrap().remove("desktop");

        let config: Config = serde_json::from_value(value).unwrap();
        assert_eq!(
            config.desktop.schema_version,
            crate::settings::DESKTOP_CONFIG_SCHEMA_VERSION
        );
        assert!(config.desktop.validate().is_empty());
    }

    #[test]
    fn declarative_bindings_cannot_shadow_graph_hotkeys() {
        let mut config = Config::new();
        config
            .desktop
            .bindings
            .push(crate::settings::BindingProfile {
                key: "K".to_string(),
                ctrl: true,
                ..crate::settings::BindingProfile::default()
            });
        config.node_graph.add_node(
            crate::graph::NodeType::OnKeyPress {
                key: "k".to_string(),
                ctrl: true,
                alt: false,
                shift: false,
                win: true,
            },
            [0.0, 0.0],
        );

        assert_eq!(
            config
                .validate()
                .iter()
                .filter(|issue| issue.severity == crate::settings::ConfigIssueSeverity::Error)
                .count(),
            1
        );
    }
}
