//! The panels, one at a time: which of them is being set (the chosen one
//! is shown, the others come on a press), what stands on it, and its
//! settings; and beside the choice, a new one from a model, a copy of
//! this one, and taking this one away.
//!
//! These are the entries of `desktop.shell.panels`. A panel is known by
//! its id (to a command, and to the shell from one start to the next), so
//! a new one gets an id no other has.

use crate::document::Document;
use crate::form::Form;
use egui::{RichText, Ui};
use gevrek_model::settings::{
    PanelAlign, PanelEdge, PanelFit, PanelModule, PanelProfile, PanelStyle, ShellConfig,
};
use gevrek_ui::{theme, widgets};
use serde_json::Value;

/// Where the panels are in the document.
pub const PANELS: &str = "/desktop/shell/panels";

/// What the form leaves out of a panel: its sides are placed by dragging,
/// in the card above the form.
const BY_DRAGGING: &[&str] = &["left", "centre", "right"];

/// What a new panel can be made as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// A strip along the top with everything on it.
    Bar,
    /// A short one at the bottom, as wide as what it shows: the windows.
    Dock,
    /// A narrow one down the left side.
    Rail,
}

impl Model {
    pub const ALL: [Model; 3] = [Model::Bar, Model::Dock, Model::Rail];

    pub fn label(self) -> &'static str {
        match self {
            Model::Bar => "A bar along the top",
            Model::Dock => "A dock at the bottom",
            Model::Rail => "A rail down the left side",
        }
    }

    /// The panel it makes (its id is made one no other has when it is
    /// added: `ShellConfig::add_panel`).
    pub fn panel(self) -> PanelProfile {
        match self {
            Model::Bar => PanelProfile {
                id: "bar".to_string(),
                ..PanelProfile::default()
            },
            Model::Dock => PanelProfile {
                id: "dock".to_string(),
                edge: PanelEdge::Bottom,
                style: PanelStyle::Bar,
                fit: PanelFit::Content,
                align: PanelAlign::Centre,
                floating: true,
                height: 46,
                margin: 8,
                corner_radius: 16,
                left: vec![PanelModule::Launcher],
                centre: vec![PanelModule::Taskbar],
                right: Vec::new(),
                task_width: 0,
                tasks_of_all_workspaces: true,
                ..PanelProfile::default()
            },
            Model::Rail => PanelProfile {
                id: "rail".to_string(),
                edge: PanelEdge::Left,
                height: 40,
                margin: 0,
                corner_radius: 0,
                left: vec![PanelModule::Launcher, PanelModule::Workspaces],
                centre: Vec::new(),
                right: vec![PanelModule::Sound, PanelModule::Battery, PanelModule::Clock],
                ..PanelProfile::default()
            },
        }
    }
}

/// What the page changes about the list itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Add(Model),
    /// A copy of the panel at this place, after it.
    Copy(usize),
    Remove(usize),
}

/// The panels after a change, and the place of the one to show then.
/// (`None`: the document's panels cannot be read as panels, and nothing
/// is changed.)
pub fn changed(panels: &Value, change: &Change) -> Option<(Value, usize)> {
    let read: Vec<PanelProfile> = serde_json::from_value(panels.clone()).ok()?;
    let mut shell = ShellConfig {
        panels: read,
        ..ShellConfig::default()
    };
    let shown = match change {
        Change::Add(model) => {
            shell.add_panel(model.panel());
            shell.panels.len() - 1
        }
        Change::Copy(index) => {
            let mut copy = shell.panels.get(*index)?.clone();
            copy.id = shell.free_panel_id(&copy.id);
            if !copy.name.trim().is_empty() {
                copy.name = format!("{} (copy)", copy.name.trim());
            }
            shell.panels.insert(index + 1, copy);
            index + 1
        }
        Change::Remove(index) => {
            if *index >= shell.panels.len() {
                return None;
            }
            shell.panels.remove(*index);
            index.saturating_sub(1)
        }
    };
    Some((serde_json::to_value(&shell.panels).ok()?, shown))
}

/// One panel in a line of the list: its name, and where it is.
pub fn line(panel: &Value) -> String {
    let said = |key: &str| panel[key].as_str().unwrap_or_default().trim().to_string();
    let name = match said("name") {
        name if name.is_empty() => said("id"),
        name => name,
    };
    let edge = match said("edge").as_str() {
        "bottom" => "at the bottom",
        "left" => "down the left",
        "right" => "down the right",
        _ => "at the top",
    };
    let on = match said("on").as_str() {
        "primary" => "the primary display".to_string(),
        "only" => {
            let named: Vec<&str> = panel["only_on"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if named.is_empty() {
                "no display".to_string()
            } else {
                named.join(", ")
            }
        }
        _ => "every display".to_string(),
    };
    let off = if panel["enabled"].as_bool() == Some(false) {
        "  ·  off"
    } else {
        ""
    };
    format!("{name}  ·  {edge}, {on}{off}")
}

/// The page's own memory: which panel is being set.
#[derive(Debug, Default)]
pub struct PanelsPage {
    shown: usize,
}

impl PanelsPage {
    /// The place in the document of the panel that is being set.
    pub fn pointer(&self) -> String {
        format!("{PANELS}/{}", self.shown)
    }

    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, highlight: Option<&str>) {
        let palette = theme::pal(ui.ctx());
        let panels: Vec<Value> = document
            .get(PANELS)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.shown = self.shown.min(panels.len().saturating_sub(1));
        let mut change: Option<Change> = None;
        widgets::card(ui, "Panels", |ui| {
            ui.horizontal_wrapped(|ui| {
                if panels.is_empty() {
                    ui.label(RichText::new("There is no panel.").color(palette.muted));
                } else {
                    // The one that is being set; the others on a press.
                    egui::ComboBox::from_id_salt("the panel being set")
                        .selected_text(line(&panels[self.shown]))
                        .width(320.0)
                        .show_ui(ui, |ui| {
                            for (index, panel) in panels.iter().enumerate() {
                                ui.selectable_value(&mut self.shown, index, line(panel));
                            }
                        });
                }
                ui.menu_button("Add a panel", |ui| {
                    for model in Model::ALL {
                        if ui.button(model.label()).clicked() {
                            change = Some(Change::Add(model));
                            ui.close();
                        }
                    }
                });
                if !panels.is_empty() {
                    if widgets::secondary_button(ui, "Copy this one").clicked() {
                        change = Some(Change::Copy(self.shown));
                    }
                    if widgets::secondary_button(ui, "Remove this one").clicked() {
                        change = Some(Change::Remove(self.shown));
                    }
                }
            });
            if panels.len() > 1 {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(
                        "Two panels at one edge of a display lie side by side, the first of them at the edge.",
                    )
                    .size(12.0)
                    .color(palette.muted),
                );
            }
        });
        if let Some(change) = change {
            if let Some((after, shown)) = changed(&Value::Array(panels.clone()), &change) {
                document.set(PANELS, after);
                self.shown = shown;
            }
            return;
        }
        if panels.is_empty() {
            return;
        }
        let pointer = self.pointer();
        widgets::card(ui, "What stands on it", |ui| {
            crate::panel_modules::show(ui, document, &pointer);
        });
        widgets::card(ui, "This panel", |ui| {
            let mut form = Form::new(document);
            form.highlight = highlight;
            form.fields(ui, &pointer, BY_DRAGGING);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panels() -> Value {
        serde_json::to_value(ShellConfig::default().panels).unwrap()
    }

    fn ids(panels: &Value) -> Vec<String> {
        panels
            .as_array()
            .unwrap()
            .iter()
            .map(|panel| panel["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_panel_is_added_copied_and_taken_away() {
        let (two, shown) = changed(&panels(), &Change::Add(Model::Rail)).expect("added");
        assert_eq!(ids(&two), ["main", "rail"]);
        assert_eq!(shown, 1);
        assert_eq!(two[1]["edge"], "left");
        // A second of the same model has an id of its own.
        let (three, _) = changed(&two, &Change::Add(Model::Rail)).expect("added");
        assert_eq!(ids(&three), ["main", "rail", "rail-2"]);
        // A copy stands after what it is a copy of, and is shown.
        let (four, shown) = changed(&three, &Change::Copy(0)).expect("copied");
        assert_eq!(ids(&four), ["main", "main-2", "rail", "rail-2"]);
        assert_eq!(shown, 1);
        assert_eq!(four[1]["right"], four[0]["right"]);
        // Taken away: the one before it is shown.
        let (three, shown) = changed(&four, &Change::Remove(2)).expect("removed");
        assert_eq!(ids(&three), ["main", "main-2", "rail-2"]);
        assert_eq!(shown, 1);
        // The last one can go too: no panel is no panel.
        let (none, shown) = changed(&panels(), &Change::Remove(0)).expect("removed");
        assert_eq!(none, serde_json::json!([]));
        assert_eq!(shown, 0);
        // What is not there is not taken away, and what cannot be read
        // is left as it is.
        assert_eq!(changed(&panels(), &Change::Remove(5)), None);
        assert_eq!(
            changed(&serde_json::json!("panels"), &Change::Add(Model::Bar)),
            None
        );
    }

    #[test]
    fn every_model_is_a_panel_the_configuration_takes() {
        let mut shell = ShellConfig::default();
        for model in Model::ALL {
            shell.add_panel(model.panel());
        }
        assert_eq!(shell.panels.len(), 4);
        assert!(shell.problems().is_empty(), "{:?}", shell.problems());
        let mut config = gevrek_model::config::Config::default();
        config.desktop.shell = shell;
        assert!(config.validate().is_empty(), "{:?}", config.validate());
    }

    #[test]
    fn a_line_says_which_panel_it_is_and_where() {
        let all = panels();
        assert_eq!(line(&all[0]), "main  ·  at the top, every display");
        let rail = serde_json::json!({
            "id": "rail", "name": "Side", "edge": "left", "on": "only",
            "only_on": ["DP-2", "DP-3"], "enabled": false
        });
        assert_eq!(line(&rail), "Side  ·  down the left, DP-2, DP-3  ·  off");
        let primary = serde_json::json!({"id": "dock", "edge": "bottom", "on": "primary"});
        assert_eq!(
            line(&primary),
            "dock  ·  at the bottom, the primary display"
        );
    }
}
