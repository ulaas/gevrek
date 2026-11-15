//! What two kinds of node have beside the form's fields.
//!
//! - A node that starts at a key (`OnKeyPress`): its keys are read as a
//!   chord, and recorded by pressing them, as a binding's are.
//! - A node that says which windows it is for (a pattern, what it is
//!   compared with, and how): the open windows it holds for now are
//!   named under it, so that a pattern is seen to catch what is meant.

use gevrek_engine::rules::{MatchStrategy, WindowRule};
use gevrek_model::graph::{NodeType, RuleMatchKind, RuleMatchStrategy};
use serde_json::Value;

/// The keys of a key node as they are read: `Super+Shift+K`.
pub fn chord(key: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (held, name) in [
        (win, "Super"),
        (ctrl, "Ctrl"),
        (alt, "Alt"),
        (shift, "Shift"),
    ] {
        if held {
            parts.push(name);
        }
    }
    let key = key.trim();
    parts.push(if key.is_empty() { "no key yet" } else { key });
    parts.join("+")
}

/// The pattern of a node that is for certain windows: what is compared,
/// how, and with what. `None` for any other node.
pub fn pattern(node: &NodeType) -> Option<(&RuleMatchKind, &RuleMatchStrategy, &str)> {
    match node {
        NodeType::CreateFloatRule {
            match_kind,
            match_strategy,
            pattern,
            ..
        }
        | NodeType::CreateIgnoreRule {
            match_kind,
            match_strategy,
            pattern,
            ..
        }
        | NodeType::CreateManageRule {
            match_kind,
            match_strategy,
            pattern,
            ..
        }
        | NodeType::CreateRule {
            match_kind,
            match_strategy,
            pattern,
        } => Some((match_kind, match_strategy, pattern)),
        _ => None,
    }
}

/// The open windows, as the state has them (the borders gevrek draws
/// itself are none).
pub fn open_windows(state: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            for window in workspace["detailed_windows"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !window["is_border_window"].as_bool().unwrap_or(false) {
                    found.push(window.clone());
                }
            }
        }
    }
    found
}

/// What of a window a kind is compared with. `None` where the state
/// does not say it (a program's whole path).
fn compared<'a>(kind: &RuleMatchKind, window: &'a Value) -> Option<&'a str> {
    let field = match kind {
        RuleMatchKind::Title => "title",
        RuleMatchKind::Class => "class",
        RuleMatchKind::Exe => "exe",
        RuleMatchKind::AppId => "app_id",
        RuleMatchKind::Path => return None,
    };
    window[field].as_str()
}

/// The open windows a pattern holds for, by their titles (their class
/// where a window has no title), as the window manager compares them.
/// `None` where that cannot be said here (an empty pattern holds for
/// nothing that is meant; a path is not in the state).
pub fn held_for(
    kind: &RuleMatchKind,
    strategy: &RuleMatchStrategy,
    pattern: &str,
    windows: &[Value],
) -> Option<Vec<String>> {
    if pattern.is_empty() || matches!(kind, RuleMatchKind::Path) {
        return None;
    }
    let rule = WindowRule::new(
        pattern.to_string(),
        // (Which field is compared is decided here, not by the rule.)
        gevrek_engine::rules::MatchKind::Title,
        match strategy {
            RuleMatchStrategy::Equals => MatchStrategy::Equals,
            RuleMatchStrategy::Contains => MatchStrategy::Contains,
            RuleMatchStrategy::StartsWith => MatchStrategy::StartsWith,
            RuleMatchStrategy::EndsWith => MatchStrategy::EndsWith,
            RuleMatchStrategy::Regex => MatchStrategy::Regex,
        },
    );
    Some(
        windows
            .iter()
            .filter(|window| compared(kind, window).is_some_and(|value| rule.matches(value)))
            .map(|window| {
                let title = window["title"].as_str().unwrap_or("").trim();
                let class = window["class"].as_str().unwrap_or("").trim();
                (if title.is_empty() { class } else { title }).to_string()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_keys_of_a_node_are_read_as_a_chord() {
        assert_eq!(chord("K", false, false, true, true), "Super+Shift+K");
        assert_eq!(chord("F5", true, true, false, false), "Ctrl+Alt+F5");
        assert_eq!(chord("RETURN", false, false, false, false), "RETURN");
        assert_eq!(chord(" ", false, false, false, true), "Super+no key yet");
    }

    #[test]
    fn the_open_windows_a_pattern_holds_for_are_named() {
        let state = json!({"monitors": [{"workspaces": [
            {"detailed_windows": [
                {"title": "notes.txt - Editor", "class": "editor", "exe": "editor", "app_id": "org.editor"},
                {"title": "", "class": "border", "is_border_window": true},
            ]},
            {"detailed_windows": [
                {"title": "Terminal", "class": "term", "exe": "term", "app_id": "term"},
                {"title": "", "class": "editor", "exe": "editor", "app_id": "org.editor"},
            ]},
        ]}]});
        let windows = open_windows(&state);
        assert_eq!(windows.len(), 3, "the border is no window");
        let held = |kind, strategy, pattern: &str| held_for(&kind, &strategy, pattern, &windows);
        assert_eq!(
            held(RuleMatchKind::Class, RuleMatchStrategy::Equals, "editor"),
            Some(vec!["notes.txt - Editor".to_string(), "editor".to_string()])
        );
        assert_eq!(
            held(RuleMatchKind::Title, RuleMatchStrategy::EndsWith, "Editor"),
            Some(vec!["notes.txt - Editor".to_string()])
        );
        assert_eq!(
            held(RuleMatchKind::AppId, RuleMatchStrategy::Regex, "^org\\."),
            Some(vec!["notes.txt - Editor".to_string(), "editor".to_string()])
        );
        assert_eq!(
            held(
                RuleMatchKind::Title,
                RuleMatchStrategy::Contains,
                "nothing like it"
            ),
            Some(vec![])
        );
        // A pattern that is not one the window manager can read holds for none.
        assert_eq!(
            held(RuleMatchKind::Title, RuleMatchStrategy::Regex, "("),
            Some(vec![])
        );
        // What cannot be said here.
        assert_eq!(
            held(RuleMatchKind::Title, RuleMatchStrategy::Contains, ""),
            None
        );
        assert_eq!(
            held(RuleMatchKind::Path, RuleMatchStrategy::Contains, "bin"),
            None
        );
    }

    #[test]
    fn the_nodes_that_are_for_certain_windows_have_a_pattern() {
        let node = NodeType::CreateRule {
            pattern: "term".to_string(),
            match_kind: RuleMatchKind::Class,
            match_strategy: RuleMatchStrategy::Equals,
        };
        let (kind, strategy, text) = pattern(&node).expect("a pattern");
        assert_eq!(
            (kind, strategy, text),
            (&RuleMatchKind::Class, &RuleMatchStrategy::Equals, "term")
        );
        assert!(pattern(&NodeType::OnStartup).is_none());
    }
}
