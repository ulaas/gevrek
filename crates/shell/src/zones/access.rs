//! The zones as a screen reader is told them. What is drawn is paint, and
//! paint says nothing: so each zone that is shown, and each item that is
//! seen in it, is also a node of the surface's accessibility tree, with
//! what it is, what it is called, and how it stands (chosen, with the
//! keyboard, open or rolled up, which one of how many).
//!
//! What is said comes from the core (`gevrek_zones::access::tree`), so a
//! screen reader, a key and `gevrek zones` speak of the same zones in the
//! same words; where each thing is comes from the layout that was drawn.
//! A zone is a list and its items are the list's options, which is what
//! screen readers have words for: "Papers, list, 3 items, locked",
//! "Budget.xlsx, 2 of 8, selected". The buttons at the end of a zone's
//! title are buttons of its list, each called by what pressing it does
//! now ("Roll up", "Show as a list"), and pressed by the screen reader
//! they do it.
//!
//! Nothing of this is done while no screen reader asks: the host turns
//! the tree on when one does, and until then this returns at its first
//! line.
//!
//! The keyboard is told here too, as far as the tree goes: the node that
//! has it is the zone or the item the core says has it, while the
//! surface has the keyboard. That names a node of this surface's own
//! tree; it asks the system for nothing, and a surface without the
//! keyboard is left without it.

use super::view::{ItemBox, Layout};
use super::Zones;
use egui::accesskit::{Action, Role};
use gevrek_zones::access::AccessNode;
use gevrek_zones::core::Command;
use gevrek_zones::{DisplayId, ItemId, ZoneId};

/// What a node of the tree stands for, to know what was meant when a
/// screen reader asks for something by its node.
#[derive(Debug, Clone, PartialEq)]
pub enum Spoken {
    Zone(ZoneId),
    Item(ItemId),
    /// A button of a zone's title.
    Button(ZoneId, super::view::Button),
}

/// What is said of one item.
fn say_item(node: &mut egui::accesskit::Node, said: &AccessNode) {
    node.set_role(Role::ListBoxOption);
    node.set_label(said.name.clone());
    node.set_selected(said.selected);
    if said.position > 0 {
        node.set_position_in_set(said.position as usize);
        node.set_size_of_set(said.of as usize);
    }
    node.add_action(Action::Focus);
    node.add_action(Action::Click);
}

/// What is said of a zone, or of a display's loose items.
fn say_list(node: &mut egui::accesskit::Node, said: &AccessNode) {
    node.set_role(Role::ListBox);
    node.set_label(said.name.clone());
    if !said.description.is_empty() {
        node.set_description(said.description.clone());
    }
    if let Some(typed) = &said.value {
        node.set_value(typed.clone());
    }
    if let Some(open) = said.expanded {
        node.set_expanded(open);
    }
    if said.position > 0 {
        node.set_position_in_set(said.position as usize);
        node.set_size_of_set(said.of as usize);
    }
    node.add_action(Action::Focus);
}

/// Tell the screen reader what one display's surface shows, and do what
/// it asked for since the last frame. `layout` is what was just drawn.
pub fn speak(zones: &mut Zones, ui: &mut egui::Ui, layout: &Layout, display: &DisplayId) {
    // No screen reader asks: nothing is made, and nothing kept.
    let root = ui.unique_id();
    if ui.ctx().accesskit_node_builder(root, |_| ()).is_none() {
        zones.said = None;
        return;
    }
    let Some(desk) = layout.desks.iter().find(|desk| &desk.display == display) else {
        return;
    };
    hear(zones, ui, layout);
    // What there is to say, made anew only when the zones changed.
    let said = match zones.said.take() {
        Some(said) => said,
        None => gevrek_zones::access::tree(&zones.core.scene(super::now())),
    };
    let shift = -desk.rect.min.to_vec2();
    let has_keyboard = ui.input(|input| input.focused);
    let mut with_keyboard = None;
    let mut ids = Vec::new();
    for list in said.children.iter().filter(|list| !list.hidden) {
        // A zone that is drawn here, or this display's loose items.
        let (rect, items, clip, zone): (egui::Rect, &[ItemBox], egui::Rect, Option<&ZoneId>) =
            if let Some(id) = list.id.strip_prefix("zone:") {
                let Some(shown) = layout
                    .zones
                    .iter()
                    .find(|zone| zone.zone.as_str() == id && zone.rect.intersects(desk.rect))
                else {
                    continue;
                };
                (shown.rect, &shown.items, shown.content, Some(&shown.zone))
            } else if list.id.strip_prefix("loose:") == Some(display.as_str()) {
                (desk.area, &desk.items, desk.rect, None)
            } else {
                continue;
            };
        let builder = egui::UiBuilder::new()
            .id_salt(list.id.as_str())
            .max_rect(rect.translate(shift));
        ui.scope_builder(builder, |ui| {
            let id = ui.unique_id();
            ui.memory_mut(|memory| memory.interested_in_focus(id, ui.layer_id()));
            ui.ctx()
                .accesskit_node_builder(id, |node| say_list(node, list));
            if let Some(zone) = zone {
                ids.push((id.accesskit_id(), Spoken::Zone(zone.clone())));
            }
            if list.focused {
                with_keyboard = Some(id);
            }
            // The buttons of its title: what each does now, to be pressed.
            if let Some(shown) = zone.and_then(|zone| layout.zone(zone)) {
                for placed in &shown.buttons {
                    let response = ui.interact(
                        placed.rect.translate(shift),
                        egui::Id::unique((list.id.as_str(), "title-button", placed.button as u8)),
                        egui::Sense::focusable_noninteractive(),
                    );
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_role(Role::Button);
                        node.set_label(super::view::button_says(shown, placed.button).to_string());
                        node.add_action(Action::Click);
                    });
                    ids.push((
                        response.id.accesskit_id(),
                        Spoken::Button(shown.zone.clone(), placed.button),
                    ));
                }
            }
            for (index, child) in list
                .children
                .iter()
                .enumerate()
                .filter(|(_, child)| !child.hidden)
            {
                // (In the order they are shown; looked up where that
                // does not hold, as while a zone's items are changing.)
                let fits = |shown: &&ItemBox| child.id.ends_with(shown.item.as_str());
                let Some(shown) = items
                    .get(index)
                    .filter(fits)
                    .or_else(|| items.iter().find(fits))
                else {
                    continue;
                };
                // What is scrolled out of sight is not there to be pointed at.
                if !clip.intersects(shown.rect) {
                    continue;
                }
                let response = ui.interact(
                    shown.rect.translate(shift),
                    egui::Id::unique(child.id.as_str()),
                    egui::Sense::focusable_noninteractive(),
                );
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| say_item(node, child));
                ids.push((response.id.accesskit_id(), Spoken::Item(shown.item.clone())));
                if child.focused {
                    with_keyboard = Some(response.id);
                }
            }
        });
    }
    // The node that has the keyboard, while the surface has it: the keys
    // themselves are read from the surface, so the toolkit is kept from
    // walking its own way among the nodes with them.
    if let (true, Some(id)) = (has_keyboard, with_keyboard) {
        let all = egui::EventFilter {
            tab: true,
            horizontal_arrows: true,
            vertical_arrows: true,
            escape: true,
        };
        ui.memory_mut(|memory| {
            if memory.focused() != Some(id) {
                memory.request_focus(id);
            }
            memory.set_focus_lock_filter(id, all);
        });
    }
    zones.said = Some(said);
    zones.said_ids.retain(|(_, of), _| of != display);
    zones.said_ids.extend(
        ids.into_iter()
            .map(|(id, spoken)| ((id, display.clone()), spoken)),
    );
}

/// Do what the screen reader asked for by a node: go to it, open it, or
/// press it. `layout` is what was just drawn (the zones' own is not
/// theirs while it is drawn from).
fn hear(zones: &mut Zones, ui: &egui::Ui, layout: &Layout) {
    let asked: Vec<(egui::accesskit::NodeId, Action)> = ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::AccessKitActionRequest(request) => {
                    Some((request.target_node, request.action))
                }
                _ => None,
            })
            .collect()
    });
    for (target, action) in asked {
        let spoken = zones
            .said_ids
            .iter()
            .find(|((id, _), _)| *id == target)
            .map(|(_, spoken)| spoken.clone());
        let command = match (spoken, action) {
            (Some(Spoken::Button(zone, button)), Action::Click) => {
                let acts = super::gesture::button_pressed(layout, &zone, button);
                zones.perform(acts);
                continue;
            }
            (Some(Spoken::Item(item)), Action::Focus) => {
                // Where the core walks (a zone that is shown), the
                // keyboard goes to it; a loose item is the chosen one.
                let went = zones.command(Command::FocusItem {
                    item: Some(item.clone()),
                });
                if gevrek_zones::ipc::Answer::of(&went).ok {
                    continue;
                }
                Command::Select {
                    items: vec![item],
                    mode: gevrek_zones::focus::SelectMode::Replace,
                }
            }
            (Some(Spoken::Item(item)), Action::Click) => Command::Open {
                items: vec![item],
                with: None,
                in_place: true,
            },
            (Some(Spoken::Zone(zone)), Action::Focus | Action::Click) => {
                Command::FocusZone { zone: Some(zone) }
            }
            _ => continue,
        };
        zones.command(command);
    }
}
