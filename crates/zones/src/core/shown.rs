//! What is shown how, for the moment: a rolled-up zone that is open for
//! now, tabs brought round, what stays when the rest is put away, what
//! is covered from view, and the room tiled windows keep off.

use super::{Effect, Outcome, ZonesCore, ZonesEvent};
use crate::config::{ExpandOn, WhenShared, ZoneDefinition};
use crate::geometry::work_area;
use crate::ids::{ItemId, ZoneId};
use crate::layout;
use crate::reserve::{self, ReservedRegion};
use std::collections::BTreeSet;

impl ZonesCore {
    // ---- roll-up ------------------------------------------------------

    /// Whether a rolled-up zone is open for now.
    pub fn is_expanded(&self, zone: &ZoneId) -> bool {
        self.view.expanded.contains(zone)
    }

    /// Whether only the title of a zone is drawn: it is rolled up and not
    /// open for now.
    pub(super) fn is_rolled(&self, zone: &ZoneId) -> bool {
        self.config.zone(zone).is_some_and(|zone| zone.rolled_up) && !self.is_expanded(zone)
    }

    /// What opens a zone while it is rolled up: what it says itself, or
    /// what is set for all.
    pub(super) fn expand_on(&self, definition: &ZoneDefinition) -> ExpandOn {
        definition.expand_on.unwrap_or(self.config.roll_up.expand_on)
    }

    /// When the core wants a `Command::Tick` at the latest: a zone is
    /// waiting to open or to close. None while nothing waits.
    pub fn wake_at_ms(&self) -> Option<u64> {
        let opening = self.view.opening.as_ref().map(|(_, due)| *due);
        opening.into_iter().chain(self.view.closing.values().copied()).min()
    }

    /// Open a rolled-up zone for now, or close it; with the tabs it
    /// shares its place with, which are one thing on the desktop.
    pub(super) fn set_expanded(&mut self, zone: &ZoneId, expanded: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        for companion in self.companions(zone) {
            let rolled_up = self.config.zone(&companion).is_some_and(|zone| zone.rolled_up);
            let changed = if expanded && rolled_up {
                self.view.expanded.insert(companion.clone())
            } else if !expanded {
                self.view.closing.remove(&companion);
                self.view.expanded.remove(&companion)
            } else {
                false
            };
            if changed {
                effects.push(Effect::Notify(ZonesEvent::ZoneExpanded { zone: companion, expanded }));
            }
        }
        if expanded && self.view.opening.as_ref().is_some_and(|(waiting, _)| self.companions(zone).contains(waiting)) {
            self.view.opening = None;
        }
        effects
    }

    pub(super) fn pointer_over(&mut self, zone: Option<ZoneId>, at_ms: u64, dragging: bool) -> Vec<Effect> {
        let over = zone.filter(|zone| self.config.zone(zone).is_some());
        let together = over.as_ref().map(|zone| self.companions(zone)).unwrap_or_default();
        self.view.pointer = over.clone();
        // What the pointer rests on opens after a moment, where zones
        // open that way; something carried over it always opens it, since
        // it could not be dropped inside otherwise.
        let opens = over.as_ref().filter(|zone| self.is_rolled(zone)).filter(|zone| {
            dragging || self.config.zone(zone).is_some_and(|zone| self.expand_on(zone) == ExpandOn::Hover)
        });
        self.view.opening = match opens {
            Some(zone) => match &self.view.opening {
                // (It has been resting there: the moment began earlier.)
                Some((waiting, due)) if together.contains(waiting) => Some((waiting.clone(), *due)),
                _ => Some((zone.clone(), at_ms + u64::from(self.config.roll_up.expand_delay_ms))),
            },
            None => None,
        };
        // What is open and left closes after a moment; coming back in
        // time keeps it open.
        let delay = u64::from(self.config.roll_up.collapse_delay_ms);
        for open in self.view.expanded.clone() {
            if together.contains(&open) {
                self.view.closing.remove(&open);
            } else {
                self.view.closing.entry(open).or_insert(at_ms + delay);
            }
        }
        self.tick(at_ms)
    }

    pub(super) fn expand(&mut self, zone: &ZoneId, expanded: Option<bool>) -> Outcome {
        let definition = self.known(zone)?;
        if !definition.rolled_up {
            return Ok(Vec::new());
        }
        let expanded = expanded.unwrap_or(!self.is_expanded(zone));
        Ok(self.set_expanded(zone, expanded))
    }

    /// Do what was waiting for this time.
    pub(super) fn tick(&mut self, at_ms: u64) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some((zone, _)) = self.view.opening.clone().filter(|(_, due)| *due <= at_ms) {
            self.view.opening = None;
            effects.extend(self.set_expanded(&zone, true));
        }
        let due: Vec<ZoneId> =
            self.view.closing.iter().filter(|(_, due)| **due <= at_ms).map(|(zone, _)| zone.clone()).collect();
        for zone in due {
            self.view.closing.remove(&zone);
            // The keyboard holds a zone open as the pointer does; it
            // closes when the keyboard leaves (`settle`).
            let held = self.view.focus.as_ref().is_some_and(|focus| self.companions(&zone).contains(&focus.zone));
            if !held && self.view.expanded.contains(&zone) {
                effects.extend(self.set_expanded(&zone, false));
            }
        }
        effects
    }

    // ---- tabs ---------------------------------------------------------

    pub(super) fn move_tab(&mut self, zone: &ZoneId, index: usize) -> Outcome {
        let group = self.known(zone)?.tab_group.clone().ok_or_else(|| format!("{zone} is no tab"))?;
        let members = self.companions(zone);
        let Some(from) = members.iter().position(|member| member == zone) else {
            return Ok(Vec::new());
        };
        let to = index.min(members.len() - 1);
        if from == to {
            return Ok(Vec::new());
        }
        // The first tab's place is the group's: whoever is first after
        // this must have that place, on every set of displays.
        let leader = members[0].clone();
        for profile in &mut self.state.profiles {
            if let Some(place) = profile.geometry.get(&leader).cloned() {
                for member in &members {
                    profile.geometry.insert(member.clone(), place.clone());
                }
            }
        }
        // The order of the tabs is the order of the zones in the
        // configuration; the zones between them stay where they are.
        let mut order = members.clone();
        let moved = order.remove(from);
        order.insert(to, moved);
        let slots: Vec<usize> = self
            .config
            .zones
            .iter()
            .enumerate()
            .filter(|(_, zone)| zone.tab_group.as_ref() == Some(&group))
            .map(|(slot, _)| slot)
            .collect();
        let mut definitions: Vec<ZoneDefinition> = order
            .iter()
            .filter_map(|member| self.config.zone(member).cloned())
            .collect();
        if definitions.len() != slots.len() {
            return Ok(Vec::new());
        }
        for slot in slots.into_iter().rev() {
            if let Some(definition) = definitions.pop() {
                self.config.zones[slot] = definition;
            }
        }
        let mut effects: Vec<Effect> = members
            .into_iter()
            .map(|member| Effect::Notify(ZonesEvent::ZoneChanged { zone: member }))
            .collect();
        effects.extend([Effect::SaveConfig, Effect::SaveState]);
        Ok(effects)
    }

    pub(super) fn next_tab(&mut self, zone: &ZoneId, backwards: bool) -> Outcome {
        self.known(zone)?.tab_group.as_ref().ok_or_else(|| format!("{zone} is no tab"))?;
        let groups = layout::tab_groups(&self.config.zones);
        let Some(group) = layout::group_of(&groups, zone) else {
            return Ok(Vec::new());
        };
        let front = self.front_of(group);
        let at = group.members.iter().position(|member| member == &front).unwrap_or(0);
        let count = group.members.len();
        let next = if backwards { (at + count - 1) % count } else { (at + 1) % count };
        let next = group.members[next].clone();
        let mut effects = self.activate_tab(&next)?;
        // The keyboard goes with the tab it was on.
        if let Some(focus) = &mut self.view.focus {
            if group.members.contains(&focus.zone) && focus.zone != next {
                focus.zone = next;
                focus.item = None;
                effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
            }
        }
        Ok(effects)
    }

    // ---- what stays when the rest is put away -------------------------

    pub(super) fn item_always_visible(&mut self, item: &ItemId, on: Option<bool>) -> Outcome {
        if self.item(item).is_none() {
            return Err(format!("{item} is not there"));
        }
        let kept = &mut self.state.always_visible_items;
        let on = on.unwrap_or(!kept.contains(item));
        let changed = if on { kept.insert(item.clone()) } else { kept.remove(item) };
        if !changed {
            return Ok(Vec::new());
        }
        let zone = self.zone_of(item).cloned();
        Ok(vec![Effect::Notify(ZonesEvent::ItemsChanged { zone }), Effect::SaveState])
    }

    /// What Escape does, the nearest thing first.
    pub(super) fn dismiss(&mut self) -> Vec<Effect> {
        if self.peeking {
            self.peeking = false;
            return vec![self.visibility()];
        }
        let focused = self.view.focus.as_ref().map(|focus| focus.zone.clone());
        if let Some(zone) = focused.filter(|zone| self.view.filters.contains_key(zone)) {
            self.view.filters.remove(&zone);
            return vec![
                Effect::Notify(ZonesEvent::FilterChanged { zone: zone.clone(), typed: String::new() }),
                Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone) }),
            ];
        }
        // (`settle` tells of the choice and of the keyboard.)
        if !self.view.selection.is_empty() {
            self.view.selection.clear();
        } else {
            self.view.focus = None;
        }
        Vec::new()
    }

    pub(super) fn open(&mut self, items: Vec<ItemId>, with: Option<String>, in_place: bool) -> Outcome {
        let mut seen = BTreeSet::new();
        let known: Vec<ItemId> = items
            .into_iter()
            .filter(|item| self.item(item).is_some() && seen.insert(item.clone()))
            .collect();
        if known.is_empty() {
            return Err("nothing of that is there to open".to_string());
        }
        if let ([only], true, None) = (known.as_slice(), in_place, &with) {
            if let Some(zone) = self.portal_with_folder(only) {
                return self.navigate(&zone, super::folders::Way::Into(only.clone()));
            }
        }
        let mut effects = vec![Effect::Notify(ZonesEvent::OpenRequested { items: known, with })];
        // Peeking was for finding something to open: it is found.
        if self.peeking {
            self.peeking = false;
            effects.push(self.visibility());
        }
        Ok(effects)
    }

    // ---- privacy ------------------------------------------------------

    fn all_covered(&self) -> bool {
        self.config.privacy.everything
            || self.view.masked
            || (self.view.screen_shared && self.config.privacy.when_shared == WhenShared::Everything)
    }

    /// Whether what a zone holds is covered from view now.
    pub fn is_masked(&self, zone: &ZoneId) -> bool {
        let private = self.config.zone(zone).is_some_and(|zone| zone.private);
        (private || self.all_covered()) && !self.view.revealed.contains(zone)
    }

    /// Whether the items that are in no zone are covered now.
    pub fn loose_masked(&self) -> bool {
        self.all_covered() && !self.view.loose_revealed
    }

    fn privacy_changed(&self) -> Vec<Effect> {
        vec![
            Effect::Notify(ZonesEvent::PrivacyChanged {
                masked: self.view.masked,
                screen_shared: self.view.screen_shared,
            }),
            Effect::Notify(ZonesEvent::LayoutChanged),
        ]
    }

    pub(super) fn mask(&mut self, masked: Option<bool>) -> Vec<Effect> {
        let masked = masked.unwrap_or(!self.view.masked);
        self.view.masked = masked;
        if masked {
            // Covering everything covers what had been asked to be seen.
            self.view.revealed.clear();
            self.view.loose_revealed = false;
        }
        self.privacy_changed()
    }

    pub(super) fn reveal(&mut self, zone: Option<ZoneId>, revealed: Option<bool>) -> Outcome {
        match zone {
            Some(zone) => {
                self.known(&zone)?;
                let revealed = revealed.unwrap_or(!self.view.revealed.contains(&zone));
                let changed =
                    if revealed { self.view.revealed.insert(zone.clone()) } else { self.view.revealed.remove(&zone) };
                Ok(if changed { vec![Effect::Notify(ZonesEvent::ZoneChanged { zone })] } else { Vec::new() })
            }
            None => {
                if revealed.unwrap_or(true) {
                    self.view.masked = false;
                    self.view.revealed = self.config.zones.iter().map(|zone| zone.id.clone()).collect();
                    self.view.loose_revealed = true;
                } else {
                    self.view.revealed.clear();
                    self.view.loose_revealed = false;
                }
                Ok(self.privacy_changed())
            }
        }
    }

    /// The system says the screen is shown to others, or no longer.
    pub(super) fn screen_shared(&mut self, shared: bool) -> Vec<Effect> {
        if self.view.screen_shared == shared {
            return Vec::new();
        }
        self.view.screen_shared = shared;
        // What the person asked to see, they asked to see alone.
        if shared && self.config.privacy.when_shared != WhenShared::Nothing {
            self.view.revealed.clear();
            self.view.loose_revealed = false;
        }
        self.privacy_changed()
    }

    // ---- room kept free of tiled windows -------------------------------

    /// The room tiled windows are to keep off: the place of every zone
    /// that asks for it and is shown here. A rolled-up zone keeps its
    /// title's room, also while it is open for a moment, and peeking
    /// changes nothing: windows are not to jump for what passes.
    pub fn reserved_regions(&self) -> Vec<ReservedRegion> {
        let mut seen = BTreeSet::new();
        let mut regions = Vec::new();
        for definition in &self.config.zones {
            let shown = self.is_about(&definition.id) && (!self.state.quick_hidden || definition.always_visible);
            if !definition.reserves_space || !shown || !seen.insert(self.leader(&definition.id)) {
                continue;
            }
            let Some((whole, display)) = self.place_of(&definition.id) else {
                continue;
            };
            let rect = if definition.rolled_up { layout::rolled_up(whole, &self.metrics) } else { whole };
            regions.push(reserve::region(
                &display.id,
                rect,
                work_area(display, &self.reserved),
                self.companions(&definition.id),
            ));
        }
        regions
    }
}
