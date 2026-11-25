//! What the pointer and the keys mean: from where they are and what they
//! do, to what is asked of the zones. Everything that changes the zones
//! comes out as a `Command`, the same one a key or another program would
//! give; what does not (a menu, a drag out of the desktop, a zone saying
//! that it is held) comes out as what to do. Nothing here draws.
//!
//! Which items are chosen and what has the keyboard is the core's to
//! keep: a press says so with a command, and what is chosen is read from
//! the layout the next time. Only what the pointer is in the middle of is
//! kept here.

use super::view::{Button, Fills, Hit, Layout, Nav, Target};
use egui::{pos2, vec2, Pos2, Rect, Vec2};
use gevrek_zones::config::{DeskGesture, ExpandOn, View};
use gevrek_zones::core::{Command, FOCUSED};
use gevrek_zones::focus::{Direction, SelectMode, Step};
use gevrek_zones::snap::Edges;
use gevrek_zones::{DisplayId, ItemId, ZoneId};
use std::collections::BTreeSet;

/// How far the pointer goes with a button held before it is a drag.
const SLACK: f32 = 4.0;
/// How far above or below its row a tab is pulled before it comes off.
const TEAR: f32 = 14.0;
/// Where a tab that came off is held by: this far into its new title.
const HELD_AT: Vec2 = vec2(48.0, 13.0);

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
    /// The middle button went down for the second time in quick
    /// succession. (Its single press means nothing to the zones.)
    pub middle_double: bool,
    /// The other button went down.
    pub menu: bool,
    pub ctrl: bool,
    pub shift: bool,
    /// Held while moving or sizing: nothing pulls, and a zone let go on
    /// another does not become its tab.
    pub alt: bool,
}

/// A key that means something to the zones, when they have the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Open what is chosen.
    Enter,
    /// Let go of whatever is being done; then what the core makes of it
    /// (`Command::Dismiss`).
    Escape,
    SelectAll,
    /// To the next item this way; with `extend`, chosen as well.
    Step {
        step: Step,
        extend: bool,
    },
    /// The keyboard to the zone that lies this way.
    Zone(Direction),
    /// What is chosen into the zone that lies this way.
    Carry(Direction),
    /// The next tab of the zone that has the keyboard (or the one before).
    NextTab {
        backwards: bool,
    },
    /// Roll the zone that has the keyboard up, or down again.
    Roll,
    /// Put what is chosen into the bin (the zones do it, as a command).
    Delete,
    /// Give the one chosen item another name.
    Rename,
    /// Take back the last letter of what is typed (into a zone's filter,
    /// or into what is looked for everywhere).
    Erase,
    /// Take back the last thing done to a file (moved, renamed, put into
    /// the bin, a folder made).
    Undo,
    /// The menu of what has the keyboard: the chosen items, else the
    /// zone, else the desktop (the key for menus, or Shift and F10).
    Menu,
    /// To the next button of the title of the zone that has the keyboard
    /// (the one before, `backwards`); past the last, back to the items.
    Title {
        backwards: bool,
    },
    /// Press the button of the title the keyboard is on.
    Space,
}

/// What follows from the pointer or a key.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Ask the zones for something.
    Do(Command),
    /// The menu of some items, at a point of the desktop. `ours`: the
    /// zones' own lines are asked for (Shift was held), where the menu
    /// would otherwise be the system's.
    ItemMenu {
        items: Vec<ItemId>,
        at: Pos2,
        ours: bool,
    },
    ZoneMenu {
        zone: ZoneId,
        at: Pos2,
    },
    /// The menu of how a zone shows and orders what it holds, alone.
    ViewMenu {
        zone: ZoneId,
        at: Pos2,
    },
    DeskMenu {
        display: DisplayId,
        at: Pos2,
    },
    /// The items are dragged off our surfaces: the system carries on.
    DragOut(Vec<ItemId>),
    /// A zone did not let itself be moved, sized or filled: it says so
    /// (it is held where it is, or lets nothing in).
    Refuse(ZoneId),
    /// Files go into a folder that is shown as an item (of a portal).
    IntoFolder {
        items: Vec<ItemId>,
        folder: ItemId,
    },
    /// What a zone shows that is not of the desktop (files of a portal's
    /// folder, of the downloads) is brought to the desktop: into a zone
    /// filled by hand, or to a place of its own. `copy`: it is left
    /// where it is as well (what a zone only found is not taken away).
    BringOut {
        items: Vec<ItemId>,
        zone: Option<ZoneId>,
        at: Option<gevrek_zones::placement::LoosePosition>,
        copy: bool,
    },
}

/// What is being done with the button held.
#[derive(Debug, Clone, PartialEq)]
pub enum Doing {
    Nothing,
    /// Pressed, and not yet moved far enough to be a drag.
    Pressed {
        hit: Hit,
        at: Pos2,
        /// The item was one of several chosen: a click (not a drag) then
        /// leaves it the only one.
        narrow: bool,
    },
    /// A zone is moved by its title; `grab` is from its corner to the
    /// pointer. `onto`: the zone it would become a tab of, let go here.
    Moving {
        zone: ZoneId,
        grab: Vec2,
        onto: Option<ZoneId>,
    },
    /// A zone is sized; `from` is what it was, `at` where the pointer took hold.
    Sizing {
        zone: ZoneId,
        edges: Edges,
        from: Rect,
        at: Pos2,
    },
    /// A tab is dragged along its row, to another place among its tabs.
    Tabbing {
        zone: ZoneId,
    },
    /// Items are carried; each with where its corner is from the pointer.
    Carrying {
        items: Vec<(ItemId, Vec2)>,
        at: Pos2,
    },
    /// A rectangle is drawn to choose with; `kept` was chosen before,
    /// `chosen` is what was last said to be.
    Banding {
        from: Pos2,
        to: Pos2,
        kept: BTreeSet<ItemId>,
        chosen: BTreeSet<ItemId>,
    },
}

/// The pointer's and the keys' own state: what is under way.
#[derive(Debug, Clone, PartialEq)]
pub struct Gestures {
    pub doing: Doing,
    /// The item a range with Shift begins at, and the keys go from among
    /// the loose items (which the core does not walk).
    pub anchor: Option<ItemId>,
    /// The button of a zone's title the keyboard is on, while it is on
    /// one (Tab goes along them).
    pub title_at: Option<(ZoneId, Button)>,
}

impl Default for Gestures {
    fn default() -> Self {
        Self {
            doing: Doing::Nothing,
            anchor: None,
            title_at: None,
        }
    }
}

/// What pressing the bare desktop twice with the middle button does, as
/// the configuration says.
fn desk_gesture(gesture: DeskGesture) -> Vec<Act> {
    match gesture {
        DeskGesture::None => Vec::new(),
        DeskGesture::QuickHide => vec![Act::Do(Command::QuickHide { hidden: None })],
        DeskGesture::GreyIcons => vec![Act::Do(Command::GreyIcons { on: None })],
        DeskGesture::Peek => vec![Act::Do(Command::TogglePeek)],
    }
}

/// What a button of a zone's title does when it is pressed (by the
/// pointer, by a key, or by a screen reader on the person's behalf).
pub fn button_pressed(layout: &Layout, zone: &ZoneId, button: Button) -> Vec<Act> {
    let Some(shown) = layout.zone(zone) else {
        return Vec::new();
    };
    let id = || zone.clone();
    // Menus open under the button they were asked for by.
    let under = shown
        .buttons
        .iter()
        .find(|placed| placed.button == button)
        .map_or(shown.head.left_bottom(), |placed| placed.rect.left_bottom());
    match button {
        Button::Roll => vec![Act::Do(Command::RollUp {
            zone: id(),
            rolled_up: None,
        })],
        Button::Lock => {
            // One step on: held where it is; then nothing in or out as
            // well; then neither. (From anything else, to neither.)
            let (geometry, members) = match shown.own_lock {
                (false, false) => (true, false),
                (true, false) => (true, true),
                _ => (false, false),
            };
            vec![Act::Do(Command::Lock {
                zone: id(),
                geometry: Some(geometry),
                members: Some(members),
            })]
        }
        Button::View => {
            let other = if shown.view == View::Grid {
                View::List
            } else {
                View::Grid
            };
            vec![Act::Do(Command::SetView {
                zone: id(),
                view: Some(other),
                sort: None,
                descending: None,
                icon_size: None,
            })]
        }
        Button::Sort => vec![Act::ViewMenu {
            zone: id(),
            at: under,
        }],
        Button::Cover => {
            // Covered: shown. Private and shown: covered again. Not
            // private: made so, which covers it.
            let command = if shown.masked {
                Command::Reveal {
                    zone: Some(id()),
                    revealed: Some(true),
                }
            } else if shown.private {
                Command::Reveal {
                    zone: Some(id()),
                    revealed: Some(false),
                }
            } else {
                Command::ModifyZone {
                    zone: id(),
                    changes: serde_json::json!({ "private": true }),
                }
            };
            vec![Act::Do(command)]
        }
        Button::More => vec![Act::ZoneMenu {
            zone: id(),
            at: under,
        }],
    }
}

fn choose(items: impl IntoIterator<Item = ItemId>, mode: SelectMode) -> Act {
    Act::Do(Command::Select {
        items: items.into_iter().collect(),
        mode,
    })
}

impl Gestures {
    /// Whether something is under way that needs every movement.
    pub fn busy(&self) -> bool {
        self.doing != Doing::Nothing
    }

    /// Whether something is in the hand that could be let go in a zone.
    pub fn carrying(&self) -> bool {
        matches!(self.doing, Doing::Carrying { .. })
    }

    /// The rectangle being drawn to choose with.
    pub fn band(&self) -> Option<Rect> {
        match &self.doing {
            Doing::Banding { from, to, .. } => Some(Rect::from_two_pos(*from, *to)),
            _ => None,
        }
    }

    /// The zone that is dragged would become a tab of this one.
    pub fn tab_onto(&self) -> Option<&ZoneId> {
        match &self.doing {
            Doing::Moving { onto, .. } => onto.as_ref(),
            _ => None,
        }
    }

    /// Forget what is no longer shown (an item went, a zone was rolled up).
    pub fn settle(&mut self, layout: &Layout) {
        if self
            .anchor
            .as_ref()
            .is_some_and(|item| layout.item(item).is_none())
        {
            self.anchor = None;
        }
        // The keyboard is on a button only of the zone that has it.
        let still = |(zone, button): &(ZoneId, Button)| {
            layout.zone(zone).is_some_and(|shown| {
                shown.focused && shown.buttons.iter().any(|placed| placed.button == *button)
            })
        };
        if !self.title_at.as_ref().is_some_and(still) {
            self.title_at = None;
        }
    }

    /// The pointer did something.
    pub fn pointer(&mut self, layout: &Layout, pointer: &Pointer) -> Vec<Act> {
        let mut acts = Vec::new();
        if let Some(at) = pointer.at {
            if pointer.menu {
                self.doing = Doing::Nothing;
                acts.extend(self.menu(layout, at, pointer.shift));
            } else if pointer.pressed {
                acts.extend(self.press(layout, at, pointer));
            }
            // The middle button twice on the bare desktop.
            if pointer.middle_double && matches!(layout.hit(at), Hit::Desk(_)) {
                acts.extend(desk_gesture(layout.twice_middle));
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

    /// Make an item the only chosen one, with the keyboard on it: in
    /// its zone, or among the loose items of its display. (What stays of
    /// a zone that is put away is in no place the keyboard walks: it is
    /// chosen, and that is all.)
    fn only(&mut self, layout: &Layout, item: &ItemId) -> Vec<Act> {
        self.anchor = Some(item.clone());
        let walked = layout.zone_of(item).is_some()
            || layout
                .desks
                .iter()
                .any(|desk| desk.items.iter().any(|shown| &shown.item == item));
        if walked {
            vec![Act::Do(Command::FocusItem {
                item: Some(item.clone()),
            })]
        } else {
            vec![
                Act::Do(Command::FocusZone { zone: None }),
                choose([item.clone()], SelectMode::Replace),
            ]
        }
    }

    fn menu(&mut self, layout: &Layout, at: Pos2, ours: bool) -> Vec<Act> {
        match layout.hit(at) {
            Hit::Item { item, .. } => {
                // The menu is of what is chosen; an item that is not
                // among it becomes the only chosen one.
                let chosen = layout.selected();
                if chosen.contains(&item) {
                    self.anchor = Some(item);
                    vec![Act::ItemMenu {
                        items: chosen,
                        at,
                        ours,
                    }]
                } else {
                    let mut acts = self.only(layout, &item);
                    acts.push(Act::ItemMenu {
                        items: vec![item],
                        at,
                        ours,
                    });
                    acts
                }
            }
            Hit::Head(zone)
            | Hit::Nav(zone, _)
            | Hit::Button(zone, _)
            | Hit::Tab(zone)
            | Hit::Edge(zone, _)
            | Hit::Body(zone) => {
                vec![Act::ZoneMenu { zone, at }]
            }
            Hit::Desk(display) => vec![Act::DeskMenu { display, at }],
            Hit::Nothing => Vec::new(),
        }
    }

    fn press(&mut self, layout: &Layout, at: Pos2, pointer: &Pointer) -> Vec<Act> {
        let hit = layout.hit(at);
        let mut acts = Vec::new();
        let mut narrow = false;
        // (The pointer has taken over from the keys.)
        self.title_at = None;
        match &hit {
            Hit::Item { item, .. } => {
                let chosen = layout.selected();
                let is_chosen = chosen.contains(item);
                if pointer.double {
                    self.doing = Doing::Nothing;
                    let items = if is_chosen {
                        chosen
                    } else {
                        vec![item.clone()]
                    };
                    let mut acts = if is_chosen {
                        Vec::new()
                    } else {
                        self.only(layout, item)
                    };
                    acts.push(Act::Do(Command::Open {
                        items,
                        with: None,
                        in_place: true,
                    }));
                    return acts;
                }
                if pointer.shift {
                    // From the item last pressed to this one, among those
                    // shown with it.
                    let beside = layout.beside(item);
                    let place =
                        |wanted: &ItemId| beside.iter().position(|shown| &shown.item == wanted);
                    let from = self
                        .anchor
                        .as_ref()
                        .and_then(place)
                        .or_else(|| place(item))
                        .unwrap_or(0);
                    let to = place(item).unwrap_or(from);
                    let last = from.max(to).min(beside.len().saturating_sub(1));
                    let range = beside[from.min(to)..=last]
                        .iter()
                        .map(|shown| shown.item.clone());
                    acts.push(choose(
                        range,
                        if pointer.ctrl {
                            SelectMode::Add
                        } else {
                            SelectMode::Replace
                        },
                    ));
                } else if pointer.ctrl {
                    acts.push(choose([item.clone()], SelectMode::Toggle));
                    self.anchor = Some(item.clone());
                } else if is_chosen {
                    // (Still all, in case they are dragged.)
                    narrow = chosen.len() > 1;
                    self.anchor = Some(item.clone());
                } else {
                    acts.extend(self.only(layout, item));
                }
            }
            Hit::Head(zone) if pointer.double => {
                self.doing = Doing::Nothing;
                return vec![Act::Do(Command::RollUp {
                    zone: zone.clone(),
                    rolled_up: None,
                })];
            }
            Hit::Tab(zone) => acts.push(Act::Do(Command::ActivateTab { zone: zone.clone() })),
            Hit::Desk(_) if pointer.double => {
                self.doing = Doing::Nothing;
                return vec![Act::Do(Command::QuickHide { hidden: None })];
            }
            Hit::Body(zone) if !(pointer.ctrl || pointer.shift) => {
                // The zone has the keyboard, and nothing in it is chosen.
                acts.push(Act::Do(Command::FocusZone {
                    zone: Some(zone.clone()),
                }));
                if !layout.selected().is_empty() {
                    acts.push(choose([], SelectMode::Replace));
                }
                self.anchor = None;
            }
            Hit::Desk(_) if !(pointer.ctrl || pointer.shift) => {
                acts.push(Act::Do(Command::FocusZone { zone: None }));
                self.anchor = None;
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
        let mut acts = Vec::new();
        if let Doing::Pressed { hit, at: from, .. } = &self.doing {
            if (at - *from).length() < SLACK {
                return Vec::new();
            }
            let from = *from;
            // A zone that is held where it is says so, once, and stays.
            let held = |zone: &ZoneId| {
                layout
                    .shown_for(zone)
                    .is_none_or(|shown| shown.locked_place)
            };
            self.doing = match hit.clone() {
                Hit::Head(zone)
                | Hit::Nav(zone, _)
                | Hit::Button(zone, _)
                | Hit::Tab(zone)
                | Hit::Edge(zone, _)
                    if held(&zone) =>
                {
                    acts.push(Act::Refuse(zone));
                    Doing::Nothing
                }
                // (A title is a title wherever it is taken: dragged by a
                // part of its way, the zone is moved all the same.)
                Hit::Head(zone) | Hit::Nav(zone, _) | Hit::Button(zone, _) => {
                    match layout.zone(&zone) {
                        Some(shown) => Doing::Moving {
                            zone,
                            grab: from - shown.rect.min,
                            onto: None,
                        },
                        None => Doing::Nothing,
                    }
                }
                Hit::Tab(zone) => Doing::Tabbing { zone },
                Hit::Edge(zone, edges) => match layout.zone(&zone) {
                    Some(shown) => Doing::Sizing {
                        zone,
                        edges,
                        from: shown.rect,
                        at: from,
                    },
                    None => Doing::Nothing,
                },
                Hit::Item { .. } => Doing::Carrying {
                    items: layout
                        .selected()
                        .into_iter()
                        .filter_map(|item| {
                            let corner = layout.item(&item)?.rect.min;
                            Some((item, corner - from))
                        })
                        .collect(),
                    at,
                },
                Hit::Body(_) | Hit::Desk(_) => {
                    let kept: BTreeSet<ItemId> = if pointer.ctrl || pointer.shift {
                        layout.selected().into_iter().collect()
                    } else {
                        BTreeSet::new()
                    };
                    Doing::Banding {
                        from,
                        to: at,
                        chosen: kept.clone(),
                        kept,
                    }
                }
                Hit::Nothing => Doing::Nothing,
            };
        }
        // A tab pulled off its row is a zone of its own from there on,
        // held by its new title.
        if let Doing::Tabbing { zone } = &self.doing {
            let row = layout.shown_for(zone).and_then(|shown| {
                shown
                    .tabs
                    .first()
                    .map(|tab| (tab.rect.top(), tab.rect.bottom()))
            });
            let off = row.is_none_or(|(top, bottom)| at.y < top - TEAR || at.y > bottom + TEAR);
            if off {
                let corner = at - HELD_AT;
                acts.push(Act::Do(Command::SplitTab {
                    zone: zone.clone(),
                    x: corner.x.round() as i32,
                    y: corner.y.round() as i32,
                    snap: false,
                }));
                self.doing = Doing::Moving {
                    zone: zone.clone(),
                    grab: HELD_AT,
                    onto: None,
                };
                return acts;
            }
        }
        match &mut self.doing {
            Doing::Moving { zone, grab, onto } => {
                let corner = at - *grab;
                // Over another zone's title it would become its tab; with
                // Alt it is only moved.
                let own = layout.together(zone);
                *onto = if pointer.alt {
                    None
                } else {
                    layout
                        .tab_target(at, &own)
                        .map(|target| target.zone.clone())
                };
                acts.push(Act::Do(Command::MoveZone {
                    zone: zone.clone(),
                    x: corner.x.round() as i32,
                    y: corner.y.round() as i32,
                    // (About to become a tab, it is not pulled elsewhere.)
                    snap: !pointer.alt && onto.is_none(),
                }));
            }
            Doing::Sizing {
                zone,
                edges,
                from,
                at: held,
            } => {
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
                acts.push(Act::Do(Command::ResizeZone {
                    zone: zone.clone(),
                    rect: gevrek_zones::Rect::new(
                        rect.min.x.round() as i32,
                        rect.min.y.round() as i32,
                        rect.width().round() as i32,
                        rect.height().round() as i32,
                    ),
                    edges: *edges,
                    snap: !pointer.alt,
                }));
            }
            Doing::Tabbing { zone } => {
                // Along its row: to the place the pointer is at.
                let now = layout
                    .shown_for(zone)
                    .and_then(|shown| shown.tabs.iter().position(|tab| &tab.zone == zone));
                if let (Some(now), Some(wanted)) = (now, layout.tab_index(zone, at)) {
                    if now != wanted {
                        acts.push(Act::Do(Command::MoveTab {
                            zone: zone.clone(),
                            index: wanted,
                        }));
                    }
                }
            }
            Doing::Carrying { at: now, .. } => *now = at,
            Doing::Banding {
                from,
                to,
                kept,
                chosen,
            } => {
                *to = at;
                let mut touched = kept.clone();
                touched.extend(layout.items_in(Rect::from_two_pos(*from, at)));
                // (Said only when it is other than last said.)
                if touched != *chosen {
                    *chosen = touched.clone();
                    acts.push(choose(touched, SelectMode::Replace));
                }
            }
            Doing::Nothing | Doing::Pressed { .. } => {}
        }
        acts
    }

    fn release(&mut self, layout: &Layout, pointer: &Pointer) -> Vec<Act> {
        match std::mem::replace(&mut self.doing, Doing::Nothing) {
            // A click on one of several chosen items leaves it alone chosen.
            Doing::Pressed {
                hit: Hit::Item { item, .. },
                narrow: true,
                ..
            } => self.only(layout, &item),
            // A click on a title: the zone has the keyboard; one that is
            // rolled up and opens at a press opens (or shuts again).
            Doing::Pressed {
                hit: Hit::Head(zone),
                ..
            } => {
                let opens = layout
                    .zone(&zone)
                    .is_some_and(|shown| shown.rolled_up && shown.expand_on == ExpandOn::Click);
                let mut acts = vec![Act::Do(Command::FocusZone {
                    zone: Some(zone.clone()),
                })];
                if opens {
                    acts.push(Act::Do(Command::Expand {
                        zone,
                        expanded: None,
                    }));
                }
                acts
            }
            // A click on a button of a title (let go on the button it
            // went down on): what the button does.
            Doing::Pressed {
                hit: Hit::Button(zone, button),
                at,
                ..
            } => {
                let still = pointer
                    .at
                    .is_none_or(|now| layout.hit(now) == Hit::Button(zone.clone(), button));
                if still && layout.hit(at) == Hit::Button(zone.clone(), button) {
                    button_pressed(layout, &zone, button)
                } else {
                    Vec::new()
                }
            }
            // A click on a part of a portal's way: there it goes.
            Doing::Pressed {
                hit: Hit::Nav(zone, nav),
                ..
            } => vec![Act::Do(match nav {
                Nav::Back => Command::NavigateBack { zone },
                Nav::Up => Command::NavigateUp { zone },
                Nav::To(directory) => Command::NavigateTo { zone, directory },
            })],
            // A click on what covers a zone: the person asks to see it.
            Doing::Pressed {
                hit: Hit::Body(zone),
                ..
            } if layout.zone(&zone).is_some_and(|shown| shown.masked) => {
                vec![Act::Do(Command::Reveal {
                    zone: Some(zone),
                    revealed: Some(true),
                })]
            }
            // A click in a zone whose folder (or whatever it looks
            // through) is not there now: ask again.
            Doing::Pressed {
                hit: Hit::Body(zone),
                ..
            } if layout.zone(&zone).is_some_and(|shown| {
                matches!(
                    shown.listing,
                    Some(gevrek_zones::portal::Listing::Unavailable { .. })
                )
            }) =>
            {
                vec![Act::Do(Command::RefreshZone { zone })]
            }
            Doing::Moving {
                zone,
                onto: Some(with),
                ..
            } => vec![Act::Do(Command::Tab { zone, with })],
            Doing::Carrying { items, at } => dropped(layout, &items, pointer.at.unwrap_or(at)),
            _ => Vec::new(),
        }
    }

    /// A key was pressed while the zones have the keyboard.
    pub fn key(&mut self, layout: &Layout, key: Key) -> Vec<Act> {
        let in_a_zone = layout.zones.iter().any(|zone| zone.focused);
        // The keyboard is somewhere the core walks: a zone, or the loose
        // items of a display.
        let somewhere = in_a_zone || layout.desks.iter().any(|desk| desk.focused);
        let focused = || ZoneId::new(FOCUSED);
        // Any key but the ones that are for a title's buttons takes the
        // keyboard off them.
        let on_title = self.title_at.clone();
        if !matches!(
            key,
            Key::Title { .. } | Key::Enter | Key::Space | Key::Escape | Key::Menu
        ) {
            self.title_at = None;
        }
        match key {
            Key::Title { backwards } => {
                let Some(shown) = layout.zones.iter().find(|zone| zone.focused) else {
                    return Vec::new();
                };
                let now = on_title.and_then(|(_, button)| {
                    shown
                        .buttons
                        .iter()
                        .position(|placed| placed.button == button)
                });
                let next = match (now, backwards) {
                    (None, false) => (!shown.buttons.is_empty()).then_some(0),
                    (None, true) => shown.buttons.len().checked_sub(1),
                    (Some(at), false) => (at + 1 < shown.buttons.len()).then_some(at + 1),
                    (Some(at), true) => at.checked_sub(1),
                };
                self.title_at = next.map(|at| (shown.zone.clone(), shown.buttons[at].button));
                Vec::new()
            }
            Key::Enter | Key::Space if on_title.is_some() => match on_title {
                Some((zone, button)) => button_pressed(layout, &zone, button),
                None => Vec::new(),
            },
            Key::Space => Vec::new(),
            Key::Escape if on_title.is_some() => {
                self.title_at = None;
                Vec::new()
            }
            Key::Menu => {
                // Of the title's button: the zone's. Of what is chosen
                // where the keyboard is; else of the zone that has it;
                // else of the desktop.
                let zone = layout.zones.iter().find(|zone| zone.focused);
                let chosen = layout.selected();
                let on_item = layout
                    .every_item()
                    .find(|shown| shown.focused)
                    .or_else(|| layout.every_item().find(|shown| shown.selected));
                match (on_title, on_item, zone) {
                    (Some((zone, _)), ..) => layout
                        .zone(&zone)
                        .map(|shown| {
                            vec![Act::ZoneMenu {
                                zone,
                                at: shown.head.left_bottom() + vec2(12.0, 0.0),
                            }]
                        })
                        .unwrap_or_default(),
                    (None, Some(item), _) if !chosen.is_empty() => {
                        vec![Act::ItemMenu {
                            items: chosen,
                            at: item.rect.center(),
                            ours: false,
                        }]
                    }
                    (None, _, Some(zone)) => {
                        vec![Act::ZoneMenu {
                            zone: zone.zone.clone(),
                            at: zone.head.left_bottom() + vec2(12.0, 0.0),
                        }]
                    }
                    (None, ..) => layout
                        .desks
                        .iter()
                        .find(|desk| desk.focused)
                        .or(layout.desks.first())
                        .map(|desk| {
                            vec![Act::DeskMenu {
                                display: desk.display.clone(),
                                at: desk.area.min + vec2(24.0, 24.0),
                            }]
                        })
                        .unwrap_or_default(),
                }
            }
            Key::Enter if somewhere => vec![Act::Do(Command::OpenFocused)],
            Key::Enter => match layout.selected() {
                items if items.is_empty() => Vec::new(),
                items => vec![Act::Do(Command::Open {
                    items,
                    with: None,
                    in_place: true,
                })],
            },
            // (These are the zones' own to do; `Zones::key` does them.)
            Key::Delete | Key::Rename | Key::Erase | Key::Undo => Vec::new(),
            Key::Escape => {
                if self.doing == Doing::Nothing {
                    self.anchor = None;
                    vec![Act::Do(Command::Dismiss)]
                } else {
                    self.doing = Doing::Nothing;
                    Vec::new()
                }
            }
            Key::SelectAll => {
                // Those shown with the item last pressed; without one,
                // those of the zone that has the keyboard, else everything.
                let desk = layout.desks.iter().find(|desk| desk.focused);
                let all: Vec<ItemId> = match (
                    &self.anchor,
                    layout.zones.iter().find(|zone| zone.focused),
                    desk,
                ) {
                    (Some(item), ..) => layout
                        .beside(item)
                        .iter()
                        .map(|shown| shown.item.clone())
                        .collect(),
                    (None, Some(zone), _) => {
                        zone.items.iter().map(|shown| shown.item.clone()).collect()
                    }
                    (None, None, Some(desk)) => {
                        desk.items.iter().map(|shown| shown.item.clone()).collect()
                    }
                    (None, None, None) => layout
                        .every_item()
                        .map(|shown| shown.item.clone())
                        .collect(),
                };
                vec![choose(all, SelectMode::Replace)]
            }
            // From item to item, in a zone or among the loose ones: the
            // core walks both (from nowhere, to the first there is).
            Key::Step { step, extend } => {
                self.anchor = None;
                vec![Act::Do(Command::MoveItemFocus { step, extend })]
            }
            Key::Zone(direction) => {
                self.anchor = None;
                vec![Act::Do(Command::FocusNeighbour { direction })]
            }
            Key::Carry(direction) => {
                vec![Act::Do(Command::MoveFocusedItems {
                    direction: Some(direction),
                    zone: None,
                })]
            }
            Key::NextTab { backwards } if in_a_zone => {
                vec![Act::Do(Command::NextTab {
                    zone: focused(),
                    backwards,
                })]
            }
            Key::Roll if in_a_zone => vec![Act::Do(Command::RollUp {
                zone: focused(),
                rolled_up: None,
            })],
            Key::NextTab { .. } | Key::Roll => Vec::new(),
        }
    }
}

/// What letting go of carried items at a point asks for. Into a zone
/// filled by hand they are put (at the cell or line under the pointer
/// where its order is the person's); onto a portal the files are moved;
/// onto the bare desktop each gets its place, keeping how they lay to
/// each other. A zone that fills itself and one that lets nothing in
/// take nothing, and say so.
pub fn dropped(layout: &Layout, items: &[(ItemId, Vec2)], at: Pos2) -> Vec<Act> {
    // Whether the first of them is shown by a zone whose items are not
    // of the desktop, and whether that zone only found them.
    let from = items.first().and_then(|(item, _)| layout.zone_of(item));
    let outside = from.is_some_and(|zone| zone.outside);
    let found = from.is_some_and(|zone| zone.fills == Fills::Query);
    let all = || {
        items
            .iter()
            .map(|(item, _)| item.clone())
            .collect::<Vec<_>>()
    };
    match layout.target(at) {
        Target::Nowhere => Vec::new(),
        // Onto a folder shown in a portal: into that folder.
        Target::Zone {
            into: Some(folder),
            locked: false,
            ..
        } if !items.iter().any(|(item, _)| item == &folder) => {
            vec![Act::IntoFolder {
                items: all(),
                folder,
            }]
        }
        // Out of a portal (or what a zone found elsewhere) into a zone
        // filled by hand: the files come to the desktop, and into it.
        Target::Zone {
            zone,
            locked: false,
            fills: Fills::ByHand,
            ..
        } if outside => {
            vec![Act::BringOut {
                items: all(),
                zone: Some(zone),
                at: None,
                copy: found,
            }]
        }
        Target::Desk { display, origin } if outside => {
            let place = at - origin;
            let at = gevrek_zones::placement::LoosePosition {
                display,
                x: place.x.round() as i32,
                y: place.y.round() as i32,
            };
            vec![Act::BringOut {
                items: all(),
                zone: None,
                at: Some(at),
                copy: found,
            }]
        }
        Target::Zone {
            zone,
            locked,
            fills,
            cell,
            index,
            ..
        } => {
            let there: Vec<&ItemId> = layout
                .zone(&zone)
                .map(|shown| shown.items.iter().map(|item| &item.item).collect())
                .unwrap_or_default();
            // (What only shows what is found has everything it shows
            // already; letting go of its own items on it is nothing.)
            let from_elsewhere = items.iter().any(|(item, _)| !there.contains(&item));
            if locked || fills == Fills::Query {
                return if from_elsewhere || locked {
                    vec![Act::Refuse(zone)]
                } else {
                    Vec::new()
                };
            }
            items
                .iter()
                .enumerate()
                // Dropped where it is, on no place of its own: nothing to do.
                .filter(|(_, (item, _))| {
                    !(there.contains(&item) && cell.is_none() && index.is_none())
                })
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
            Some((
                item.clone(),
                Rect::from_min_size(pos2(at.x + corner.x, at.y + corner.y), size),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::view::tests::Desk;
    use super::*;
    use gevrek_zones::placement::Cell;

    fn at(x: f32, y: f32) -> Pointer {
        Pointer {
            at: Some(pos2(x, y)),
            ..Pointer::default()
        }
    }

    fn press(x: f32, y: f32) -> Pointer {
        Pointer {
            pressed: true,
            down: true,
            ..at(x, y)
        }
    }

    fn hold(x: f32, y: f32) -> Pointer {
        Pointer {
            down: true,
            ..at(x, y)
        }
    }

    fn release(x: f32, y: f32) -> Pointer {
        Pointer {
            released: true,
            ..at(x, y)
        }
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
        let inside = ["a.txt", "b.txt", "c.txt"]
            .map(|name| desk.file(name, Some(&zone)))
            .to_vec();
        let loose = ["x.txt", "y.txt"]
            .map(|name| desk.file(name, None))
            .to_vec();
        (desk, zone, inside, loose)
    }

    fn centre(desk: &Desk, item: &ItemId) -> Pos2 {
        desk.layout().item(item).unwrap().rect.center()
    }

    /// The pointer does something on the desk as it is, and what follows
    /// is carried out, as the zones do it.
    fn act(desk: &mut Desk, gestures: &mut Gestures, pointer: Pointer) -> Vec<Act> {
        let acts = gestures.pointer(&desk.layout(), &pointer);
        for act in &acts {
            if let Act::Do(command) = act {
                desk.tries(command.clone());
            }
        }
        acts
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
            [Command::MoveZone {
                zone: zone.clone(),
                x: 200,
                y: 150,
                snap: true
            }]
        );
        // With Alt nothing pulls.
        let free = Pointer {
            alt: true,
            ..hold(351.0, 162.0)
        };
        assert_eq!(
            commands(gestures.pointer(&layout, &free)),
            [Command::MoveZone {
                zone: zone.clone(),
                x: 201,
                y: 150,
                snap: false
            }]
        );
        assert_eq!(gestures.pointer(&layout, &release(351.0, 162.0)), []);
        assert!(!gestures.busy());
        // A second press in quick succession rolls it up.
        let twice = Pointer {
            double: true,
            ..press(250.0, 112.0)
        };
        assert_eq!(
            commands(gestures.pointer(&layout, &twice)),
            [Command::RollUp {
                zone: zone.clone(),
                rolled_up: None
            }]
        );
        // A click on the title (no drag) gives the zone the keyboard.
        gestures.pointer(&layout, &press(250.0, 112.0));
        assert_eq!(
            commands(gestures.pointer(&layout, &release(250.0, 112.0))),
            [Command::FocusZone { zone: Some(zone) }]
        );
    }

    #[test]
    fn a_zone_that_is_held_where_it_is_says_so_and_stays() {
        let (mut desk, zone, ..) = desk();
        desk.does(Command::Lock {
            zone: zone.clone(),
            geometry: Some(true),
            members: None,
        });
        let layout = desk.layout();
        let mut gestures = Gestures::default();
        // By its title.
        gestures.pointer(&layout, &press(250.0, 112.0));
        assert_eq!(
            gestures.pointer(&layout, &hold(350.0, 162.0)),
            [Act::Refuse(zone.clone())]
        );
        assert!(!gestures.busy());
        // Said once: the rest of the drag is nothing.
        assert_eq!(gestures.pointer(&layout, &hold(360.0, 170.0)), []);
        gestures.pointer(&layout, &release(360.0, 170.0));
        // By its side.
        gestures.pointer(&layout, &press(499.0, 250.0));
        assert_eq!(
            gestures.pointer(&layout, &hold(540.0, 250.0)),
            [Act::Refuse(zone.clone())]
        );
        // The lock for all zones holds it as its own does.
        desk.does(Command::Lock {
            zone: zone.clone(),
            geometry: Some(false),
            members: None,
        });
        desk.does(Command::LockAll {
            geometry: Some(true),
            members: None,
        });
        let layout = desk.layout();
        gestures.pointer(&layout, &press(250.0, 112.0));
        assert_eq!(
            gestures.pointer(&layout, &hold(350.0, 162.0)),
            [Act::Refuse(zone)]
        );
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
                edges: Edges {
                    left: true,
                    ..Edges::default()
                },
                snap: true,
            }]
        );
    }

    #[test]
    fn a_zone_let_go_on_anothers_title_becomes_its_tab() {
        let (mut desk, papers, ..) = desk();
        let other = desk.zone("Other", (700, 100, 400, 300));
        let mut gestures = Gestures::default();
        // Taken by its title and brought over the other's.
        act(&mut desk, &mut gestures, press(250.0, 112.0));
        act(&mut desk, &mut gestures, hold(500.0, 300.0));
        assert_eq!(gestures.tab_onto(), None, "over nothing");
        let over = act(&mut desk, &mut gestures, hold(800.0, 112.0));
        assert_eq!(gestures.tab_onto(), Some(&other));
        // (About to become a tab, nothing else pulls it.)
        assert!(matches!(
            over.as_slice(),
            [Act::Do(Command::MoveZone { snap: false, .. })]
        ));
        // With Alt it is only moved there.
        act(
            &mut desk,
            &mut gestures,
            Pointer {
                alt: true,
                ..hold(801.0, 112.0)
            },
        );
        assert_eq!(gestures.tab_onto(), None);
        act(&mut desk, &mut gestures, hold(800.0, 112.0));
        let let_go = act(&mut desk, &mut gestures, release(800.0, 112.0));
        assert_eq!(
            commands(let_go),
            [Command::Tab {
                zone: papers.clone(),
                with: other.clone()
            }]
        );
        let layout = desk.layout();
        assert_eq!(layout.zones.len(), 1);
        assert_eq!(layout.together(&papers), [papers.clone(), other.clone()]);
        // A zone that is held where it is takes no tab.
        let third = desk.zone("Third", (700, 500, 300, 200));
        desk.does(Command::LockAll {
            geometry: Some(true),
            members: None,
        });
        assert!(desk
            .layout()
            .tab_target(pos2(800.0, 112.0), &[third])
            .is_none());
    }

    #[test]
    fn a_tab_goes_along_its_row_and_comes_off_when_pulled_away() {
        let mut desk = Desk::new();
        let one = desk.zone("One", (100, 100, 400, 300));
        let two = desk.zone("Two", (600, 100, 300, 200));
        let three = desk.zone("Three", (600, 400, 300, 200));
        desk.does(Command::Tab {
            zone: two.clone(),
            with: one.clone(),
        });
        desk.does(Command::Tab {
            zone: three.clone(),
            with: one.clone(),
        });
        let order = |desk: &Desk| -> Vec<ZoneId> { desk.layout().together(&one) };
        assert_eq!(order(&desk), [one.clone(), two.clone(), three.clone()]);
        // Tabs are 133 wide from x = 100, under the title (y 128 to 154).
        let mut gestures = Gestures::default();
        let pressed = act(&mut desk, &mut gestures, press(120.0, 140.0));
        assert_eq!(
            commands(pressed),
            [Command::ActivateTab { zone: one.clone() }]
        );
        // Along the row to the third place.
        let moved = act(&mut desk, &mut gestures, hold(400.0, 142.0));
        assert_eq!(
            commands(moved),
            [Command::MoveTab {
                zone: one.clone(),
                index: 2
            }]
        );
        assert_eq!(order(&desk), [two.clone(), three.clone(), one.clone()]);
        // Resting there says nothing more.
        assert_eq!(act(&mut desk, &mut gestures, hold(401.0, 142.0)), []);
        // Back to the middle.
        act(&mut desk, &mut gestures, hold(300.0, 150.0));
        assert_eq!(order(&desk), [two.clone(), one.clone(), three.clone()]);
        // Pulled down off the row: it is a zone of its own, held by its title.
        let off = act(&mut desk, &mut gestures, hold(700.0, 500.0));
        assert_eq!(
            commands(off),
            [Command::SplitTab {
                zone: one.clone(),
                x: 652,
                y: 487,
                snap: false
            }]
        );
        assert!(matches!(gestures.doing, Doing::Moving { .. }));
        assert_eq!(desk.layout().together(&two), [two.clone(), three.clone()]);
        assert_eq!(desk.layout().zones.len(), 2);
        // And goes on with the pointer as any zone that is moved.
        let on = act(&mut desk, &mut gestures, hold(800.0, 600.0));
        assert_eq!(
            commands(on),
            [Command::MoveZone {
                zone: one.clone(),
                x: 752,
                y: 587,
                snap: true
            }]
        );
        act(&mut desk, &mut gestures, release(800.0, 600.0));
        assert!(!gestures.busy());
        // Tabs that are held where they are let none be pulled off.
        desk.does(Command::Lock {
            zone: two.clone(),
            geometry: Some(true),
            members: None,
        });
        act(&mut desk, &mut gestures, press(120.0, 140.0));
        assert_eq!(
            act(&mut desk, &mut gestures, hold(700.0, 500.0)),
            [Act::Refuse(two)]
        );
    }

    #[test]
    fn items_are_chosen_one_several_and_in_a_row() {
        let (mut desk, zone, inside, loose) = desk();
        let mut gestures = Gestures::default();
        let click = |desk: &mut Desk, gestures: &mut Gestures, item: &ItemId, with: Pointer| {
            let place = centre(desk, item);
            act(
                desk,
                gestures,
                Pointer {
                    at: Some(place),
                    pressed: true,
                    down: true,
                    ..with
                },
            );
            act(
                desk,
                gestures,
                Pointer {
                    at: Some(place),
                    released: true,
                    ..with
                },
            );
        };
        let plain = Pointer::default();
        let ctrl = Pointer {
            ctrl: true,
            ..plain
        };
        let shift = Pointer {
            shift: true,
            ..plain
        };
        click(&mut desk, &mut gestures, &inside[0], plain);
        assert_eq!(desk.layout().selected(), [inside[0].clone()]);
        // The keyboard is on it, in its zone.
        assert!(
            desk.layout().item(&inside[0]).unwrap().focused
                && desk.layout().zone(&zone).unwrap().focused
        );
        click(&mut desk, &mut gestures, &inside[2], ctrl);
        assert_eq!(
            desk.layout().selected(),
            [inside[0].clone(), inside[2].clone()]
        );
        click(&mut desk, &mut gestures, &inside[0], ctrl);
        assert_eq!(desk.layout().selected(), [inside[2].clone()]);
        // Shift: from the one last pressed (the first) to this one.
        click(&mut desk, &mut gestures, &inside[2], shift);
        assert_eq!(desk.layout().selected(), inside);
        // A plain click on one of them leaves it alone, when the button comes up.
        let place = centre(&desk, &inside[1]);
        act(
            &mut desk,
            &mut gestures,
            Pointer {
                at: Some(place),
                pressed: true,
                down: true,
                ..plain
            },
        );
        assert_eq!(
            desk.layout().selected().len(),
            3,
            "still all, in case they are dragged"
        );
        act(
            &mut desk,
            &mut gestures,
            Pointer {
                at: Some(place),
                released: true,
                ..plain
            },
        );
        assert_eq!(desk.layout().selected(), [inside[1].clone()]);
        // A loose item: chosen, and no zone has the keyboard.
        click(&mut desk, &mut gestures, &loose[0], plain);
        assert_eq!(desk.layout().selected(), [loose[0].clone()]);
        assert!(desk.layout().zones.iter().all(|zone| !zone.focused));
        click(&mut desk, &mut gestures, &inside[1], ctrl);
        assert_eq!(desk.layout().selected().len(), 2);
        // A press on the bare desktop lets go of all; with Ctrl it keeps them.
        act(
            &mut desk,
            &mut gestures,
            Pointer {
                ctrl: true,
                ..press(900.0, 900.0)
            },
        );
        assert_eq!(desk.layout().selected().len(), 2);
        act(&mut desk, &mut gestures, release(900.0, 900.0));
        act(&mut desk, &mut gestures, press(900.0, 900.0));
        assert!(desk.layout().selected().is_empty());
        act(&mut desk, &mut gestures, release(900.0, 900.0));
        // A press in a zone beside its items: it has the keyboard, and
        // nothing in it is chosen.
        click(&mut desk, &mut gestures, &inside[0], plain);
        act(&mut desk, &mut gestures, press(400.0, 350.0));
        assert!(desk.layout().selected().is_empty() && desk.layout().zone(&zone).unwrap().focused);
    }

    #[test]
    fn a_rectangle_drawn_on_the_desktop_chooses_what_it_touches() {
        let (mut desk, _, inside, loose) = desk();
        let mut gestures = Gestures::default();
        act(&mut desk, &mut gestures, press(900.0, 900.0));
        act(&mut desk, &mut gestures, hold(300.0, 150.0));
        assert_eq!(
            gestures.band(),
            Some(Rect::from_two_pos(pos2(900.0, 900.0), pos2(300.0, 150.0)))
        );
        // The third file of the zone is in the rectangle's reach, the first is not.
        let chosen = desk.layout().selected();
        assert!(chosen.contains(&inside[2]) && !chosen.contains(&inside[0]));
        // A movement that touches nothing new says nothing.
        assert_eq!(act(&mut desk, &mut gestures, hold(301.0, 151.0)), []);
        act(&mut desk, &mut gestures, hold(0.0, 0.0));
        assert_eq!(desk.layout().selected().len(), inside.len() + loose.len());
        // Smaller again: what it no longer touches is let go.
        act(&mut desk, &mut gestures, hold(850.0, 850.0));
        assert!(desk.layout().selected().is_empty());
        assert_eq!(act(&mut desk, &mut gestures, release(850.0, 850.0)), []);
        assert_eq!(gestures.band(), None);
    }

    #[test]
    fn carried_items_go_where_they_are_let_go() {
        let (mut desk, zone, inside, loose) = desk();
        let other = desk.zone("Other", (700, 100, 400, 300));
        let mut gestures = Gestures::default();
        // (Each from the desk as it was at first: nothing is carried out.)
        let layout = desk.layout();
        let mut carry = |desk: &mut Desk, item: &ItemId, to: Pos2| {
            desk.does(Command::Select {
                items: vec![item.clone()],
                mode: SelectMode::Replace,
            });
            let layout = Layout { ..desk.layout() };
            let from = layout.item(item).unwrap().rect.center();
            gestures.pointer(&layout, &press(from.x, from.y));
            gestures.pointer(&layout, &hold(to.x, to.y));
            assert!(gestures.carrying());
            commands(gestures.pointer(&layout, &release(to.x, to.y)))
        };
        // Onto a cell of the other zone.
        assert_eq!(
            carry(&mut desk, &inside[0], pos2(708.0 + 85.0, 136.0 + 90.0)),
            [Command::AddItem {
                zone: other.clone(),
                item: inside[0].clone(),
                index: None,
                cell: Some(Cell { column: 1, row: 1 })
            }]
        );
        // Onto its title: into it, at no place in particular.
        assert_eq!(
            carry(&mut desk, &loose[0], pos2(800.0, 112.0)),
            [Command::AddItem {
                zone: other.clone(),
                item: loose[0].clone(),
                index: None,
                cell: None
            }]
        );
        // Onto the bare desktop: where its corner is, from the display's.
        let from = layout.item(&inside[1]).unwrap().rect.center();
        let corner = layout.item(&inside[1]).unwrap().rect.min;
        assert_eq!(
            carry(&mut desk, &inside[1], pos2(900.0, 800.0)),
            [Command::PlaceLooseItem {
                item: inside[1].clone(),
                display: DisplayId::new("fake-1"),
                x: (900.0 + corner.x - from.x) as i32,
                y: (800.0 + corner.y - from.y) as i32,
            }]
        );
        // Within its own zone, onto another cell: it is put there.
        assert_eq!(
            carry(
                &mut desk,
                &inside[2],
                pos2(108.0 + 5.0, 136.0 + 2.0 * 86.0 + 5.0)
            ),
            [Command::AddItem {
                zone: zone.clone(),
                item: inside[2].clone(),
                index: None,
                cell: Some(Cell { column: 0, row: 2 })
            }]
        );
        // Several at once keep how they lie to each other on the desktop.
        desk.does(Command::Select {
            items: loose.clone(),
            mode: SelectMode::Replace,
        });
        let layout = desk.layout();
        let from = layout.item(&loose[0]).unwrap().rect.center();
        let mut gestures = Gestures::default();
        gestures.pointer(&layout, &press(from.x, from.y));
        gestures.pointer(&layout, &hold(1300.0, 700.0));
        let placed = commands(gestures.pointer(&layout, &release(1300.0, 700.0)));
        let Command::PlaceLooseItem { y: first, .. } = placed[0] else {
            panic!()
        };
        let Command::PlaceLooseItem { y: second, .. } = placed[1] else {
            panic!()
        };
        assert_eq!(second - first, 86, "one cell below, as before");
    }

    #[test]
    fn a_zone_that_takes_nothing_gets_nothing_and_says_so() {
        let (mut desk, zone, inside, loose) = desk();
        desk.does(Command::Lock {
            zone: zone.clone(),
            geometry: None,
            members: Some(true),
        });
        let layout = desk.layout();
        let items = [(loose[0].clone(), Vec2::ZERO)];
        assert_eq!(
            dropped(&layout, &items, pos2(300.0, 300.0)),
            [Act::Refuse(zone.clone())]
        );
        assert_eq!(dropped(&layout, &items, pos2(-10.0, 5.0)), []);
        // What is in it does not go to another place in it either.
        let own = [(inside[0].clone(), Vec2::ZERO)];
        assert_eq!(
            dropped(&layout, &own, pos2(300.0, 300.0)),
            [Act::Refuse(zone)]
        );
    }

    #[test]
    fn items_dragged_off_our_surfaces_are_handed_to_the_system() {
        let (mut desk, _, inside, _) = desk();
        let mut gestures = Gestures::default();
        let from = centre(&desk, &inside[0]);
        act(&mut desk, &mut gestures, press(from.x, from.y));
        act(&mut desk, &mut gestures, hold(from.x + 40.0, from.y));
        let gone = Pointer {
            at: None,
            down: true,
            ..Pointer::default()
        };
        assert_eq!(
            gestures.pointer(&desk.layout(), &gone),
            [Act::DragOut(vec![inside[0].clone()])]
        );
        assert!(!gestures.busy());
        // The release that follows is nobody's.
        assert_eq!(
            gestures.pointer(
                &desk.layout(),
                &Pointer {
                    released: true,
                    ..Pointer::default()
                }
            ),
            []
        );
    }

    #[test]
    fn a_double_click_opens_and_the_other_button_asks_for_a_menu() {
        let (mut desk, zone, inside, _) = desk();
        let mut gestures = Gestures::default();
        let place = centre(&desk, &inside[1]);
        let twice = Pointer {
            double: true,
            ..press(place.x, place.y)
        };
        // Through the core, which says when it is to be started.
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &twice)),
            [
                Command::FocusItem {
                    item: Some(inside[1].clone())
                },
                Command::Open {
                    items: vec![inside[1].clone()],
                    with: None,
                    in_place: true
                }
            ]
        );
        // The menu of an item that is not chosen is of that item alone.
        desk.does(Command::Select {
            items: vec![inside[0].clone(), inside[2].clone()],
            mode: SelectMode::Replace,
        });
        let menu = |x: f32, y: f32| Pointer {
            menu: true,
            ..at(x, y)
        };
        assert_eq!(
            gestures.pointer(&desk.layout(), &menu(place.x, place.y)),
            [
                Act::Do(Command::FocusItem {
                    item: Some(inside[1].clone())
                }),
                Act::ItemMenu {
                    items: vec![inside[1].clone()],
                    at: place,
                    ours: false
                }
            ]
        );
        // Of one that is chosen, of all that are, in the order shown.
        desk.does(Command::Select {
            items: vec![inside[2].clone(), inside[1].clone()],
            mode: SelectMode::Replace,
        });
        assert_eq!(
            gestures.pointer(&desk.layout(), &menu(place.x, place.y)),
            [Act::ItemMenu {
                items: vec![inside[1].clone(), inside[2].clone()],
                at: place,
                ours: false
            }]
        );
        // With Shift the zones' own lines are asked for.
        assert_eq!(
            gestures.pointer(
                &desk.layout(),
                &Pointer {
                    shift: true,
                    ..menu(place.x, place.y)
                }
            ),
            [Act::ItemMenu {
                items: vec![inside[1].clone(), inside[2].clone()],
                at: place,
                ours: true
            }]
        );
        // Twice on one of them opens them all.
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &twice)),
            [Command::Open {
                items: vec![inside[1].clone(), inside[2].clone()],
                with: None,
                in_place: true
            }]
        );
        assert_eq!(
            gestures.pointer(&desk.layout(), &menu(250.0, 112.0)),
            [Act::ZoneMenu {
                zone: zone.clone(),
                at: pos2(250.0, 112.0)
            }]
        );
        assert_eq!(
            gestures.pointer(&desk.layout(), &menu(900.0, 900.0)),
            [Act::DeskMenu {
                display: DisplayId::new("fake-1"),
                at: pos2(900.0, 900.0)
            }]
        );
        // Twice on the bare desktop: everything is hidden, or shown again.
        let twice = Pointer {
            double: true,
            ..press(900.0, 900.0)
        };
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &twice)),
            [Command::QuickHide { hidden: None }]
        );
    }

    #[test]
    fn a_rolled_up_zone_that_opens_at_a_press_is_asked_to_by_a_click_on_its_title() {
        let (mut desk, zone, ..) = desk();
        desk.does(Command::RollUp {
            zone: zone.clone(),
            rolled_up: Some(true),
        });
        let mut gestures = Gestures::default();
        // Where zones open under the pointer, a click only gives the keyboard.
        gestures.pointer(&desk.layout(), &press(250.0, 112.0));
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &release(250.0, 112.0))),
            [Command::FocusZone {
                zone: Some(zone.clone())
            }]
        );
        desk.does(Command::ModifySettings {
            changes: serde_json::json!({"roll_up": {"expand_on": "click"}}),
        });
        gestures.pointer(&desk.layout(), &press(250.0, 112.0));
        let clicked = gestures.pointer(&desk.layout(), &release(250.0, 112.0));
        assert_eq!(
            commands(clicked),
            [
                Command::FocusZone {
                    zone: Some(zone.clone())
                },
                Command::Expand {
                    zone: zone.clone(),
                    expanded: None
                }
            ]
        );
        // A drag of the title is no click: nothing opens.
        gestures.pointer(&desk.layout(), &press(250.0, 112.0));
        gestures.pointer(&desk.layout(), &hold(300.0, 200.0));
        assert_eq!(gestures.pointer(&desk.layout(), &release(300.0, 200.0)), []);
    }

    #[test]
    fn the_keys_open_choose_and_step() {
        let (mut desk, zone, inside, loose) = desk();
        let mut gestures = Gestures::default();
        let key = |desk: &mut Desk, gestures: &mut Gestures, key: Key| {
            let acts = gestures.key(&desk.layout(), key);
            for act in &acts {
                if let Act::Do(command) = act {
                    desk.tries(command.clone());
                }
            }
            acts
        };
        let right = Key::Step {
            step: Step::Right,
            extend: false,
        };
        assert_eq!(key(&mut desk, &mut gestures, Key::Enter), []);
        // With the keyboard on a zone, a step goes to its first item.
        desk.does(Command::FocusZone {
            zone: Some(zone.clone()),
        });
        key(&mut desk, &mut gestures, right);
        assert_eq!(desk.layout().selected(), [inside[0].clone()]);
        assert_eq!(
            commands(key(&mut desk, &mut gestures, right)),
            [Command::MoveItemFocus {
                step: Step::Right,
                extend: false
            }]
        );
        key(
            &mut desk,
            &mut gestures,
            Key::Step {
                step: Step::Right,
                extend: true,
            },
        );
        assert_eq!(
            desk.layout().selected(),
            [inside[1].clone(), inside[2].clone()]
        );
        assert_eq!(
            commands(key(&mut desk, &mut gestures, Key::Enter)),
            [Command::OpenFocused]
        );
        key(&mut desk, &mut gestures, Key::SelectAll);
        assert_eq!(
            desk.layout().selected(),
            inside,
            "those of the zone that has the keyboard"
        );
        // The zone's own keys name the zone that has the keyboard.
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::Roll)),
            [Command::RollUp {
                zone: ZoneId::new("@focused"),
                rolled_up: None
            }]
        );
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::NextTab { backwards: true })),
            [Command::NextTab {
                zone: ZoneId::new("@focused"),
                backwards: true
            }]
        );
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::Zone(Direction::Left))),
            [Command::FocusNeighbour {
                direction: Direction::Left
            }]
        );
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::Carry(Direction::Right))),
            [Command::MoveFocusedItems {
                direction: Some(Direction::Right),
                zone: None
            }]
        );
        // Escape is the core's to make something of: first nothing is chosen...
        assert_eq!(
            commands(key(&mut desk, &mut gestures, Key::Escape)),
            [Command::Dismiss]
        );
        assert!(desk.layout().selected().is_empty());
        // ...then the keyboard is let go.
        key(&mut desk, &mut gestures, Key::Escape);
        assert!(!desk.layout().zone(&zone).unwrap().focused);
        assert_eq!(
            gestures.key(&desk.layout(), Key::Roll),
            [],
            "no zone has the keyboard"
        );
        // Among the loose items the keys are the same: down the column.
        let place = centre(&desk, &loose[0]);
        act(&mut desk, &mut gestures, press(place.x, place.y));
        act(&mut desk, &mut gestures, release(place.x, place.y));
        key(
            &mut desk,
            &mut gestures,
            Key::Step {
                step: Step::Down,
                extend: false,
            },
        );
        assert_eq!(desk.layout().selected(), [loose[1].clone()]);
        key(
            &mut desk,
            &mut gestures,
            Key::Step {
                step: Step::Up,
                extend: true,
            },
        );
        assert_eq!(desk.layout().selected(), loose);
        assert!(
            desk.layout().desks[0].focused,
            "the keyboard is among the loose items"
        );
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::Enter)),
            [Command::OpenFocused]
        );
        assert_eq!(
            gestures.key(&desk.layout(), Key::Roll),
            [],
            "the desktop is no zone to roll up"
        );
        key(&mut desk, &mut gestures, Key::SelectAll);
        assert_eq!(desk.layout().selected(), loose, "those shown with it");
        // Escape with something under way lets go of that, and of nothing else.
        act(&mut desk, &mut gestures, press(900.0, 900.0));
        act(&mut desk, &mut gestures, hold(800.0, 800.0));
        assert_eq!(gestures.key(&desk.layout(), Key::Escape), []);
        assert!(!gestures.busy());
    }

    /// Where a button of a zone's title is.
    fn button_at(desk: &Desk, zone: &ZoneId, button: Button) -> Pos2 {
        let layout = desk.layout();
        let shown = layout.zone(zone).unwrap();
        shown
            .buttons
            .iter()
            .find(|placed| placed.button == button)
            .unwrap_or_else(|| panic!("{button:?} is not there"))
            .rect
            .center()
    }

    #[test]
    fn a_button_of_a_title_does_what_the_menu_does_when_it_is_let_go_on() {
        let (mut desk, zone, ..) = desk();
        let mut gestures = Gestures::default();
        let mut click = |desk: &mut Desk, button: Button| {
            let at = button_at(desk, &zone, button);
            assert_eq!(
                act(desk, &mut gestures, press(at.x, at.y)),
                [],
                "{button:?}: nothing at the press"
            );
            act(desk, &mut gestures, release(at.x, at.y))
        };
        // Rolled up, and down again.
        assert_eq!(
            commands(click(&mut desk, Button::Roll)),
            [Command::RollUp {
                zone: zone.clone(),
                rolled_up: None
            }]
        );
        assert!(desk.layout().zone(&zone).unwrap().rolled_up);
        click(&mut desk, Button::Roll);
        assert!(!desk.layout().zone(&zone).unwrap().rolled_up);
        // The lock goes on by one each time: held where it is, then
        // nothing in or out as well, then neither.
        let locks = |desk: &Desk| desk.layout().zone(&zone).unwrap().own_lock;
        assert_eq!(locks(&desk), (false, false));
        assert_eq!(
            commands(click(&mut desk, Button::Lock)),
            [Command::Lock {
                zone: zone.clone(),
                geometry: Some(true),
                members: Some(false)
            }]
        );
        assert_eq!(locks(&desk), (true, false));
        click(&mut desk, Button::Lock);
        assert_eq!(locks(&desk), (true, true));
        click(&mut desk, Button::Lock);
        assert_eq!(locks(&desk), (false, false));
        // As a list, and as a grid again.
        click(&mut desk, Button::View);
        assert_eq!(desk.layout().zone(&zone).unwrap().view, View::List);
        click(&mut desk, Button::View);
        assert_eq!(desk.layout().zone(&zone).unwrap().view, View::Grid);
        // How it is ordered, and everything else: menus, under their buttons.
        let under = |desk: &Desk, button: Button| {
            desk.layout()
                .zone(&zone)
                .unwrap()
                .buttons
                .iter()
                .find(|placed| placed.button == button)
                .unwrap()
                .rect
                .left_bottom()
        };
        assert_eq!(
            click(&mut desk, Button::Sort),
            [Act::ViewMenu {
                zone: zone.clone(),
                at: under(&desk, Button::Sort)
            }]
        );
        assert_eq!(
            click(&mut desk, Button::More),
            [Act::ZoneMenu {
                zone: zone.clone(),
                at: under(&desk, Button::More)
            }]
        );
        // Covered: made private, which covers it; shown; covered again.
        click(&mut desk, Button::Cover);
        assert!(
            desk.layout().zone(&zone).unwrap().masked && desk.layout().zone(&zone).unwrap().private
        );
        assert_eq!(
            commands(click(&mut desk, Button::Cover)),
            [Command::Reveal {
                zone: Some(zone.clone()),
                revealed: Some(true)
            }]
        );
        assert!(!desk.layout().zone(&zone).unwrap().masked);
        click(&mut desk, Button::Cover);
        assert!(desk.layout().zone(&zone).unwrap().masked);
    }

    #[test]
    fn a_button_let_go_elsewhere_does_nothing_and_dragged_it_moves_its_zone() {
        let (mut desk, zone, ..) = desk();
        let mut gestures = Gestures::default();
        let roll = button_at(&desk, &zone, Button::Roll);
        let lock = button_at(&desk, &zone, Button::Lock);
        // Down on one, up on its neighbour (within what is no drag yet
        // would be the same button: this is farther).
        gestures.pointer(&desk.layout(), &press(roll.x, roll.y));
        assert_eq!(
            gestures.pointer(&desk.layout(), &release(lock.x, lock.y)),
            []
        );
        // A title is a title wherever it is taken: dragged by a button,
        // the zone is moved, and no button is pressed.
        act(&mut desk, &mut gestures, press(roll.x, roll.y));
        let moved = act(&mut desk, &mut gestures, hold(roll.x + 60.0, roll.y + 40.0));
        assert!(matches!(
            commands(moved).as_slice(),
            [Command::MoveZone { .. }]
        ));
        assert_eq!(
            act(
                &mut desk,
                &mut gestures,
                release(roll.x + 60.0, roll.y + 40.0)
            ),
            []
        );
        assert!(!desk.layout().zone(&zone).unwrap().rolled_up);
        // The other button on a button is the zone's menu, as anywhere on it.
        let at = button_at(&desk, &zone, Button::More);
        assert_eq!(
            gestures.pointer(
                &desk.layout(),
                &Pointer {
                    menu: true,
                    ..super::tests::at(at.x, at.y)
                }
            ),
            [Act::ZoneMenu { zone, at }]
        );
    }

    #[test]
    fn tab_goes_along_the_buttons_of_the_title_and_enter_or_space_presses_one() {
        let (mut desk, zone, ..) = desk();
        let mut gestures = Gestures::default();
        let next = Key::Title { backwards: false };
        // Without a zone that has the keyboard, there is no title to go along.
        assert_eq!(gestures.key(&desk.layout(), next), []);
        assert_eq!(gestures.title_at, None);
        desk.does(Command::FocusZone {
            zone: Some(zone.clone()),
        });
        let mut seen = Vec::new();
        for _ in 0..Button::ALL.len() {
            gestures.key(&desk.layout(), next);
            seen.push(gestures.title_at.clone().unwrap().1);
        }
        assert_eq!(seen, Button::ALL, "from the left, each once");
        // Past the last: back among the items.
        gestures.key(&desk.layout(), next);
        assert_eq!(gestures.title_at, None);
        // Backwards from the items: the last first.
        gestures.key(&desk.layout(), Key::Title { backwards: true });
        assert_eq!(gestures.title_at, Some((zone.clone(), Button::More)));
        gestures.key(&desk.layout(), Key::Title { backwards: true });
        assert_eq!(gestures.title_at, Some((zone.clone(), Button::Roll)));
        // Space and Enter press it; the keyboard stays on it.
        let rolled = gestures.key(&desk.layout(), Key::Space);
        assert_eq!(
            commands(rolled),
            [Command::RollUp {
                zone: zone.clone(),
                rolled_up: None
            }]
        );
        assert_eq!(
            commands(gestures.key(&desk.layout(), Key::Enter)),
            [Command::RollUp {
                zone: zone.clone(),
                rolled_up: None
            }],
            "not the item that would be opened"
        );
        assert_eq!(gestures.title_at, Some((zone.clone(), Button::Roll)));
        // Escape takes the keyboard off the title, and nothing else yet.
        assert_eq!(gestures.key(&desk.layout(), Key::Escape), []);
        assert_eq!(gestures.title_at, None);
        // An arrow does too, and goes among the items as always.
        gestures.key(&desk.layout(), next);
        let stepped = gestures.key(
            &desk.layout(),
            Key::Step {
                step: Step::Right,
                extend: false,
            },
        );
        assert_eq!(
            commands(stepped),
            [Command::MoveItemFocus {
                step: Step::Right,
                extend: false
            }]
        );
        assert_eq!(gestures.title_at, None);
        // Space alone presses nothing.
        assert_eq!(gestures.key(&desk.layout(), Key::Space), []);
        // The keyboard is on a button only of the zone that has it.
        gestures.key(&desk.layout(), next);
        desk.does(Command::FocusZone { zone: None });
        gestures.settle(&desk.layout());
        assert_eq!(gestures.title_at, None);
    }

    #[test]
    fn the_key_for_menus_asks_for_the_menu_of_what_has_the_keyboard() {
        let (mut desk, zone, inside, _) = desk();
        let mut gestures = Gestures::default();
        // Nothing has it: the desktop's, on its first display.
        let layout = desk.layout();
        assert_eq!(
            gestures.key(&layout, Key::Menu),
            [Act::DeskMenu {
                display: DisplayId::new("fake-1"),
                at: layout.desks[0].area.min + vec2(24.0, 24.0)
            }]
        );
        // A zone has it, and nothing is chosen: the zone's, under its title.
        desk.does(Command::FocusZone {
            zone: Some(zone.clone()),
        });
        let layout = desk.layout();
        let head = layout.zone(&zone).unwrap().head;
        assert_eq!(
            gestures.key(&layout, Key::Menu),
            [Act::ZoneMenu {
                zone: zone.clone(),
                at: head.left_bottom() + vec2(12.0, 0.0)
            }]
        );
        // On an item: of what is chosen, at the item.
        desk.does(Command::FocusItem {
            item: Some(inside[1].clone()),
        });
        let layout = desk.layout();
        let at = layout.item(&inside[1]).unwrap().rect.center();
        assert_eq!(
            gestures.key(&layout, Key::Menu),
            [Act::ItemMenu {
                items: vec![inside[1].clone()],
                at,
                ours: false
            }]
        );
        // On a button of the title: the zone's.
        gestures.key(&layout, Key::Title { backwards: false });
        assert!(matches!(
            gestures.key(&layout, Key::Menu).as_slice(),
            [Act::ZoneMenu { .. }]
        ));
    }

    #[test]
    fn the_middle_button_twice_on_the_bare_desktop_does_what_the_configuration_says() {
        use serde_json::json;
        let (mut desk, ..) = desk();
        let mut gestures = Gestures::default();
        let twice = Pointer {
            double: true,
            ..press(900.0, 900.0)
        };
        let middle = Pointer {
            middle_double: true,
            ..at(900.0, 900.0)
        };
        // As it comes: the middle button makes every icon grey or lets it
        // be. The main button hides the zones or shows them again, always.
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &twice)),
            [Command::QuickHide { hidden: None }]
        );
        assert_eq!(
            commands(gestures.pointer(&desk.layout(), &middle)),
            [Command::GreyIcons { on: None }]
        );
        for (word, does) in [
            ("peek", vec![Command::TogglePeek]),
            ("quick_hide", vec![Command::QuickHide { hidden: None }]),
            ("grey_icons", vec![Command::GreyIcons { on: None }]),
            ("none", Vec::new()),
        ] {
            desk.does(Command::ModifySettings {
                changes: json!({"gestures": {"double_middle_click": word}}),
            });
            assert_eq!(
                commands(gestures.pointer(&desk.layout(), &middle)),
                does,
                "{word}"
            );
            // Whatever the middle button is set to, the main one is as it was.
            assert_eq!(
                commands(gestures.pointer(&desk.layout(), &twice)),
                [Command::QuickHide { hidden: None }],
                "{word}"
            );
        }
    }

    #[test]
    fn the_middle_button_twice_means_something_on_the_bare_desktop_only() {
        let (mut desk, zone, inside, _) = desk();
        let mut gestures = Gestures::default();
        let middle = |x: f32, y: f32| Pointer {
            middle_double: true,
            ..at(x, y)
        };
        let item = centre(&desk, &inside[0]);
        for (what, place) in [
            ("an item", item),
            ("a zone's title", pos2(250.0, 112.0)),
            ("a zone beside its items", pos2(400.0, 350.0)),
        ] {
            assert_eq!(
                gestures.pointer(&desk.layout(), &middle(place.x, place.y)),
                [],
                "{what}"
            );
        }
        // And it takes nothing out of the hand that the main button holds.
        act(&mut desk, &mut gestures, press(250.0, 112.0));
        act(&mut desk, &mut gestures, hold(350.0, 162.0));
        let both = Pointer {
            middle_double: true,
            down: true,
            ..at(900.0, 900.0)
        };
        let acts = commands(gestures.pointer(&desk.layout(), &both));
        assert!(acts.contains(&Command::GreyIcons { on: None }));
        assert!(
            matches!(gestures.doing, Doing::Moving { .. }),
            "{:?}",
            gestures.doing
        );
        let _ = zone;
    }
}
