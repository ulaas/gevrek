//! Zones made by type, carried out: the plan of `crate::auto` is made
//! from the desktop as the core knows it, and done exactly as it says;
//! what was done is kept in the state so that one command takes it back.
//! And the one switch for the whole desktop that is kept with it: icons
//! drawn grey.

use super::{Effect, Outcome, ZonesCore, ZonesEvent};
use crate::auto::{self, Asked, AutoMade, AutoMoved, AutoPlan, AutoUndo, Room};
use crate::config::ZoneKind;
use crate::geometry::{fallback_display, work_area};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::placement::Whereabouts;
use crate::rules::Scope;
use gevrek_model::geometry::Rect;
use std::collections::BTreeSet;

impl ZonesCore {
    /// Whether every icon is drawn grey (`Command::GreyIcons`). It is a
    /// way of looking at the desktop, laid over whatever the looks say
    /// (`look.icon_saturation` stays as the person set it), and it is
    /// remembered as the one key's putting away is.
    pub fn grey_icons(&self) -> bool {
        self.state.grey_icons
    }

    pub(super) fn set_grey_icons(&mut self, on: Option<bool>) -> Outcome {
        let on = on.unwrap_or(!self.state.grey_icons);
        if on == self.state.grey_icons {
            return Ok(Vec::new());
        }
        self.state.grey_icons = on;
        Ok(vec![Effect::Notify(ZonesEvent::GreyIconsChanged { on }), Effect::SaveState])
    }

    /// Whether the last making of zones by type can be taken back
    /// (`Command::UndoAutoCreate`).
    pub fn can_undo_auto(&self) -> bool {
        self.state.auto_undo.is_some()
    }

    /// The zones that belong to what is shown here and stand on a
    /// display, each whole (a rolled-up one opens to that), tabs once.
    fn standing_on(&self, display: &DisplayId) -> Vec<Rect> {
        let mut seen = BTreeSet::new();
        self.config
            .zones
            .iter()
            .filter(|zone| self.is_about(&zone.id) && seen.insert(self.leader(&zone.id)))
            .filter_map(|zone| self.place_of(&zone.id))
            .filter(|(_, on)| &on.id == display)
            .map(|(rect, _)| rect)
            .collect()
    }

    /// What making zones by type would do now, or why it cannot be said.
    fn auto_planned(
        &self,
        scope: Scope,
        min_items: Option<u32>,
        display: Option<&DisplayId>,
        add_rules: Option<bool>,
    ) -> Result<AutoPlan, String> {
        if self.current.is_none() {
            return Err("no display is known yet".to_string());
        }
        let display = match display {
            Some(id) => self
                .displays
                .iter()
                .find(|display| &display.id == id)
                .ok_or_else(|| format!("no display is called {id}"))?,
            None => fallback_display(&self.displays).ok_or("no display is known yet")?,
        };
        let mut taken = self.standing_on(&display.id);
        let area = work_area(display, &self.reserved);
        let asked = Asked {
            scope,
            min_items: min_items.unwrap_or(self.config.auto.min_items),
            add_rules: add_rules.unwrap_or(self.config.auto.add_rules),
        };
        let planned = |taken: &[Rect]| {
            let room = Room { display: &display.id, area, taken, metrics: &self.metrics };
            auto::plan(&self.config, self.items.values(), &self.state.placements, asked, &room, &|zone| self.zone_rect(zone))
        };
        // What stays loose keeps its place on the desktop: a new zone is
        // not made over an icon that goes into none. Which items stay is
        // known once it is known which go, so the zones are placed twice:
        // first to see what goes, then around what is left lying.
        let first = planned(&taken);
        let goes: BTreeSet<&ItemId> = first.zones.iter().flat_map(|zone| zone.items.iter()).collect();
        taken.extend(self.loose_left_on(display, area, &goes));
        Ok(planned(&taken))
    }

    /// Where the loose items of a display would lie once some of them
    /// have gone into zones: each one's cell, on the desktop.
    fn loose_left_on(&self, display: &crate::geometry::Display, area: Rect, goes: &BTreeSet<&ItemId>) -> Vec<Rect> {
        let Some(fallback) = fallback_display(&self.displays).map(|display| &display.id) else {
            return Vec::new();
        };
        let here = |item: &&crate::Item| {
            let own = self.state.placements.loose.get(&item.id).map(|position| &position.display);
            own.filter(|wanted| self.displays.iter().any(|display| &display.id == *wanted)).unwrap_or(fallback) == &display.id
        };
        let stays: Vec<&crate::Item> = self.loose_items().into_iter().filter(|item| !goes.contains(&item.id) && !self.is_withheld(&item.id)).filter(here).collect();
        let ids: Vec<ItemId> = crate::layout::sorted(stays, crate::config::Sort::Name, false).iter().map(|item| item.id.clone()).collect();
        let own: std::collections::BTreeMap<ItemId, (i32, i32)> = ids
            .iter()
            .filter_map(|id| {
                let position = self.state.placements.loose.get(id)?;
                (position.display == display.id)
                    .then(|| (id.clone(), (position.x - (area.x - display.rect.x), position.y - (area.y - display.rect.y))))
            })
            .collect();
        crate::layout::lay_out_loose(self.config.loose_icon_size, &self.metrics, (area.width, area.height), &ids, &own)
            .into_iter()
            .map(|placed| Rect::new(area.x + placed.rect.x, area.y + placed.rect.y, placed.rect.width, placed.rect.height))
            .collect()
    }

    /// What making zones by type would do now (`Command::AutoCreate`
    /// carries out exactly this), and nothing is changed by asking.
    ///
    /// `scope`: `Loose` asks about the items that are in no zone; `All`
    /// also about the items of the zones the person filled by hand (what
    /// is in a zone that was made by type stays there). `min_items`: how
    /// many items a type needs for a zone of its own (`auto.min_items`
    /// where none is given). `display`: where new zones are made (the
    /// primary one where none is given). While no display is known, or
    /// the one named is not there, the plan is empty.
    pub fn auto_plan(&self, scope: Scope, min_items: Option<u32>, display: Option<&DisplayId>, _now: u64) -> AutoPlan {
        self.auto_planned(scope, min_items, display, None).unwrap_or_default()
    }

    pub(super) fn auto_create(
        &mut self,
        scope: Scope,
        min_items: Option<u32>,
        display: Option<&DisplayId>,
        add_rules: Option<bool>,
        dry_run: bool,
        now: u64,
    ) -> Outcome {
        if self.config.lock.members {
            return Err("every zone is locked: nothing goes in or out".to_string());
        }
        let plan = self.auto_planned(scope, min_items, display, add_rules)?;
        if dry_run {
            return Ok(vec![Effect::Notify(ZonesEvent::AutoPlanned { plan })]);
        }
        if plan.zones.is_empty() {
            // Nothing to do, and nothing to take back that was not there before.
            return Ok(vec![Effect::Notify(ZonesEvent::AutoCreated { zones: Vec::new(), items: 0, rules: 0 })]);
        }
        let mut record = AutoUndo { made_at: now, ..AutoUndo::default() };
        let mut effects = Vec::new();
        let mut filled: Vec<ZoneId> = Vec::new();
        let mut emptied: BTreeSet<Option<ZoneId>> = BTreeSet::new();
        for planned in &plan.zones {
            let zone = match &planned.zone {
                Some(zone) => zone.clone(),
                None => {
                    let made = self.create_zone(planned.title.clone(), ZoneKind::Manual, Some(planned.rect))?;
                    let zone = made
                        .iter()
                        .find_map(|effect| match effect {
                            Effect::Notify(ZonesEvent::ZoneCreated { zone }) => Some(zone.clone()),
                            _ => None,
                        })
                        .ok_or("the zone was not made")?;
                    effects.extend(made);
                    if let Some(definition) = self.config.zone_mut(&zone) {
                        definition.auto = Some(planned.category.clone());
                    }
                    record.zones.push(AutoMade { zone: zone.clone(), category: planned.category.clone() });
                    let category = self.config.auto.categories.iter().find(|category| category.id == planned.category);
                    if let Some(rule) = category.filter(|_| planned.rule).and_then(|category| auto::rule_for(category, &zone)) {
                        self.config.rules.push(rule.clone());
                        record.rules.push(rule);
                    }
                    zone
                }
            };
            for item in &planned.items {
                emptied.insert(self.zone_of(item).cloned());
                let before = self.state.placements.whereabouts(item);
                self.state.placements.assign(item, &zone, None, None);
                record.entries.push(AutoMoved { item: item.clone(), before, zone: zone.clone() });
            }
            filled.push(zone);
        }
        let config_changed = !record.zones.is_empty();
        // One saving for all of it, at the end.
        effects.retain(|effect| !matches!(effect, Effect::SaveConfig | Effect::SaveState));
        for zone in &filled {
            effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone.clone()) }));
        }
        for zone in emptied {
            effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone }));
        }
        effects.push(Effect::Notify(ZonesEvent::AutoCreated {
            zones: record.zones.iter().map(|made| made.zone.clone()).collect(),
            items: record.entries.len(),
            rules: record.rules.len(),
        }));
        self.state.auto_undo = Some(record);
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        if config_changed {
            effects.push(Effect::SaveConfig);
        }
        effects.push(Effect::SaveState);
        Ok(effects)
    }

    /// Take back the last making of zones by type: every item it moved
    /// that is still where it was put goes back where it was (into its
    /// zone at its place, or loose where it lay), the zones it made are
    /// taken away where they are still that type's, and the rules it
    /// added are taken out where they are still there under their names.
    /// What the person has changed since is theirs: an item put elsewhere
    /// stays; a zone that is locked, or was given to another purpose (its
    /// `auto` taken off), stays with what it holds; and what the person
    /// put into a zone that goes lies loose, as with any zone that is
    /// taken away.
    pub(super) fn undo_auto_create(&mut self) -> Outcome {
        if self.state.auto_undo.is_none() {
            return Err("no zones were made by type, or it was taken back already".to_string());
        }
        if self.config.lock.members {
            return Err("every zone is locked: nothing goes in or out".to_string());
        }
        let record = self.state.auto_undo.take().unwrap_or_default();
        // The zones it made that go again: still that type's, and not locked.
        let goes = |core: &Self, made: &AutoMade| {
            core.config.zone(&made.zone).is_some_and(|zone| zone.auto.as_deref() == Some(made.category.as_str()))
                && !core.lock_of(&made.zone).members
        };
        let stays: BTreeSet<&ZoneId> =
            record.zones.iter().filter(|made| !goes(self, made)).map(|made| &made.zone).collect();
        let mut touched: BTreeSet<Option<ZoneId>> = BTreeSet::new();
        let mut items = 0;
        // The last first: each entry says where its item was when it was
        // moved, which is where it is again once the later ones are back.
        for entry in record.entries.iter().rev() {
            let still_there = self.state.placements.zone_of(&entry.item) == Some(&entry.zone);
            if !still_there || stays.contains(&entry.zone) || self.lock_of(&entry.zone).members {
                continue;
            }
            // A zone that is gone, fills itself now or is locked takes
            // nothing back: its item lies loose.
            let before = match &entry.before {
                Whereabouts::Zone { zone, .. }
                    if !self.config.zone(zone).is_some_and(|zone| zone.kind == ZoneKind::Manual && !zone.lock.members) =>
                {
                    Whereabouts::Loose { position: None }
                }
                before => before.clone(),
            };
            self.state.placements.put_back(&entry.item, &before);
            touched.insert(Some(entry.zone.clone()));
            touched.insert(match before {
                Whereabouts::Zone { zone, .. } => Some(zone),
                Whereabouts::Loose { .. } => None,
            });
            items += 1;
        }
        let mut effects = Vec::new();
        let mut zones = 0;
        for made in &record.zones {
            if !goes(self, made) {
                continue;
            }
            // (What the person put into it since lies loose then.)
            touched.insert(None);
            effects.extend(self.delete_zone(&made.zone)?);
            zones += 1;
        }
        let mut rules = 0;
        for rule in &record.rules {
            let place = self
                .config
                .rules
                .iter()
                .position(|other| other.name == rule.name && other.action == rule.action);
            if let Some(place) = place {
                self.config.rules.remove(place);
                rules += 1;
            }
        }
        effects.retain(|effect| !matches!(effect, Effect::SaveConfig | Effect::SaveState));
        for zone in touched {
            if zone.as_ref().is_none_or(|zone| self.config.zone(zone).is_some()) {
                effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone }));
            }
        }
        effects.push(Effect::Notify(ZonesEvent::AutoUndone { zones, items, rules }));
        effects.push(Effect::Notify(ZonesEvent::LayoutChanged));
        if zones > 0 || rules > 0 {
            effects.push(Effect::SaveConfig);
        }
        effects.push(Effect::SaveState);
        Ok(effects)
    }
}
