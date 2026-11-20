//! Nodes that build and register window rules.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_rules_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Rule creation nodes - now support batch rule creation
            NodeType::CreateFloatRule {
                match_kind,
                match_strategy,
                pattern,
                rules,
            }
            | NodeType::CreateIgnoreRule {
                match_kind,
                match_strategy,
                pattern,
                rules,
            }
            | NodeType::CreateManageRule {
                match_kind,
                match_strategy,
                pattern,
                rules,
            } => {
                use crate::rules::{
                    MatchKind as DwmMatchKind, MatchStrategy as DwmMatchStrategy, WindowRule,
                };

                // Helper function to convert RuleDefinition to WindowRule
                let convert_rule = |rule_def: &RuleDefinition| -> WindowRule {
                    WindowRule::new(
                        rule_def.pattern.clone(),
                        rule_def.match_kind.clone(),
                        rule_def.match_strategy.clone(),
                    )
                };

                // Determine which rules to use (priority: pin input > static rules > legacy single rule)
                let rules_to_apply: Vec<WindowRule> = if let Some(Value::RuleList(rule_list)) =
                    self.get_input_value(node_id, 1)
                {
                    // 1. Use rules from RuleList pin if connected
                    log::trace!("Using {} rules from RuleList pin", rule_list.len());
                    rule_list.iter().map(convert_rule).collect()
                } else if !rules.is_empty() {
                    // 2. Use static rules from node properties
                    log::trace!("Using {} static rules from node properties", rules.len());
                    rules.iter().map(convert_rule).collect()
                } else {
                    // 3. Fall back to legacy single-rule mode (Pattern pin + match_kind/match_strategy)
                    let pattern_str = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| pattern.clone());

                    let dwm_match_kind = match match_kind {
                        gevrek_model::graph::RuleMatchKind::Exe => DwmMatchKind::Exe,
                        gevrek_model::graph::RuleMatchKind::Class => DwmMatchKind::Class,
                        gevrek_model::graph::RuleMatchKind::Title => DwmMatchKind::Title,
                        gevrek_model::graph::RuleMatchKind::Path => DwmMatchKind::Path,
                        gevrek_model::graph::RuleMatchKind::AppId => DwmMatchKind::AppId,
                    };

                    let dwm_match_strategy = match match_strategy {
                        gevrek_model::graph::RuleMatchStrategy::Equals => DwmMatchStrategy::Equals,
                        gevrek_model::graph::RuleMatchStrategy::Contains => {
                            DwmMatchStrategy::Contains
                        }
                        gevrek_model::graph::RuleMatchStrategy::StartsWith => {
                            DwmMatchStrategy::StartsWith
                        }
                        gevrek_model::graph::RuleMatchStrategy::EndsWith => {
                            DwmMatchStrategy::EndsWith
                        }
                        gevrek_model::graph::RuleMatchStrategy::Regex => DwmMatchStrategy::Regex,
                    };

                    log::trace!(
                        "Using legacy single-rule mode with pattern '{}'",
                        pattern_str
                    );
                    vec![WindowRule::new(
                        pattern_str,
                        dwm_match_kind,
                        dwm_match_strategy,
                    )]
                };

                // Add all rules to the RuleEngine in batch
                let rule_count = rules_to_apply.len();
                if let Ok(mut engine) = self.rule_engine.lock() {
                    match self.graph.nodes.get(&node_id).map(|n| &n.node_type) {
                        Some(NodeType::CreateFloatRule { .. }) => {
                            use crate::rules::FloatRule;
                            for window_rule in rules_to_apply {
                                let float_rule = FloatRule::new(window_rule);
                                engine.add_float_rule(float_rule);
                            }
                            log::debug!("✓ Created {} float rule(s)", rule_count);
                        }
                        Some(NodeType::CreateIgnoreRule { .. }) => {
                            use crate::rules::IgnoreRule;
                            for window_rule in rules_to_apply {
                                let ignore_rule = IgnoreRule::new(window_rule);
                                engine.add_ignore_rule(ignore_rule);
                            }
                            log::debug!("✓ Created {} ignore rule(s)", rule_count);
                        }
                        Some(NodeType::CreateManageRule { .. }) => {
                            use crate::rules::ManageRule;
                            for window_rule in rules_to_apply {
                                let manage_rule = ManageRule::new(window_rule);
                                engine.add_manage_rule(manage_rule);
                            }
                            log::debug!("✓ Created {} manage rule(s)", rule_count);
                        }
                        _ => {}
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // CreateRule - Build a single Rule from pattern + match_kind + match_strategy
            NodeType::CreateRule {
                pattern: default_pattern,
                match_kind: default_kind,
                match_strategy: default_strategy,
            } => {
                // Get inputs from pins, fall back to properties if not connected
                let pattern = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| default_pattern.clone());

                // Convert RuleMatchKind to MatchKind
                let dwm_default_kind = match default_kind {
                    gevrek_model::graph::RuleMatchKind::Exe => crate::rules::MatchKind::Exe,
                    gevrek_model::graph::RuleMatchKind::Class => crate::rules::MatchKind::Class,
                    gevrek_model::graph::RuleMatchKind::Title => crate::rules::MatchKind::Title,
                    gevrek_model::graph::RuleMatchKind::Path => crate::rules::MatchKind::Path,
                    gevrek_model::graph::RuleMatchKind::AppId => crate::rules::MatchKind::AppId,
                };

                let match_kind = self
                    .get_input_value(node_id, 2)
                    .and_then(|v| {
                        if let Value::MatchKind(mk) = v {
                            Some(mk)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(dwm_default_kind);

                // Convert RuleMatchStrategy to MatchStrategy
                let dwm_default_strategy = match default_strategy {
                    gevrek_model::graph::RuleMatchStrategy::Equals => {
                        crate::rules::MatchStrategy::Equals
                    }
                    gevrek_model::graph::RuleMatchStrategy::Contains => {
                        crate::rules::MatchStrategy::Contains
                    }
                    gevrek_model::graph::RuleMatchStrategy::StartsWith => {
                        crate::rules::MatchStrategy::StartsWith
                    }
                    gevrek_model::graph::RuleMatchStrategy::EndsWith => {
                        crate::rules::MatchStrategy::EndsWith
                    }
                    gevrek_model::graph::RuleMatchStrategy::Regex => {
                        crate::rules::MatchStrategy::Regex
                    }
                };

                let match_strategy = self
                    .get_input_value(node_id, 3)
                    .and_then(|v| {
                        if let Value::MatchStrategy(ms) = v {
                            Some(ms)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(dwm_default_strategy);

                // Create RuleDefinition
                let rule_def = RuleDefinition::new(
                    pattern.clone(),
                    match_kind.clone(),
                    match_strategy.clone(),
                );

                log::trace!(
                    "Created rule: {:?} {:?} '{}'",
                    match_kind,
                    match_strategy,
                    pattern
                );

                // Set output value
                self.set_output_value(node_id, 1, Value::Rule(rule_def));

                // Trigger execution
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // CombineRules - Combine multiple Rules into a RuleList
            NodeType::CombineRules => {
                let mut rules = Vec::new();

                // Collect up to 4 rules from input pins
                for pin_index in 1..=4 {
                    if let Some(Value::Rule(rule)) = self.get_input_value(node_id, pin_index) {
                        rules.push(rule);
                    }
                }

                log::trace!("Combined {} rules into RuleList", rules.len());

                // Set output value
                self.set_output_value(node_id, 1, Value::RuleList(rules));

                // Trigger execution
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ApplySystemFilters { enabled } => {
                if !enabled {
                    log::debug!("System filters disabled by user");
                    self.set_output_value(node_id, 1, Value::Number(0.0));
                    self.trigger_output_execution(
                        window_manager,
                        monitor_manager,
                        node_id,
                        0,
                        visited,
                    )?;
                    return Ok(NodeDone::Early);
                }

                // Add all system ignore rules via RuleEngine
                let count = if let Ok(mut engine) = self.rule_engine.lock() {
                    let before_count = engine.ignore_rules.len();
                    engine.add_system_ignore_rules();
                    let after_count = engine.ignore_rules.len();
                    let added = after_count - before_count;
                    log::debug!("✓ Applied {} system ignore rules", added);
                    added as f64
                } else {
                    log::error!("Failed to lock rule engine");
                    0.0
                };

                self.set_output_value(node_id, 1, Value::Number(count));
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Other families are run by their own function.
            _ => {}
        }
        Ok(NodeDone::Normal)
    }
}
