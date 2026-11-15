//! The colours of the sheet, all made from the palette of the window, so
//! that a wire is as easy to follow in a light palette as in a dark one.
//!
//! A kind of pin has a hue (the palette's own: its blue, its green, ...),
//! and that hue is then moved towards the text colour until it stands out
//! enough from both the sheet and a node's body. A test goes through
//! every palette and every kind of pin.

use egui::Color32;
use gevrek_model::graph::PinType;
use gevrek_ui::theme::{mix, readable, Palette};

/// What a line or a dot must have against what it lies on (the measure
/// the accessibility guidelines give for parts of a drawing).
pub const LINE_CONTRAST: f32 = 3.0;

/// The sheet the nodes lie on: a little deeper than the page.
pub fn sheet(palette: &Palette) -> Color32 {
    mix(
        palette.bg,
        Color32::BLACK,
        if palette.dark { 0.16 } else { 0.035 },
    )
}

pub fn body(palette: &Palette) -> Color32 {
    palette.surface
}

pub fn body_selected(palette: &Palette) -> Color32 {
    mix(palette.surface, palette.accent, 0.10)
}

/// A colour that is seen on the sheet and on a node.
fn seen(palette: &Palette, hue: Color32) -> Color32 {
    readable(
        readable(hue, sheet(palette), LINE_CONTRAST),
        body(palette),
        LINE_CONTRAST,
    )
}

/// The colour of a pin and of the wire that leaves it. Kinds that go
/// together share a hue: a list has the colour of its items (and is drawn
/// as a square, see [`is_list`]).
pub fn pin(palette: &Palette, pin_type: &PinType) -> Color32 {
    let p = palette;
    let hue = match pin_type {
        PinType::Execution => p.text,
        PinType::WindowHandle | PinType::WindowList => p.blue,
        PinType::Number => p.ok,
        PinType::String => p.purple,
        PinType::Bool => p.error,
        PinType::Color => p.warn,
        PinType::BorderStyle => mix(p.blue, p.text, 0.5),
        PinType::Rule | PinType::RuleList | PinType::WindowRule => mix(p.warn, p.error, 0.5),
        PinType::MatchKind | PinType::MatchStrategy => mix(p.purple, p.error, 0.5),
        PinType::Monitor | PinType::MonitorList => mix(p.ok, p.blue, 0.5),
        PinType::Layout => mix(p.warn, p.ok, 0.5),
        PinType::List | PinType::Item => p.muted,
    };
    seen(palette, hue)
}

/// Whether a pin carries many things: it is drawn as a square.
pub fn is_list(pin_type: &PinType) -> bool {
    matches!(
        pin_type,
        PinType::WindowList | PinType::MonitorList | PinType::RuleList | PinType::List
    )
}

/// What a kind of pin is called in the window.
pub fn pin_name(pin_type: &PinType) -> &'static str {
    match pin_type {
        PinType::Execution => "the order of things",
        PinType::WindowHandle => "a window",
        PinType::WindowList => "windows",
        PinType::Number => "a number",
        PinType::String => "a text",
        PinType::Bool => "yes or no",
        PinType::Color => "a colour",
        PinType::BorderStyle => "a border style",
        PinType::WindowRule | PinType::Rule => "a rule",
        PinType::MatchKind => "what a rule looks at",
        PinType::MatchStrategy => "how a rule compares",
        PinType::Monitor => "a display",
        PinType::MonitorList => "displays",
        PinType::Layout => "a layout",
        PinType::RuleList => "rules",
        PinType::List => "a list of anything",
        PinType::Item => "one of anything",
    }
}

/// The colour of a group of nodes: the line at the top of a node and the
/// dot in the list of nodes.
pub fn group(palette: &Palette, group: &str) -> Color32 {
    let p = palette;
    let hue = match group {
        "Events" => p.error,
        "Actions" => p.blue,
        "Focus" => mix(p.blue, p.text, 0.35),
        "Layout" => mix(p.blue, p.purple, 0.5),
        "Rules" => mix(p.warn, p.error, 0.5),
        "Conditions" => p.warn,
        "Helpers" | "Data" => mix(p.ok, p.blue, 0.5),
        "Workspaces" | "Workspace navigation" => p.purple,
        "Monitors" | "Scratchpads" => p.ok,
        "Window appearance" => mix(p.purple, p.error, 0.5),
        "Math" | "Conversion" => mix(p.ok, p.warn, 0.4),
        _ => p.muted,
    };
    seen(palette, hue)
}

/// The header of a node: its body with some of the group's colour.
pub fn header(palette: &Palette, group_colour: Color32) -> Color32 {
    mix(body(palette), group_colour, 0.20)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_ui::theme::contrast;
    use gevrek_ui::Preset;

    const KINDS: [PinType; 18] = [
        PinType::Execution,
        PinType::WindowHandle,
        PinType::WindowList,
        PinType::Number,
        PinType::String,
        PinType::Bool,
        PinType::Color,
        PinType::BorderStyle,
        PinType::WindowRule,
        PinType::MatchKind,
        PinType::MatchStrategy,
        PinType::Monitor,
        PinType::MonitorList,
        PinType::Layout,
        PinType::Rule,
        PinType::RuleList,
        PinType::List,
        PinType::Item,
    ];

    fn palettes() -> Vec<(String, Palette)> {
        let mut all = Vec::new();
        for preset in Preset::ALL {
            for system in [egui::Theme::Dark, egui::Theme::Light] {
                all.push((
                    format!("{} ({system:?})", preset.name()),
                    preset.palette(system),
                ));
            }
        }
        all
    }

    #[test]
    fn wires_and_pins_are_seen_in_every_palette() {
        for (name, palette) in palettes() {
            for kind in &KINDS {
                let colour = pin(&palette, kind);
                for (on, ground) in [("the sheet", sheet(&palette)), ("a node", body(&palette))] {
                    let found = contrast(colour, ground);
                    assert!(
                        found >= LINE_CONTRAST,
                        "{name}: {kind:?} on {on} has contrast {found:.2}"
                    );
                }
                // A selected node's body has a little of the accent in it.
                let found = contrast(colour, body_selected(&palette));
                assert!(
                    found >= LINE_CONTRAST - 0.5,
                    "{name}: {kind:?} on a selected node has contrast {found:.2}"
                );
            }
        }
    }

    #[test]
    fn a_nodes_name_is_read_on_its_header_in_every_palette() {
        for (name, palette) in palettes() {
            for (group_name, _) in gevrek_model::graph::node_catalog() {
                let colour = group(&palette, group_name);
                assert!(
                    contrast(colour, sheet(&palette)) >= LINE_CONTRAST,
                    "{name}: the line of {group_name}"
                );
                let found = contrast(palette.text, header(&palette, colour));
                assert!(
                    found >= 4.5,
                    "{name}: text on the header of {group_name} has contrast {found:.2}"
                );
            }
            let found = contrast(palette.muted, body(&palette));
            assert!(
                found >= LINE_CONTRAST,
                "{name}: a pin's name on a node has contrast {found:.2}"
            );
        }
    }

    #[test]
    fn every_kind_of_pin_has_a_name() {
        for kind in &KINDS {
            assert!(!pin_name(kind).is_empty());
        }
        assert!(is_list(&PinType::WindowList) && !is_list(&PinType::WindowHandle));
    }
}
