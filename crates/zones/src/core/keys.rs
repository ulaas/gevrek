//! The zones under the keyboard. What has the keyboard is kept here, in
//! the core, and not by whoever draws: a key, `gevrek zones`, and a
//! screen reader then all speak of the same zone and the same item.
//!
//! The keyboard is on a place, and perhaps on one item of it. A place is
//! a zone (one that is shown, and of tabs the one in front) or the bare
//! desktop of a display: the items there that are in no zone, while there
//! are some and they are not put away. The desktop is named `@desk` where
//! a zone would be named. Going from place to place is by where they are
//! drawn; the desktop of a display counts as the smallest rectangle that
//! holds its loose items.

use super::{Effect, Outcome, ZonesCore, ZonesEvent};
use crate::focus::{self, Direction, Focus, SelectMode, Step, DESK};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::layout::{self, PlacedItem};
use gevrek_model::geometry::Rect;

/// Somewhere the keyboard can be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Zone(ZoneId),
    /// The loose items of a display.
    Desk(DisplayId),
}

impl ZonesCore {
    /// What has the keyboard.
    pub fn focus(&self) -> Option<&Focus> {
        self.view.focus.as_ref()
    }

    /// The chosen items, in the order of their ids.
    pub fn selection(&self) -> Vec<ItemId> {
        self.view.selection.iter().cloned().collect()
    }

    /// The bare desktops the keyboard can be on: each display on which
    /// items lie loose and are to be seen (not put away with the one key,
    /// not covered), with the rectangle that holds them and the items as
    /// they are laid out, on the desktop.
    pub(super) fn desks(&self) -> Vec<(DisplayId, Rect, Vec<PlacedItem>)> {
        if self.loose_hidden() {
            return Vec::new();
        }
        self.loose_scenes()
            .into_iter()
            .filter_map(|loose| {
                let items: Vec<PlacedItem> = loose
                    .items
                    .into_iter()
                    .map(|mut placed| {
                        placed.rect.x += loose.area.x;
                        placed.rect.y += loose.area.y;
                        placed
                    })
                    .collect();
                let around = focus::around(items.iter().map(|placed| placed.rect))?;
                Some((loose.display, around, items))
            })
            .collect()
    }

    /// The places the keyboard can go to, each with where it is drawn.
    fn reachable(&self) -> Vec<(Place, Rect)> {
        let groups = layout::tab_groups(&self.config.zones);
        let zones = self
            .config
            .zones
            .iter()
            .filter(|zone| self.is_shown(&zone.id))
            .filter(|zone| layout::group_of(&groups, &zone.id).is_none_or(|group| self.front_of(group) == zone.id))
            .filter_map(|zone| Some((Place::Zone(zone.id.clone()), self.zone_rect(&zone.id)?)));
        let desks = self.desks().into_iter().map(|(display, around, _)| (Place::Desk(display), around));
        zones.chain(desks).collect()
    }

    /// The place that has the keyboard.
    fn keyboard_place(&self) -> Option<Place> {
        let focus = self.view.focus.as_ref()?;
        Some(match &focus.desk {
            Some(display) => Place::Desk(display.clone()),
            None => Place::Zone(focus.zone.clone()),
        })
    }

    /// The items of a place as the keyboard walks them, in their order.
    fn items_of(&self, place: &Place, now: u64) -> Vec<ItemId> {
        match place {
            Place::Zone(zone) => self.zone_items(zone, now).iter().map(|item| item.id.clone()).collect(),
            Place::Desk(display) => self
                .desks()
                .into_iter()
                .find(|(on, _, _)| on == display)
                .map(|(_, _, items)| items.into_iter().map(|placed| placed.item).collect())
                .unwrap_or_default(),
        }
    }

    /// Put the keyboard on a place. A zone's tab comes to the front. What
    /// was chosen in another place is chosen no longer.
    fn take_focus(&mut self, place: &Place, item: Option<ItemId>) -> Outcome {
        let mut effects = Vec::new();
        let focus = match place {
            Place::Zone(zone) => {
                self.known(zone)?;
                if !self.is_shown(zone) {
                    return Err(format!("{zone} is not shown"));
                }
                if self.known(zone)?.tab_group.is_some() {
                    effects.extend(self.activate_tab(zone)?);
                }
                // An item cannot be walked to in a zone of which only the
                // title is there: the zone opens for as long as the
                // keyboard is in it.
                if item.is_some() && self.is_rolled(zone) {
                    effects.extend(self.set_expanded(zone, true));
                }
                Focus::on_zone(zone, item)
            }
            Place::Desk(display) => {
                if !self.desks().iter().any(|(on, _, _)| on == display) {
                    return Err("nothing lies loose there to go to".to_string());
                }
                Focus::on_desk(display, item)
            }
        };
        if self.keyboard_place().as_ref() != Some(place) {
            self.view.selection.clear();
        }
        self.view.focus = Some(focus);
        Ok(effects)
    }

    pub(super) fn focus_zone(&mut self, zone: Option<ZoneId>) -> Outcome {
        match zone {
            // The bare desktop, by its name: the first display that has
            // loose items to go to.
            Some(zone) if zone.as_str() == DESK => {
                let display = self.desks().into_iter().next().map(|(display, _, _)| display);
                let display = display.ok_or("nothing lies loose on the desktop to go to")?;
                self.take_focus(&Place::Desk(display), None)
            }
            Some(zone) => self.take_focus(&Place::Zone(zone), None),
            None => {
                self.view.focus = None;
                self.view.selection.clear();
                Ok(Vec::new())
            }
        }
    }

    /// The place that lies in a direction from the one that has the
    /// keyboard; from none, the first there is.
    fn neighbour(&self, direction: Direction) -> Option<Place> {
        let own = self.keyboard_place();
        // (Tabs lie on one place: the others of its tabs are not "beside" a zone.)
        let together = match &own {
            Some(Place::Zone(zone)) => self.companions(zone),
            _ => Vec::new(),
        };
        let all = self.reachable();
        let from = own.as_ref().and_then(|own| all.iter().find(|(place, _)| place == own)).map(|(_, rect)| *rect);
        // (A zone that has the keyboard and is no longer among the
        // reachable ones, a tab behind, is gone from where it is drawn.)
        let from = from.or_else(|| match &own {
            Some(Place::Zone(zone)) => self.zone_rect(zone),
            _ => None,
        });
        let others: Vec<(Place, Rect)> = all
            .into_iter()
            .filter(|(place, _)| Some(place) != own.as_ref())
            .filter(|(place, _)| !matches!(place, Place::Zone(zone) if together.contains(zone)))
            .collect();
        let rects: Vec<Rect> = others.iter().map(|(_, rect)| *rect).collect();
        let index = match from {
            Some(from) => focus::neighbour(from, &rects, direction),
            None => focus::first(&rects),
        };
        index.map(|index| others[index].0.clone())
    }

    pub(super) fn focus_neighbour(&mut self, direction: Direction) -> Outcome {
        match self.neighbour(direction) {
            Some(place) => self.take_focus(&place, None),
            // At the side of the desktop the keyboard stays where it is.
            None => Ok(Vec::new()),
        }
    }

    /// The place an item is shown in: the zone filled by hand it was put
    /// into, else the bare desktop it lies on, else the first shown zone
    /// that holds it.
    fn shown_in(&self, item: &ItemId, now: u64) -> Option<Place> {
        if let Some(zone) = self.zone_of(item).filter(|zone| self.is_shown(zone)) {
            return Some(Place::Zone(zone.clone()));
        }
        let lies_loose = |items: &[PlacedItem]| items.iter().any(|placed| &placed.item == item);
        if let Some((display, _, _)) = self.desks().into_iter().find(|(_, _, items)| lies_loose(items)) {
            return Some(Place::Desk(display));
        }
        self.config
            .zones
            .iter()
            .filter(|zone| self.is_shown(&zone.id))
            .find(|zone| self.zone_holds(&zone.id, now).iter().any(|held| &held.id == item))
            .map(|zone| Place::Zone(zone.id.clone()))
    }

    pub(super) fn focus_item(&mut self, item: Option<ItemId>, now: u64) -> Outcome {
        let Some(item) = item else {
            if let Some(focus) = &mut self.view.focus {
                focus.item = None;
            }
            return Ok(Vec::new());
        };
        let place = self.shown_in(&item, now).ok_or_else(|| format!("{item} is nowhere that is shown"))?;
        let mut effects = Vec::new();
        if let Place::Zone(zone) = &place {
            if self.is_masked(zone) {
                return Err(format!("{zone} is covered"));
            }
            // A filter that hides the item would hide the keyboard with it.
            let hidden = !self.zone_items(zone, now).iter().any(|shown| shown.id == item);
            if hidden && self.view.filters.remove(zone).is_some() {
                effects.push(Effect::Notify(ZonesEvent::FilterChanged { zone: zone.clone(), typed: String::new() }));
                effects.push(Effect::Notify(ZonesEvent::ItemsChanged { zone: Some(zone.clone()) }));
            }
        }
        effects.extend(self.take_focus(&place, Some(item.clone()))?);
        self.view.selection = [item].into();
        Ok(effects)
    }

    pub(super) fn move_item_focus(&mut self, step: Step, extend: bool, now: u64) -> Outcome {
        let mut effects = Vec::new();
        let place = match self.keyboard_place() {
            Some(place) => place,
            None => {
                let first = self.neighbour(Direction::Right).ok_or("nothing is shown to go to")?;
                effects.extend(self.take_focus(&first, None)?);
                first
            }
        };
        let from = self.view.focus.as_ref().and_then(|focus| focus.item.clone());
        let next = match &place {
            Place::Zone(zone) => {
                if self.is_masked(zone) {
                    return Err(format!("{zone} is covered"));
                }
                // (Open before it is laid out: the items are the same, and
                // this way the zone is open even where it turns out to be
                // empty.)
                if self.is_rolled(zone) {
                    effects.extend(self.set_expanded(zone, true));
                }
                let groups = layout::tab_groups(&self.config.zones);
                let placed = self
                    .config
                    .zone(zone)
                    .and_then(|definition| self.zone_scene(definition, &groups, now))
                    .map(|scene| scene.items.items)
                    .unwrap_or_default();
                focus::step(&placed, from.as_ref(), step).map(|index| placed[index].item.clone())
            }
            // Loose items lie where they were put: to the nearest that way.
            Place::Desk(display) => {
                let placed =
                    self.desks().into_iter().find(|(on, _, _)| on == display).map(|(_, _, items)| items).unwrap_or_default();
                focus::step_free(&placed, from.as_ref(), step).map(|index| placed[index].item.clone())
            }
        };
        let Some(item) = next else {
            return Ok(effects);
        };
        if extend {
            self.view.selection.extend(from);
            self.view.selection.insert(item.clone());
        } else {
            self.view.selection = [item.clone()].into();
        }
        if let Some(focus) = &mut self.view.focus {
            focus.item = Some(item);
        }
        Ok(effects)
    }

    pub(super) fn select(&mut self, items: &[ItemId], mode: SelectMode) -> Vec<Effect> {
        let known: Vec<ItemId> = items.iter().filter(|item| self.item(item).is_some()).cloned().collect();
        let chosen = &mut self.view.selection;
        match mode {
            SelectMode::Replace => *chosen = known.into_iter().collect(),
            SelectMode::Add => chosen.extend(known),
            SelectMode::Remove => known.iter().for_each(|item| {
                chosen.remove(item);
            }),
            SelectMode::Toggle => known.into_iter().for_each(|item| {
                if !chosen.remove(&item) {
                    chosen.insert(item);
                }
            }),
        }
        // (`settle` tells of it.)
        Vec::new()
    }

    /// What a key acts on: the chosen items, or else the one that has
    /// the keyboard.
    pub(super) fn acted_on(&self) -> Vec<ItemId> {
        if !self.view.selection.is_empty() {
            return self.view.selection.iter().cloned().collect();
        }
        self.view.focus.as_ref().and_then(|focus| focus.item.clone()).into_iter().collect()
    }

    pub(super) fn move_focused_items(
        &mut self,
        direction: Option<Direction>,
        zone: Option<ZoneId>,
        now: u64,
    ) -> Outcome {
        let items = self.acted_on();
        if items.is_empty() {
            return Err("no item has the keyboard and none is chosen".to_string());
        }
        // Where to: the zone named, the bare desktop by its name, or what
        // lies that way.
        let to_zone = match (zone, direction) {
            (Some(zone), _) if zone.as_str() == DESK => None,
            (Some(zone), _) => Some(zone),
            (None, Some(direction)) => match self.neighbour(direction).ok_or("nothing lies that way")? {
                Place::Zone(zone) => Some(zone),
                Place::Desk(_) => None,
            },
            (None, None) => return Err("say which zone, or which way".to_string()),
        };
        if let Some(zone) = &to_zone {
            self.known(zone)?;
        }
        // Where the keyboard was among the items, to leave it on what
        // comes to stand there.
        let source = self.keyboard_place();
        let stood_at = source.as_ref().and_then(|place| {
            let item = self.view.focus.as_ref()?.item.clone()?;
            self.items_of(place, now).iter().position(|shown| shown == &item)
        });
        let mut effects = Vec::new();
        let mut refusal = None;
        for item in &items {
            let moved = match &to_zone {
                Some(zone) => self.add_item(zone, item, None, None),
                // Out of its zone, to lie with the loose items.
                None => self.loosen_item(item, None),
            };
            match moved {
                Ok(done) => effects.extend(done),
                Err(reason) => refusal = refusal.or(Some(reason)),
            }
        }
        // Only where nothing at all could go is the command refused.
        if effects.is_empty() {
            return Err(refusal.unwrap_or_else(|| "they are there already".to_string()));
        }
        if let (Some(place), Some(stood_at)) = (source, stood_at) {
            let left = self.items_of(&place, now);
            let item = left.get(stood_at.min(left.len().saturating_sub(1))).cloned();
            self.view.selection = item.iter().cloned().collect();
            if let Some(focus) = &mut self.view.focus {
                focus.item = item;
            }
        }
        Ok(effects)
    }
}
