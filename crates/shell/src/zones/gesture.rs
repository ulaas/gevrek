//! What the pointer and the keys mean: from where they are and what they
//! do, to what is asked of the zones. Everything that changes the zones
//! comes out as a `Command`, the same one a key or another program would
//! give; what does not (opening an item, a menu, a drag out of the
//! desktop) comes out as what to do. Nothing here draws.

use super::view::{Fills, Hit, Layout, Target};
use egui::{pos2, Pos2, Rect, Vec2};
use gevrek_zones::core::Command;
use gevrek_zones::snap::Edges;
use gevrek_zones::{DisplayId, ItemId, ZoneId};
use std::collections::BTreeSet;

/// How far the pointer goes with a button held before it is a drag.
const SLACK: f32 = 4.0;

/// What the pointer did since the last look, on the desktop as a whole
/// (in its logical units).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pointer {
    /// Where it is; `None` when it is on none of our surfaces.
    pub at: Option<Pos2>,
    /// The main button went down, came up, is held.
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    /// The press was the second of two in quick succession.
    pub double: bool,
    /// The other button went down.
    pub menu: bool,
    pub ctrl: bool,
    pub shift: bool,
    /// Held while moving or sizing: nothing pulls.
    pub alt: bool,
}

/// A key that means something to the zones, when they have the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Open what is selected.
    Enter,
    /// Let go of whatever is being done, then of the selection.
    Escape,
    SelectAll,
    /// To the next item this way (columns, rows).
    Step(i32, i32),
    /// Put what is selected into the bin (the zones do it, as a command).
    Delete,
    /// Give the one selected item another name.
    Rename,
}

/// What follows from the pointer or a key.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Ask the zones for something.
    Do(Command),
    /// Open items as a double click does.
    Open(Vec<ItemId>),
    /// The menu of some items, at a point of the desktop.
    ItemMenu { items: Vec<ItemId>, at: Pos2 },
    ZoneMenu { zone: ZoneId, at: Pos2 },
    DeskMenu { display: DisplayId, at: Pos2 },
    /// The items are dragged off our surfaces: the system carries on.
    DragOut(Vec<ItemId>),
}

/// What is being done with the button held.
#[derive(Debug, Clone, PartialEq)]
pub enum Doing {
    Nothing,
    /// Pressed, and not yet moved far enough to be a drag.
    Pressed {
        hit: Hit,
        at: Pos2,
        /// The item was selected before: a click (not a drag) then
        /// leaves it the only one.
        narrow: bool,
    },
    /// A zone is moved by its title; `grab` is from its corner to the pointer.
    Moving { zone: ZoneId, grab: Vec2 },
    /// A zone is sized; `from` is what it was, `at` where the pointer took hold.
    Sizing { zone: ZoneId, edges: Edges, from: Rect, at: Pos2 },
    /// Items are carried; each with where its corner is from the pointer.
    Carrying { items: Vec<(ItemId, Vec2)>, at: Pos2 },
    /// A rectangle is drawn to select with; `kept` was selected before.
    Banding { from: Pos2, to: Pos2, kept: BTreeSet<ItemId> },
}

/// The pointer's and the keys' own state: what is selected, and what is
/// under way.
#[derive(Debug, Clone, PartialEq)]
pub struct Gestures {
    pub doing: Doing,
    pub selected: BTreeSet<ItemId>,
    /// The item the keys go from, and a range with Shift begins at.
    pub focus: Option<ItemId>,
}

impl Default for Gestures {
    fn default() -> Self {
        Self { doing: Doing::Nothing, selected: BTreeSet::new(), focus: None }
    }
}

impl Gestures {
    /// Whether something is under way that needs every movement.
    pub fn busy(&self) -> bool {
        self.doing != Doing::Nothing
    }

    /// The rectangle being drawn to select with.
    pub fn band(&self) -> Option<Rect> {
        match &self.doing {
            Doing::Banding { from, to, .. } => Some(Rect::from_two_pos(*from, *to)),
            _ => None,
        }
    }

    /// The selection in the order it is shown.
    fn selection_in_order(&self, layout: &Layout) -> Vec<ItemId> {
        layout
            .every_item()
            .filter(|shown| self.selected.contains(&shown.item))
            .map(|shown| shown.item.clone())
            .collect()
    }

    /// Forget what is no longer shown (an item went, a zone was rolled up).
    pub fn settle(&mut self, layout: &Layout) {
        let shown: BTreeSet<&ItemId> = layout.every_item().map(|item| &item.item).collect();
        self.selected.retain(|item| shown.contains(item));
        if self.focus.as_ref().is_some_and(|item| !shown.contains(item)) {
            self.focus = None;
        }
    }

    /// The pointer did something.
    pub fn pointer(&mut self, layout: &Layout, pointer: &Pointer) -> Vec<Act> {
        let mut acts = Vec::new();
        if let Some(at) = pointer.at {
            if pointer.menu {
                self.doing = Doing::Nothing;
                acts.extend(self.menu(layout, at));
            } else if pointer.pressed {
                acts.extend(self.press(layout, at, pointer));
            }
        }
        if pointer.down && !pointer.pressed {
            acts.extend(self.drag(layout, pointer));
        }
        if pointer.released {
            acts.extend(self.release(layout, pointer));
        }
        acts
    }

    fn menu(&mut self, layout: &Layout, at: Pos2) -> Option<Act> {
        Some(match layout.hit(at) {
            Hit::Item { item, .. } => {
                // The menu is of what is selected; an item outside the
                // selection becomes it.
                if !self.selected.contains(&item) {
                    self.selected = BTreeSet::from([item.clone()]);
                }
                self.focus = Some(item);
                Act::ItemMenu { items: self.selection_in_order(layout), at }
            }
            Hit::Head(zone) | Hit::Tab(zone) | Hit::Edge(zone, _) | Hit::Body(zone) => {
                Act::ZoneMenu { zone, at }
            }
            Hit::Desk(display) => Act::DeskMenu { display, at },
            Hit::Nothing => return None,
        })
    }

    fn press(&mut self, layout: &Layout, at: Pos2, pointer: &Pointer) -> Vec<Act> {
        let hit = layout.hit(at);
        let mut acts = Vec::new();
        let mut narrow = false;
        match &hit {
            Hit::Item { item, .. } => {
                if pointer.double {
                    if !self.selected.contains(item) {
                        self.selected = BTreeSet::from([item.clone()]);
                    }
                    self.focus = Some(item.clone());
                    self.doing = Doing::Nothing;
                    return vec![Act::Open(self.selection_in_order(layout))];
                }
                if pointer.shift {
                    // From the item the keys are on to this one, among
                    // those shown with it.
                    let beside = layout.beside(item);
                    let place = |wanted: &ItemId| beside.iter().position(|shown| &shown.item == wanted);
                    let from = self.focus.as_ref().and_then(place).or_else(|| place(item)).unwrap_or(0);
                    let to = place(item).unwrap_or(from);
                    if !pointer.ctrl {
                        self.selected.clear();
                    }
                    for shown in &beside[from.min(to)..=from.max(to).min(beside.len().saturating_sub(1))] {
                        self.selected.insert(shown.item.clone());
                    }
                } else if pointer.ctrl {
                    if !self.selected.remove(item) {
                        self.selected.insert(item.clone());
                    }
                    self.focus = Some(item.clone());
                } else {
                    narrow = self.selected.contains(item) && self.selected.len() > 1;
                    if !self.selected.contains(item) {
                        self.selected = BTreeSet::from([item.clone()]);
                    }
                    self.focus = Some(item.clone());
                }
            }
            Hit::Head(zone) if pointer.double => {
                self.doing = Doing::Nothing;
                return vec![Act::Do(Command::RollUp { zone: zone.clone(), rolled_up: None })];
            }
            Hit::Tab(zone) => acts.push(Act::Do(Command::ActivateTab { zone: zone.clone() })),
            Hit::Desk(_) if pointer.double => {
                self.doing = Doing::Nothing;
                return vec![Act::Do(Command::QuickHide { hidden: None })];
            }
            Hit::Body(_) | Hit::Desk(_) if !(pointer.ctrl || pointer.shift) => {
                self.selected.clear();
                self.focus = None;
            }
            _ => {}
        }
        self.doing = match hit {
            Hit::Nothing => Doing::Nothing,
            hit => Doing::Pressed { hit, at, narrow },
        };
        acts
    }

    fn drag(&mut self, layout: &Layout, pointer: &Pointer) -> Vec<Act> {
        // Off every surface of ours with items in hand: the system takes
        // over, and the drag is no longer ours to follow.
        let Some(at) = pointer.at else {
            if let Doing::Carrying { items, .. } = &self.doing {
                let items = items.iter().map(|(item, _)| item.clone()).collect();
                self.doing = Doing::Nothing;
                return vec![Act::DragOut(items)];
            }
            return Vec::new();
        };
        if let Doing::Pressed { hit, at: from, .. } = &self.doing {
            if (at - *from).length() < SLACK {
                return Vec::new();
            }
            let from = *from;
            self.doing = match hit.clone() {
                Hit::Head(zone) | Hit::Tab(zone) => match layout.zone(&zone) {
                    Some(shown) if !shown.locked_place => Doing::Moving { zone, grab: from - shown.rect.min },
                    _ => Doing::Nothing,
                },
                Hit::Edge(zone, edges) => match layout.zone(&zone) {
                    Some(shown) => Doing::Sizing { zone, edges, from: shown.rect, at: from },
                    None => Doing::Nothing,
                },
                Hit::Item { .. } => Doing::Carrying {
                    items: self
                        .selection_in_order(layout)
                        .into_iter()
                        .filter_map(|item| {
                            let corner = layout.item(&item)?.rect.min;
                            Some((item, corner - from))
                        })
                        .collect(),
                    at,
                },
                Hit::Body(_) | Hit::Desk(_) => {
                    Doing::Banding { from, to: at, kept: self.selected.clone() }
                }
                Hit::Nothing => Doing::Nothing,
            };
        }
        match &mut self.doing {
            Doing::Moving { zone, grab } => {
                let corner = at - *grab;
                vec![Act::Do(Command::MoveZone {
                    zone: zone.clone(),
                    x: corner.x.round() as i32,
                    y: corner.y.round() as i32,
                    snap: !pointer.alt,
                })]
            }
            Doing::Sizing { zone, edges, from, at: held } => {
                let by = at - *held;
                let mut rect = *from;
                if edges.left {
                    rect.min.x += by.x;
                }
                if edges.right {
                    rect.max.x += by.x;
                }
                if edges.top {
                    rect.min.y += by.y;
                }
                if edges.bottom {
                    rect.max.y += by.y;
                }
                vec![Act::Do(Command::ResizeZone {
                    zone: zone.clone(),
                    rect: gevrek_zones::Rect::new(
                        rect.min.x.round() as i32,
                        rect.min.y.round() as i32,
                        rect.width().round() as i32,
                        rect.height().round() as i32,
                    ),
                    edges: *edges,
                    snap: !pointer.alt,
                })]
            }
            Doing::Carrying { at: now, .. } => {
                *now = at;
                Vec::new()
            }
            Doing::Banding { from, to, kept } => {
                *to = at;
                let mut selected = kept.clone();
                selected.extend(layout.items_in(Rect::from_two_pos(*from, at)));
                self.selected = selected;
                Vec::new()
            }
            Doing::Nothing | Doing::Pressed { .. } => Vec::new(),
        }
    }

    fn release(&mut self, layout: &Layout, pointer: &Pointer) -> Vec<Act> {
        match std::mem::replace(&mut self.doing, Doing::Nothing) {
            // A click on one of several selected items leaves it alone selected.
            Doing::Pressed { hit: Hit::Item { item, .. }, narrow: true, .. } => {
                self.selected = BTreeSet::from([item]);
                Vec::new()
            }
            Doing::Carrying { items, at } => dropped(layout, &items, pointer.at.unwrap_or(at)),
            _ => Vec::new(),
        }
    }

    /// A key was pressed while the zones have the keyboard.
    pub fn key(&mut self, layout: &Layout, key: Key) -> Vec<Act> {
        match key {
            Key::Enter if !self.selected.is_empty() => vec![Act::Open(self.selection_in_order(layout))],
            // (These two are commands; `Zones::key` gives them.)
            Key::Enter | Key::Delete | Key::Rename => Vec::new(),
            Key::Escape => {
                if self.doing == Doing::Nothing {
                    self.selected.clear();
                } else {
                    self.doing = Doing::Nothing;
                }
                Vec::new()
            }
            Key::SelectAll => {
                // Those shown with the item the keys are on; without one,
                // everything.
                self.selected = match &self.focus {
                    Some(item) => layout.beside(item).iter().map(|shown| shown.item.clone()).collect(),
                    None => layout.every_item().map(|shown| shown.item.clone()).collect(),
                };
                Vec::new()
            }
            Key::Step(across, down) => {
                let from = self.focus.as_ref().and_then(|item| layout.item(item)).map(|shown| shown.rect);
                let next = match (&self.focus, from) {
                    (Some(item), Some(from)) => {
                        // The nearest one that lies that way.
                        let way = egui::vec2(across as f32, down as f32);
                        layout
                            .beside(item)
                            .iter()
                            .filter(|shown| (shown.rect.center() - from.center()).dot(way) > 1.0)
                            .min_by(|a, b| {
                                let far = |rect: Rect| {
                                    let by = rect.center() - from.center();
                                    // Straight ahead before aside.
                                    by.dot(way).abs() + 3.0 * (by.x * way.y).abs() + 3.0 * (by.y * way.x).abs()
                                };
                                far(a.rect).total_cmp(&far(b.rect))
                            })
                            .map(|shown| shown.item.clone())
                    }
                    _ => layout.every_item().next().map(|shown| shown.item.clone()),
                };
                if let Some(next) = next {
                    self.selected = BTreeSet::from([next.clone()]);
                    self.focus = Some(next);
                }
                Vec::new()
            }
        }
    }
}

/// What letting go of carried items at a point asks for. Into a zone
/// filled by hand they are put (at the cell or line under the pointer
/// where its order is the person's); onto a portal the files are moved;
/// onto the bare desktop each gets its place, keeping how they lay to
/// each other. A smart zone and a locked one take nothing.
pub fn dropped(layout: &Layout, items: &[(ItemId, Vec2)], at: Pos2) -> Vec<Act> {
    match layout.target(at) {
        Target::Zone { locked: true, .. } | Target::Zone { fills: Fills::Query, .. } | Target::Nowhere => Vec::new(),
        Target::Zone { zone, cell, index, .. } => {
            let there: Vec<&ItemId> = layout
                .zone(&zone)
                .map(|shown| shown.items.iter().map(|item| &item.item).collect())
                .unwrap_or_default();
            items
                .iter()
                .enumerate()
                // Dropped where it is, on no place of its own: nothing to do.
                .filter(|(_, (item, _))| !(there.contains(&item) && cell.is_none() && index.is_none()))
                .map(|(number, (item, _))| {
                    Act::Do(Command::AddItem {
                        zone: zone.clone(),
                        item: item.clone(),
                        // The first goes under the pointer, the others after it.
                        index: index.map(|index| index + number),
                        cell: cell.filter(|_| number == 0),
                    })
                })
                .collect()
        }
        Target::Desk { display, origin } => items
            .iter()
            .map(|(item, corner)| {
                let place = at + *corner - origin;
                Act::Do(Command::PlaceLooseItem {
                    item: item.clone(),
                    display: display.clone(),
                    x: place.x.round() as i32,
                    y: place.y.round() as i32,
                })
            })
            .collect(),
    }
}

/// Where carried items are drawn while they are carried.
pub fn carried(items: &[(ItemId, Vec2)], at: Pos2, layout: &Layout) -> Vec<(ItemId, Rect)> {
    items
        .iter()
        .filter_map(|(item, corner)| {
            let size = layout.item(item)?.rect.size();
            Some((item.clone(), Rect::from_min_size(pos2(at.x + corner.x, at.y + corner.y), size)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::view::tests::Desk;
    use super::*;
    use gevrek_zones::placement::Cell;

    fn at(x: f32, y: f32) -> Pointer {
        Pointer { at: Some(pos2(x, y)), ..Pointer::default() }
    }

    fn press(x: f32, y: f32) -> Pointer {
        Pointer { pressed: true, down: true, ..at(x, y) }
    }

    fn hold(x: f32, y: f32) -> Pointer {
        Pointer { down: true, ..at(x, y) }
    }

    fn release(x: f32, y: f32) -> Pointer {
        Pointer { released: true, ..at(x, y) }
    }

    fn commands(acts: Vec<Act>) -> Vec<Command> {
        acts.into_iter()
            .map(|act| match act {
                Act::Do(command) => command,
                other => panic!("{other:?} is no command"),
            })
            .collect()
    }

    /// Three files in a zone at (100, 100) 400 by 300, two loose ones.
    fn desk() -> (Desk, ZoneId, Vec<ItemId>, Vec<ItemId>) {
        let mut desk = Desk::new();
        let zone = desk.zone("Papers", (100, 100, 400, 300));
        let inside = ["a.txt", "b.txt", "c.txt"].map(|name| desk.file(name, Some(&zone))).to_vec();
        let loose = ["x.txt", "y.txt"].map(|name| desk.file(name, None)).to_vec();
        (desk, zone, inside, loose)
    }

    fn centre(desk: &Desk, item: &ItemId) -> Pos2 {
        desk.layout().item(item).unwrap().rect.center()
    }

    #[test]
    fn a_zone_is_moved_by_its_title_and_only_after_a_real_movement() {
        let (desk, zone, ..) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        assert_eq!(gestures.pointer(&layout, &press(250.0, 112.0)), []);
        // A tremble of the hand is no drag.
        assert_eq!(gestures.pointer(&layout, &hold(252.0, 113.0)), []);
        assert_eq!(
            commands(gestures.pointer(&layout, &hold(350.0, 162.0))),
            [Command::MoveZone { zone: zone.clone(), x: 200, y: 150, snap: true }]
        );
        // With Alt nothing pulls.
        let free = Pointer { alt: true, ..hold(351.0, 162.0) };
        assert_eq!(
            commands(gestures.pointer(&layout, &free)),
            [Command::MoveZone { zone: zone.clone(), x: 201, y: 150, snap: false }]
        );
        assert_eq!(gestures.pointer(&layout, &release(351.0, 162.0)), []);
        assert!(!gestures.busy());
        // A second press in quick succession rolls it up.
        let twice = Pointer { double: true, ..press(250.0, 112.0) };
        assert_eq!(
            commands(gestures.pointer(&layout, &twice)),
            [Command::RollUp { zone, rolled_up: None }]
        );
    }

    #[test]
    fn a_locked_zone_is_not_moved() {
        let (mut desk, zone, ..) = desk();
        desk.does(Command::Lock { zone, geometry: Some(true), members: None });
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        gestures.pointer(&layout, &press(250.0, 112.0));
        assert_eq!(gestures.pointer(&layout, &hold(350.0, 162.0)), []);
        assert!(!gestures.busy());
    }

    #[test]
    fn a_zone_is_sized_by_the_side_or_corner_that_was_taken() {
        let (desk, zone, ..) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        gestures.pointer(&layout, &press(498.0, 398.0));
        assert_eq!(
            commands(gestures.pointer(&layout, &hold(548.0, 418.0))),
            [Command::ResizeZone {
                zone: zone.clone(),
                rect: gevrek_zones::Rect::new(100, 100, 450, 320),
                edges: Edges::BOTTOM_RIGHT,
                snap: true,
            }]
        );
        gestures.pointer(&layout, &release(548.0, 418.0));
        // By the left side: the right one stays.
        gestures.pointer(&layout, &press(101.0, 250.0));
        assert_eq!(
            commands(gestures.pointer(&layout, &hold(61.0, 250.0))),
            [Command::ResizeZone {
                zone,
                rect: gevrek_zones::Rect::new(60, 100, 440, 300),
                edges: Edges { left: true, ..Edges::default() },
                snap: true,
            }]
        );
    }

    #[test]
    fn items_are_selected_one_several_and_in_a_row() {
        let (desk, _, inside, loose) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        let click = |gestures: &mut Gestures, item: &ItemId, with: Pointer| {
            let place = centre(&desk, item);
            gestures.pointer(&layout, &Pointer { at: Some(place), pressed: true, down: true, ..with });
            gestures.pointer(&layout, &Pointer { at: Some(place), released: true, ..with });
        };
        let plain = Pointer::default();
        let ctrl = Pointer { ctrl: true, ..plain };
        let shift = Pointer { shift: true, ..plain };
        let selected = |gestures: &Gestures| gestures.selected.iter().cloned().collect::<Vec<_>>();
        click(&mut gestures, &inside[0], plain);
        assert_eq!(selected(&gestures), [inside[0].clone()]);
        click(&mut gestures, &inside[2], ctrl);
        assert_eq!(selected(&gestures), [inside[0].clone(), inside[2].clone()]);
        click(&mut gestures, &inside[0], ctrl);
        assert_eq!(selected(&gestures), [inside[2].clone()]);
        // Shift: from the one last pressed (the first) to this one.
        click(&mut gestures, &inside[2], shift);
        assert_eq!(selected(&gestures), inside);
        // A plain click on one of them leaves it alone, when the button comes up.
        let place = centre(&desk, &inside[1]);
        gestures.pointer(&layout, &Pointer { at: Some(place), pressed: true, down: true, ..plain });
        assert_eq!(gestures.selected.len(), 3, "still all, in case they are dragged");
        gestures.pointer(&layout, &Pointer { at: Some(place), released: true, ..plain });
        assert_eq!(selected(&gestures), [inside[1].clone()]);
        // A press on the bare desktop lets go of all; with Ctrl it keeps them.
        click(&mut gestures, &loose[0], ctrl);
        assert_eq!(gestures.selected.len(), 2);
        gestures.pointer(&layout, &Pointer { ctrl: true, ..press(900.0, 900.0) });
        assert_eq!(gestures.selected.len(), 2);
        gestures.pointer(&layout, &release(900.0, 900.0));
        gestures.pointer(&layout, &press(900.0, 900.0));
        assert!(gestures.selected.is_empty());
    }

    #[test]
    fn a_rectangle_drawn_on_the_desktop_selects_what_it_touches() {
        let (desk, _, inside, loose) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        gestures.pointer(&layout, &press(900.0, 900.0));
        gestures.pointer(&layout, &hold(300.0, 150.0));
        assert_eq!(gestures.band(), Some(Rect::from_two_pos(pos2(900.0, 900.0), pos2(300.0, 150.0))));
        // The third file of the zone is in the rectangle's reach, the first is not.
        assert!(gestures.selected.contains(&inside[2]) && !gestures.selected.contains(&inside[0]));
        gestures.pointer(&layout, &hold(0.0, 0.0));
        assert_eq!(gestures.selected.len(), inside.len() + loose.len());
        // Smaller again: what it no longer touches is let go.
        gestures.pointer(&layout, &hold(850.0, 850.0));
        assert!(gestures.selected.is_empty());
        assert_eq!(gestures.pointer(&layout, &release(850.0, 850.0)), []);
        assert_eq!(gestures.band(), None);
    }

    #[test]
    fn carried_items_go_where_they_are_let_go() {
        let (mut desk, zone, inside, loose) = desk();
        let other = desk.zone("Other", (700, 100, 400, 300));
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        let carry = |gestures: &mut Gestures, item: &ItemId, to: Pos2| {
            let from = centre(&desk, item);
            gestures.pointer(&layout, &press(from.x, from.y));
            gestures.pointer(&layout, &hold(to.x, to.y));
            assert!(matches!(gestures.doing, Doing::Carrying { .. }));
            commands(gestures.pointer(&layout, &release(to.x, to.y)))
        };
        // Onto a cell of the other zone.
        assert_eq!(
            carry(&mut gestures, &inside[0], pos2(708.0 + 85.0, 136.0 + 90.0)),
            [Command::AddItem {
                zone: other.clone(),
                item: inside[0].clone(),
                index: None,
                cell: Some(Cell { column: 1, row: 1 })
            }]
        );
        // Onto its title: into it, at no place in particular.
        assert_eq!(
            carry(&mut gestures, &loose[0], pos2(800.0, 112.0)),
            [Command::AddItem { zone: other.clone(), item: loose[0].clone(), index: None, cell: None }]
        );
        // Onto the bare desktop: where its corner is, from the display's.
        let from = centre(&desk, &inside[1]);
        let corner = desk.layout().item(&inside[1]).unwrap().rect.min;
        assert_eq!(
            carry(&mut gestures, &inside[1], pos2(900.0, 800.0)),
            [Command::PlaceLooseItem {
                item: inside[1].clone(),
                display: DisplayId::new("fake-1"),
                x: (900.0 + corner.x - from.x) as i32,
                y: (800.0 + corner.y - from.y) as i32,
            }]
        );
        // Within its own zone, onto another cell: it is put there.
        assert_eq!(
            carry(&mut gestures, &inside[2], pos2(108.0 + 5.0, 136.0 + 2.0 * 86.0 + 5.0)),
            [Command::AddItem {
                zone: zone.clone(),
                item: inside[2].clone(),
                index: None,
                cell: Some(Cell { column: 0, row: 2 })
            }]
        );
        // Several at once keep how they lie to each other on the desktop.
        gestures.selected = BTreeSet::from([loose[0].clone(), loose[1].clone()]);
        let from = centre(&desk, &loose[0]);
        gestures.pointer(&layout, &press(from.x, from.y));
        gestures.pointer(&layout, &hold(1300.0, 700.0));
        let placed = commands(gestures.pointer(&layout, &release(1300.0, 700.0)));
        let Command::PlaceLooseItem { y: first, .. } = placed[0] else { panic!() };
        let Command::PlaceLooseItem { y: second, .. } = placed[1] else { panic!() };
        assert_eq!(second - first, 86, "one cell below, as before");
    }

    #[test]
    fn a_zone_that_takes_nothing_gets_nothing() {
        let (mut desk, zone, _, loose) = desk();
        desk.does(Command::Lock { zone: zone.clone(), geometry: None, members: Some(true) });
        let layout = desk.layout();
        let items = [(loose[0].clone(), Vec2::ZERO)];
        assert_eq!(dropped(&layout, &items, pos2(300.0, 300.0)), []);
        assert_eq!(dropped(&layout, &items, pos2(-10.0, 5.0)), []);
    }

    #[test]
    fn items_dragged_off_our_surfaces_are_handed_to_the_system() {
        let (desk, _, inside, _) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        let from = centre(&desk, &inside[0]);
        gestures.pointer(&layout, &press(from.x, from.y));
        gestures.pointer(&layout, &hold(from.x + 40.0, from.y));
        let gone = Pointer { at: None, down: true, ..Pointer::default() };
        assert_eq!(gestures.pointer(&layout, &gone), [Act::DragOut(vec![inside[0].clone()])]);
        assert!(!gestures.busy());
        // The release that follows is nobody's.
        assert_eq!(gestures.pointer(&layout, &Pointer { released: true, ..Pointer::default() }), []);
    }

    #[test]
    fn a_double_click_opens_and_the_other_button_asks_for_a_menu() {
        let (desk, zone, inside, _) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        let place = centre(&desk, &inside[1]);
        let twice = Pointer { double: true, ..press(place.x, place.y) };
        assert_eq!(gestures.pointer(&layout, &twice), [Act::Open(vec![inside[1].clone()])]);
        // The menu of an item outside the selection is of that item alone.
        gestures.selected = BTreeSet::from([inside[0].clone(), inside[2].clone()]);
        let menu = |x: f32, y: f32| Pointer { menu: true, ..at(x, y) };
        assert_eq!(
            gestures.pointer(&layout, &menu(place.x, place.y)),
            [Act::ItemMenu { items: vec![inside[1].clone()], at: place }]
        );
        // Of one in the selection, of all of it, in the order shown.
        gestures.selected = BTreeSet::from([inside[2].clone(), inside[1].clone()]);
        assert_eq!(
            gestures.pointer(&layout, &menu(place.x, place.y)),
            [Act::ItemMenu { items: vec![inside[1].clone(), inside[2].clone()], at: place }]
        );
        assert_eq!(
            gestures.pointer(&layout, &menu(250.0, 112.0)),
            [Act::ZoneMenu { zone: zone.clone(), at: pos2(250.0, 112.0) }]
        );
        assert_eq!(
            gestures.pointer(&layout, &menu(900.0, 900.0)),
            [Act::DeskMenu { display: DisplayId::new("fake-1"), at: pos2(900.0, 900.0) }]
        );
        // Twice on the bare desktop: everything is hidden, or shown again.
        let twice = Pointer { double: true, ..press(900.0, 900.0) };
        assert_eq!(commands(gestures.pointer(&layout, &twice)), [Command::QuickHide { hidden: None }]);
    }

    #[test]
    fn the_keys_open_select_and_step() {
        let (desk, _, inside, loose) = desk();
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        assert_eq!(gestures.key(&layout, Key::Enter), []);
        // With nothing in hand, a step goes to the first item there is.
        gestures.key(&layout, Key::Step(1, 0));
        assert_eq!(gestures.focus.as_ref(), Some(&inside[0]));
        gestures.key(&layout, Key::Step(1, 0));
        gestures.key(&layout, Key::Step(1, 0));
        assert_eq!(gestures.focus.as_ref(), Some(&inside[2]));
        // No further than there is.
        gestures.key(&layout, Key::Step(1, 0));
        assert_eq!(gestures.focus.as_ref(), Some(&inside[2]));
        gestures.key(&layout, Key::Step(-1, 0));
        assert_eq!(gestures.focus.as_ref(), Some(&inside[1]));
        assert_eq!(gestures.key(&layout, Key::Enter), [Act::Open(vec![inside[1].clone()])]);
        gestures.key(&layout, Key::SelectAll);
        assert_eq!(gestures.selected.len(), inside.len(), "those of its zone");
        gestures.key(&layout, Key::Escape);
        assert!(gestures.selected.is_empty());
        gestures.focus = None;
        gestures.key(&layout, Key::SelectAll);
        assert_eq!(gestures.selected.len(), inside.len() + loose.len());
        // What is no longer shown is no longer selected.
        let empty = Layout::default();
        gestures.settle(&empty);
        assert!(gestures.selected.is_empty());
    }
}
