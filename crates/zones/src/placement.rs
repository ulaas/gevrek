//! Which item is in which zone. An item is in one zone filled by hand at
//! most; every other item of the desktop is loose, and may have a place of
//! its own on a display. Nothing here knows whether an item or a zone
//! exists: placements of what is not there (a drive that is not plugged
//! in, a zone taken out of the configuration for a moment) are kept and
//! simply not shown.

use crate::ids::{DisplayId, ItemId, ZoneId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A place in a zone's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cell {
    pub column: u32,
    pub row: u32,
}

/// Where a loose item was put: on a display, from its top left corner, in
/// logical units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoosePosition {
    pub display: DisplayId,
    pub x: i32,
    pub y: i32,
}

/// What a zone holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneMembers {
    /// In the order the person gave.
    pub items: Vec<ItemId>,
    /// The cells of the items that were put on one by hand.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub cells: BTreeMap<ItemId, Cell>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: gevrek_model::Extra,
}

/// Where an item is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "in", rename_all = "snake_case")]
pub enum Whereabouts {
    Zone {
        zone: ZoneId,
        /// Its place in the zone's order.
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<Cell>,
    },
    Loose {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<LoosePosition>,
    },
}

/// Every placement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placements {
    pub zones: BTreeMap<ZoneId, ZoneMembers>,
    /// The loose items that were put somewhere by hand. A loose item
    /// without an entry is laid out with the others.
    pub loose: BTreeMap<ItemId, LoosePosition>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: gevrek_model::Extra,
}

impl Placements {
    /// The zone an item is in.
    pub fn zone_of(&self, item: &ItemId) -> Option<&ZoneId> {
        self.zones
            .iter()
            .find(|(_, members)| members.items.contains(item))
            .map(|(zone, _)| zone)
    }

    /// The items of a zone, in order.
    pub fn items_of(&self, zone: &ZoneId) -> &[ItemId] {
        self.zones
            .get(zone)
            .map(|members| members.items.as_slice())
            .unwrap_or_default()
    }

    pub fn whereabouts(&self, item: &ItemId) -> Whereabouts {
        for (zone, members) in &self.zones {
            if let Some(index) = members.items.iter().position(|member| member == item) {
                return Whereabouts::Zone {
                    zone: zone.clone(),
                    index,
                    cell: members.cells.get(item).copied(),
                };
            }
        }
        Whereabouts::Loose {
            position: self.loose.get(item).cloned(),
        }
    }

    /// Take an item out of wherever it is. It is loose afterwards, with
    /// no place of its own.
    pub fn take(&mut self, item: &ItemId) {
        for members in self.zones.values_mut() {
            members.items.retain(|member| member != item);
            members.cells.remove(item);
        }
        self.zones.retain(|_, members| !members.items.is_empty());
        self.loose.remove(item);
    }

    /// Put an item into a zone, at `index` in its order (at the end
    /// without one, or with one beyond the end). It leaves wherever it
    /// was.
    pub fn assign(&mut self, item: &ItemId, zone: &ZoneId, index: Option<usize>, cell: Option<Cell>) {
        self.take(item);
        let members = self.zones.entry(zone.clone()).or_default();
        let index = index.unwrap_or(members.items.len()).min(members.items.len());
        members.items.insert(index, item.clone());
        if let Some(cell) = cell {
            // (One item a cell: who was there is laid out with the rest.)
            members.cells.retain(|_, taken| *taken != cell);
            members.cells.insert(item.clone(), cell);
        }
    }

    /// Make an item loose, at a place of its own if one is given.
    pub fn loosen(&mut self, item: &ItemId, position: Option<LoosePosition>) {
        self.take(item);
        if let Some(position) = position {
            self.loose.insert(item.clone(), position);
        }
    }

    /// Put an item where it was.
    pub fn put_back(&mut self, item: &ItemId, whereabouts: &Whereabouts) {
        match whereabouts {
            Whereabouts::Zone { zone, index, cell } => {
                self.assign(item, zone, Some(*index), *cell)
            }
            Whereabouts::Loose { position } => self.loosen(item, position.clone()),
        }
    }

    /// An item has another id (it was renamed): it stays where it is.
    pub fn rename(&mut self, from: &ItemId, to: &ItemId) {
        if from == to {
            return;
        }
        // (What had the new name goes first: where it stood before the
        // item in the same zone, the place read before was one too far.)
        self.take(to);
        let whereabouts = self.whereabouts(from);
        self.take(from);
        if whereabouts != (Whereabouts::Loose { position: None }) {
            self.put_back(to, &whereabouts);
        }
    }

    /// A zone is no more: its items are loose.
    pub fn drop_zone(&mut self, zone: &ZoneId) {
        self.zones.remove(zone);
    }

    /// What is wrong, if anything: an item in two places.
    pub fn broken(&self) -> Option<String> {
        let mut seen: BTreeMap<&ItemId, &ZoneId> = BTreeMap::new();
        for (zone, members) in &self.zones {
            for item in &members.items {
                if let Some(first) = seen.insert(item, zone) {
                    return Some(format!("{item} is in {first} and in {zone}"));
                }
                if self.loose.contains_key(item) {
                    return Some(format!("{item} is in {zone} and loose"));
                }
            }
            if let Some(stray) = members.cells.keys().find(|item| !members.items.contains(item)) {
                return Some(format!("{stray} has a cell in {zone} and is not in it"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> ItemId {
        ItemId::new(name)
    }

    fn zone(name: &str) -> ZoneId {
        ZoneId::new(name)
    }

    #[test]
    fn an_item_renamed_over_one_before_it_keeps_its_place() {
        let mut placements = Placements::default();
        for name in ["b", "a", "c"] {
            placements.assign(&item(name), &zone("one"), None, None);
        }
        // `a` takes the name of `b`, which stood before it: it is where
        // it was among what is left (it was put one place too far).
        placements.rename(&item("a"), &item("b"));
        assert_eq!(placements.items_of(&zone("one")), [item("b"), item("c")]);
    }

    #[test]
    fn an_item_is_in_one_place_only() {
        let mut placements = Placements::default();
        placements.assign(&item("a"), &zone("one"), None, None);
        placements.assign(&item("b"), &zone("one"), None, None);
        placements.assign(&item("a"), &zone("two"), None, None);
        assert_eq!(placements.items_of(&zone("one")), [item("b")]);
        assert_eq!(placements.items_of(&zone("two")), [item("a")]);
        let at = LoosePosition { display: DisplayId::new("d"), x: 4, y: 5 };
        placements.loosen(&item("a"), Some(at.clone()));
        assert_eq!(placements.zone_of(&item("a")), None);
        assert_eq!(placements.whereabouts(&item("a")), Whereabouts::Loose { position: Some(at) });
        placements.assign(&item("a"), &zone("one"), Some(0), None);
        assert!(placements.loose.is_empty());
        assert_eq!(placements.items_of(&zone("one")), [item("a"), item("b")]);
        assert_eq!(placements.broken(), None);
    }

    #[test]
    fn an_item_put_back_is_where_it_was() {
        let mut placements = Placements::default();
        for name in ["a", "b", "c"] {
            placements.assign(&item(name), &zone("one"), None, None);
        }
        placements.assign(&item("b"), &zone("one"), Some(1), Some(Cell { column: 2, row: 0 }));
        let before = placements.clone();
        let was = placements.whereabouts(&item("b"));
        placements.assign(&item("b"), &zone("two"), None, None);
        assert_ne!(placements, before);
        placements.put_back(&item("b"), &was);
        assert_eq!(placements, before);
    }

    #[test]
    fn a_renamed_item_stays_in_its_place() {
        let mut placements = Placements::default();
        for name in ["a", "b", "c"] {
            placements.assign(&item(name), &zone("one"), None, None);
        }
        placements.rename(&item("b"), &item("B"));
        assert_eq!(placements.items_of(&zone("one")), [item("a"), item("B"), item("c")]);
        let at = LoosePosition { display: DisplayId::new("d"), x: 1, y: 2 };
        placements.loosen(&item("x"), Some(at.clone()));
        placements.rename(&item("x"), &item("y"));
        assert_eq!(placements.loose.get(&item("y")), Some(&at));
        assert!(!placements.loose.contains_key(&item("x")));
        assert_eq!(placements.broken(), None);
    }

    #[test]
    fn two_items_never_share_a_cell() {
        let mut placements = Placements::default();
        let cell = Cell { column: 1, row: 1 };
        placements.assign(&item("a"), &zone("one"), None, Some(cell));
        placements.assign(&item("b"), &zone("one"), None, Some(cell));
        let members = &placements.zones[&zone("one")];
        assert_eq!(members.cells.len(), 1);
        assert_eq!(members.cells.get(&item("b")), Some(&cell));
    }
}
