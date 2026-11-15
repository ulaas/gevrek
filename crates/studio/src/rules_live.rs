//! Which of the windows that are open each rule holds for now: what the
//! window manager says of every window (the rules it matched), turned
//! round to a list by rule. On the rules page, so that a rule that is
//! being written can be seen to catch what it is meant for.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_ui::theme;
use serde_json::Value;

/// The windows of the state, each with its title (its program where it
/// has no title) and the names of the rules it matched.
fn windows(state: &Value) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            for window in workspace["detailed_windows"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if window["is_border_window"].as_bool().unwrap_or(false) {
                    continue;
                }
                let title = window["title"].as_str().unwrap_or("").trim();
                let class = window["class"].as_str().unwrap_or("").trim();
                let name = if title.is_empty() { class } else { title };
                let rules = window["matched_rules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|rule| rule.as_str().map(str::to_string))
                    .collect();
                found.push((name.to_string(), rules));
            }
        }
    }
    found
}

/// For each rule of `names`, in their order, the windows it holds for
/// now. A rule that is named twice is one line.
pub fn by_rule(names: &[String], state: &Value) -> Vec<(String, Vec<String>)> {
    let open = windows(state);
    let mut seen = std::collections::HashSet::new();
    names
        .iter()
        .filter(|name| !name.trim().is_empty() && seen.insert(name.as_str()))
        .map(|name| {
            let held: Vec<String> = open
                .iter()
                .filter(|(_, rules)| rules.iter().any(|rule| rule == name))
                .map(|(window, _)| window.clone())
                .collect();
            (name.clone(), held)
        })
        .collect()
}

/// The names of the rules the document has.
fn rule_names(document: &Document) -> Vec<String> {
    document
        .get("/desktop/window_rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rule| rule["name"].as_str().map(str::to_string))
        .collect()
}

/// Draw the list. `state`: the running window manager's, where there is one.
pub fn show(ui: &mut Ui, document: &Document, state: Option<&Value>) {
    let palette = theme::pal(ui.ctx());
    let Some(state) = state else {
        ui.label(
            RichText::new("No window manager is running to ask which windows a rule holds for.")
                .color(palette.muted),
        );
        return;
    };
    let names = rule_names(document);
    if names.is_empty() {
        ui.label(RichText::new("There are no rules yet.").color(palette.muted));
        return;
    }
    ui.label(
        RichText::new(
            "By the rules the window manager runs now: one that was changed here holds \
             for these after it is applied.",
        )
        .color(palette.muted),
    );
    ui.add_space(4.0);
    for (rule, held) in by_rule(&names, state) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&rule).strong());
            if held.is_empty() {
                ui.label(RichText::new("no window that is open").color(palette.faint));
            } else {
                for window in held.iter().take(8) {
                    let short: String = window.chars().take(40).collect();
                    ui.label(
                        RichText::new(short)
                            .background_color(palette.control)
                            .color(palette.text),
                    );
                }
                if held.len() > 8 {
                    ui.label(
                        RichText::new(format!("and {} more", held.len() - 8)).color(palette.muted),
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_windows_are_listed_under_the_rules_they_matched() {
        let window = |title: &str, class: &str, rules: &[&str]| json!({"title": title, "class": class, "matched_rules": rules});
        let state = json!({"monitors": [
            {"workspaces": [
                {"detailed_windows": [
                    window("Notes", "editor", &["Notes float"]),
                    window("", "mpv", &["Video on top", "Notes float"]),
                    {"title": "border", "is_border_window": true, "matched_rules": ["Notes float"]},
                ]},
                {"detailed_windows": [window("Terminal", "foot", &[])]},
            ]},
            {"workspaces": [{"detailed_windows": [window("Film", "mpv", &["Video on top"])]}]},
        ]});
        let names: Vec<String> = ["Notes float", "Never", "Video on top", "Notes float", " "]
            .iter()
            .map(|name| name.to_string())
            .collect();
        let found = by_rule(&names, &state);
        assert_eq!(
            found,
            [
                (
                    "Notes float".to_string(),
                    vec!["Notes".to_string(), "mpv".to_string()]
                ),
                ("Never".to_string(), vec![]),
                (
                    "Video on top".to_string(),
                    vec!["mpv".to_string(), "Film".to_string()]
                ),
            ]
        );
        assert!(by_rule(&names, &json!({}))
            .iter()
            .all(|(_, held)| held.is_empty()));
    }
}
