//! Where everything of the zones is on the desktop, as rectangles: what
//! is drawn from, and what a press is looked up in. Made anew from the
//! core's scene whenever something changed; nothing here draws or reads
//! the pointer, so all of it is tried in tests.
//!
//! Everything is in the desktop's own logical units (as the core's
//! rectangles are), not in a surface's: a zone dragged from one display
//! to the next is one movement.

use egui::{pos2, vec2, Pos2, Rect};
use gevrek_zones::config::{Sort, View, ZoneKind, ZoneLook};
use gevrek_zones::core::ZonesCore;
use gevrek_zones::layout::{self, Metrics};
use gevrek_zones::placement::Cell;
use gevrek_zones::snap::Edges;
use gevrek_zones::{DisplayId, ItemId, ZoneId};
use std::collections::BTreeMap;

/// How near a zone's side the pointer takes hold of it: this far inside,
/// and a little outside.
const GRIP_INSIDE: f32 = 6.0;
const GRIP_OUTSIDE: f32 = 3.0;
/// How long a corner is along each of its sides.
const CORNER: f32 = 16.0;
/// The widest a tab is.
const TAB_WIDTH: f32 = 150.0;

fn rect_of(rect: gevrek_zones::Rect) -> Rect {
    Rect::from_min_size(
        pos2(rect.x as f32, rect.y as f32),
        vec2(rect.width as f32, rect.height as f32),
    )
}

/// One item, where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemBox {
    pub item: ItemId,
    /// Its whole cell.
    pub rect: Rect,
}

/// One tab of zones that share a place.
#[derive(Debug, Clone, PartialEq)]
pub struct TabBox {
    pub zone: ZoneId,
    pub title: String,
    pub rect: Rect,
    pub front: bool,
}

/// Where a zone's items come from, as far as dropping on it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fills {
    ByHand,
    /// A portal: what is dropped is moved into its folder.
    Folder,
    /// A smart zone: nothing can be put into it.
    Query,
}

/// One zone that is drawn (of tabs, the one in front).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneBox {
    pub zone: ZoneId,
    pub title: String,
    pub display: DisplayId,
    pub rect: Rect,
    /// The bar with the title.
    pub head: Rect,
    /// The tabs, when the zone shares its place; empty otherwise.
    pub tabs: Vec<TabBox>,
    /// What the items are seen through. Nothing of it when rolled up.
    pub content: Rect,
    /// The items, already moved by how far the zone is scrolled. Those
    /// outside `content` are not seen.
    pub items: Vec<ItemBox>,
    pub rolled_up: bool,
    pub fills: Fills,
    pub locked_place: bool,
    pub locked_items: bool,
    pub view: View,
    /// The order is the person's own, so a drop says where in it.
    pub by_hand: bool,
    pub icon_size: u32,
    pub look: ZoneLook,
    /// How far it is scrolled, and how far it can be.
    pub scroll: f32,
    pub scroll_extent: f32,
    /// From one cell to the next.
    pub step: egui::Vec2,
    pub columns: u32,
}

/// The bare desktop of one display.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskBox {
    pub display: DisplayId,
    /// The whole display.
    pub rect: Rect,
    /// The room the loose items are laid out in.
    pub area: Rect,
    pub items: Vec<ItemBox>,
    pub icon_size: u32,
}

/// Everything, as it is now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// In the order they are drawn: a later one lies over an earlier.
    pub zones: Vec<ZoneBox>,
    pub desks: Vec<DeskBox>,
}

/// What lies at a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Item { item: ItemId, zone: Option<ZoneId> },
    /// A zone's title bar.
    Head(ZoneId),
    Tab(ZoneId),
    /// A side or corner of a zone, to size it by.
    Edge(ZoneId, Edges),
    /// A zone, where no item is.
    Body(ZoneId),
    /// The bare desktop of a display.
    Desk(DisplayId),
    /// No display of ours.
    Nothing,
}

/// Where something that is dropped would go.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Zone {
        zone: ZoneId,
        fills: Fills,
        locked: bool,
        /// The cell under the point, where the zone's order is by hand
        /// and it shows a grid.
        cell: Option<Cell>,
        /// The line under the point, where it is a list in the person's
        /// own order.
        index: Option<usize>,
    },
    Desk { display: DisplayId, origin: Pos2 },
    Nowhere,
}

impl Layout {
    /// Everything the core shows at `now`. `scrolls` is how far each zone
    /// was scrolled; it is brought within what the zone has.
    pub fn of(core: &ZonesCore, scrolls: &BTreeMap<ZoneId, f32>, now: u64) -> Self {
        let scene = core.scene(now);
        let metrics = *core.metrics();
        let mut layout = Layout::default();
        for zone in scene.zones.iter().filter(|zone| zone.shown && zone.front) {
            let Some(definition) = core.config().zone(&zone.zone) else {
                continue;
            };
            let rect = rect_of(zone.rect);
            let tabbed = zone.tab_group.is_some();
            let head = Rect::from_min_size(
                rect.min,
                vec2(rect.width(), (metrics.title_height as f32).min(rect.height())),
            );
            let tabs = match &zone.tab_group {
                Some(group) if !zone.rolled_up => {
                    let members: Vec<_> = scene
                        .zones
                        .iter()
                        .filter(|other| other.tab_group.as_ref() == Some(group))
                        .collect();
                    let width = (rect.width() / members.len().max(1) as f32).min(TAB_WIDTH);
                    members
                        .iter()
                        .enumerate()
                        .map(|(index, member)| TabBox {
                            zone: member.zone.clone(),
                            title: member.title.clone(),
                            rect: Rect::from_min_size(
                                pos2(rect.left() + index as f32 * width, head.bottom()),
                                vec2(width, metrics.tab_height as f32),
                            ),
                            front: member.front,
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            // The whole rectangle (not the rolled-up one) is not known
            // here; rolled up, there is no content at all.
            let content = if zone.rolled_up {
                Rect::from_min_size(rect.left_bottom(), vec2(rect.width(), 0.0))
            } else {
                rect_of(layout::content_area(zone.rect, tabbed, &metrics))
            };
            let extent = if zone.rolled_up { 0.0 } else { zone.items.scroll_extent as f32 };
            let scroll = scrolls.get(&zone.zone).copied().unwrap_or(0.0).clamp(0.0, extent);
            let (cell_width, cell_height) =
                metrics.cell(zone.view, zone.icon_size, content.width() as i32);
            let step = vec2(
                (cell_width + metrics.spacing) as f32,
                (cell_height + if zone.view == View::Grid { metrics.spacing } else { 0 }) as f32,
            );
            let items = if zone.rolled_up {
                Vec::new()
            } else {
                zone.items
                    .items
                    .iter()
                    .map(|placed| ItemBox {
                        item: placed.item.clone(),
                        rect: rect_of(placed.rect)
                            .translate(content.min.to_vec2() - vec2(0.0, scroll)),
                    })
                    .collect()
            };
            layout.zones.push(ZoneBox {
                zone: zone.zone.clone(),
                title: zone.title.clone(),
                display: zone.display.clone(),
                rect,
                head,
                tabs,
                content,
                items,
                rolled_up: zone.rolled_up,
                fills: match definition.kind {
                    ZoneKind::Manual => Fills::ByHand,
                    ZoneKind::Portal { .. } => Fills::Folder,
                    ZoneKind::Smart { .. } => Fills::Query,
                },
                locked_place: definition.lock.geometry,
                locked_items: definition.lock.members,
                view: zone.view,
                by_hand: definition.kind == ZoneKind::Manual && definition.sort == Sort::Manual,
                icon_size: zone.icon_size,
                look: zone.look.clone(),
                scroll,
                scroll_extent: extent,
                step,
                columns: zone.items.columns.max(1),
            });
        }
        // Hidden with one key, the loose items are put away with the zones.
        let loose_shown = !scene.quick_hidden || scene.peeking;
        for display in core.displays() {
            let loose = scene.loose.iter().find(|loose| loose.display == display.id);
            let area = loose.map_or(rect_of(display.rect), |loose| rect_of(loose.area));
            layout.desks.push(DeskBox {
                display: display.id.clone(),
                rect: rect_of(display.rect),
                area,
                items: loose
                    .filter(|_| loose_shown && core.enabled())
                    .map(|loose| {
                        loose
                            .items
                            .iter()
                            .map(|placed| ItemBox {
                                item: placed.item.clone(),
                                rect: rect_of(placed.rect).translate(area.min.to_vec2()),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                icon_size: core.config().loose_icon_size,
            });
        }
        layout
    }

    pub fn zone(&self, zone: &ZoneId) -> Option<&ZoneBox> {
        self.zones.iter().find(|shown| &shown.zone == zone)
    }

    /// What lies at a point: of zones that overlap, the one drawn last.
    pub fn hit(&self, at: Pos2) -> Hit {
        for zone in self.zones.iter().rev() {
            if let Some(edges) = grip(zone, at) {
                return Hit::Edge(zone.zone.clone(), edges);
            }
            if !zone.rect.contains(at) {
                continue;
            }
            if zone.head.contains(at) {
                return Hit::Head(zone.zone.clone());
            }
            if let Some(tab) = zone.tabs.iter().find(|tab| tab.rect.contains(at)) {
                return Hit::Tab(tab.zone.clone());
            }
            if zone.content.contains(at) {
                if let Some(item) = zone.items.iter().find(|item| item.rect.contains(at)) {
                    return Hit::Item { item: item.item.clone(), zone: Some(zone.zone.clone()) };
                }
            }
            return Hit::Body(zone.zone.clone());
        }
        for desk in &self.desks {
            if !desk.rect.contains(at) {
                continue;
            }
            if let Some(item) = desk.items.iter().find(|item| item.rect.contains(at)) {
                return Hit::Item { item: item.item.clone(), zone: None };
            }
            return Hit::Desk(desk.display.clone());
        }
        Hit::Nothing
    }

    /// Where something dropped at a point would go.
    pub fn target(&self, at: Pos2) -> Target {
        // (Of zones that overlap, the one drawn last.)
        let over = self.zones.iter().rev().find(|zone| zone.rect.expand(GRIP_OUTSIDE).contains(at));
        if let Some(zone) = over {
            let within = zone.content.contains(at);
            let column = ((at.x - zone.content.left()) / zone.step.x).floor().max(0.0) as u32;
            let row = ((at.y - zone.content.top() + zone.scroll) / zone.step.y).floor().max(0.0) as u32;
            let by_hand = zone.by_hand && within;
            return Target::Zone {
                zone: zone.zone.clone(),
                fills: zone.fills,
                locked: zone.locked_items,
                cell: (by_hand && zone.view == View::Grid && column < zone.columns)
                    .then_some(Cell { column, row }),
                index: (by_hand && zone.view == View::List).then_some(row as usize),
            };
        }
        self.desks
            .iter()
            .find(|desk| desk.rect.contains(at))
            .map_or(Target::Nowhere, |desk| Target::Desk {
                display: desk.display.clone(),
                origin: desk.rect.min,
            })
    }

    /// The items a rectangle drawn with the pointer touches: those that
    /// are seen, wherever they are.
    pub fn items_in(&self, band: Rect) -> Vec<ItemId> {
        let seen_in_zones = self.zones.iter().flat_map(|zone| {
            zone.items
                .iter()
                .filter(|item| zone.content.intersects(item.rect))
                .filter(|item| !self.covered(item.rect.center(), Some(&zone.zone)))
        });
        let loose = self
            .desks
            .iter()
            .flat_map(|desk| desk.items.iter())
            .filter(|item| !self.covered(item.rect.center(), None));
        seen_in_zones
            .chain(loose)
            .filter(|item| band.intersects(item.rect))
            .map(|item| item.item.clone())
            .collect()
    }

    /// Whether a point of `under` (a zone, or the bare desktop) has
    /// another zone over it.
    fn covered(&self, at: Pos2, under: Option<&ZoneId>) -> bool {
        let above = match under {
            Some(zone) => self.zones.iter().position(|shown| &shown.zone == zone).map_or(0, |at| at + 1),
            None => 0,
        };
        self.zones[above..].iter().any(|zone| zone.rect.contains(at))
    }

    /// The items that are shown together with an item, in their order:
    /// those of its zone, or the loose ones of its display.
    pub fn beside(&self, item: &ItemId) -> &[ItemBox] {
        let holds = |items: &[ItemBox]| items.iter().any(|shown| &shown.item == item);
        self.zones
            .iter()
            .map(|zone| zone.items.as_slice())
            .chain(self.desks.iter().map(|desk| desk.items.as_slice()))
            .find(|items| holds(items))
            .unwrap_or_default()
    }

    /// Where an item is drawn.
    pub fn item(&self, item: &ItemId) -> Option<&ItemBox> {
        self.beside(item).iter().find(|shown| &shown.item == item)
    }

    /// Every item that is shown, in the order of the zones and then the
    /// displays.
    pub fn every_item(&self) -> impl Iterator<Item = &ItemBox> {
        self.zones
            .iter()
            .flat_map(|zone| zone.items.iter())
            .chain(self.desks.iter().flat_map(|desk| desk.items.iter()))
    }
}

/// The sides of a zone a point takes hold of, if it is on its rim. A
/// zone that is locked where it is has no rim to take; one that is rolled
/// up is only made wider or narrower.
fn grip(zone: &ZoneBox, at: Pos2) -> Option<Edges> {
    if zone.locked_place
        || !zone.rect.expand(GRIP_OUTSIDE).contains(at)
        || zone.rect.shrink(GRIP_INSIDE).contains(at)
    {
        return None;
    }
    let rect = zone.rect;
    let mut left = at.x <= rect.left() + GRIP_INSIDE;
    let mut right = at.x >= rect.right() - GRIP_INSIDE;
    let mut top = at.y <= rect.top() + GRIP_INSIDE;
    let mut bottom = at.y >= rect.bottom() - GRIP_INSIDE;
    if zone.rolled_up {
        // Only wider or narrower: its height is its title's.
        (top, bottom) = (false, false);
    } else {
        // A corner reaches a little along both of its sides.
        if top || bottom {
            left |= at.x < rect.left() + CORNER;
            right |= at.x > rect.right() - CORNER;
        }
        if left || right {
            top |= at.y < rect.top() + CORNER;
            bottom |= at.y > rect.bottom() - CORNER;
        }
    }
    // (Too small to tell its sides apart: the far ones.)
    let edges = Edges { left: left && !right, top: top && !bottom, right, bottom };
    (edges != Edges::default()).then_some(edges)
}

/// A name broken into lines no wider than `width`, `most` of them at
/// most. Lines break between words; a word that is too long alone breaks
/// where it must. What does not fit ends in an ellipsis. `measure` says
/// how wide a letter is.
pub fn label_lines(text: &str, width: f32, most: usize, measure: &mut dyn FnMut(char) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_width = 0.0;
    let mut rest = text.trim().chars().peekable();
    let mut word = String::new();
    let mut cut = false;
    let width_of = |text: &str, measure: &mut dyn FnMut(char) -> f32| text.chars().map(&mut *measure).sum::<f32>();
    'words: loop {
        // The next word, with the room before it.
        word.clear();
        while let Some(letter) = rest.peek().copied() {
            if letter.is_whitespace() {
                if !word.trim().is_empty() {
                    break;
                }
                // (However much room there is between two words, it is one.)
                if word.is_empty() {
                    word.push(' ');
                }
            } else {
                word.push(letter);
            }
            rest.next();
        }
        if word.trim().is_empty() {
            break;
        }
        // A word that does not fit goes to the next line; on the last
        // line there is none, and as much of it is shown as fits.
        let fits = line_width + width_of(&word, measure) <= width;
        if !fits && !line.is_empty() && lines.len() + 1 < most {
            lines.push(std::mem::take(&mut line));
            line_width = 0.0;
        }
        // (A new line does not begin with the room between words.)
        let word_now = if line.is_empty() { word.trim_start() } else { word.as_str() };
        for letter in word_now.chars() {
            let wide = measure(letter);
            if line_width + wide > width && !line.is_empty() {
                if lines.len() + 1 == most {
                    cut = true;
                    break 'words;
                }
                lines.push(std::mem::take(&mut line));
                line_width = 0.0;
            }
            line.push(letter);
            line_width += wide;
        }
    }
    if cut {
        let dots = measure('…');
        while !line.is_empty() && width_of(&line, measure) + dots > width {
            line.pop();
        }
        line = format!("{}…", line.trim_end());
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A line shown while a zone is moved or sized, where one of its sides
/// meets something.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub from: Pos2,
    pub to: Pos2,
}

/// The lines a zone at `rect` lies on: the sides of the free room and of
/// the display, the sides of other zones, and the gap beside them. Each
/// reaches over the zone and what it meets.
pub fn guides(rect: Rect, area: Rect, display: Rect, others: &[Rect], gap: f32) -> Vec<Guide> {
    let on = |one: f32, other: f32| (one - other).abs() < 0.5;
    let mut found: Vec<Guide> = Vec::new();
    // Two that lie on one line are one, as long as both together.
    let mut add = |guide: Guide| {
        let upright = guide.from.x == guide.to.x;
        let same_line = |other: &&mut Guide| {
            (other.from.x == other.to.x) == upright
                && if upright { on(other.from.x, guide.from.x) } else { on(other.from.y, guide.from.y) }
        };
        match found.iter_mut().find(same_line) {
            Some(other) => {
                other.from = other.from.min(guide.from);
                other.to = other.to.max(guide.to);
            }
            None => found.push(guide),
        }
    };
    // (the thing met, how far its lines lie outside it)
    let met = [(area, 0.0), (display, 0.0)]
        .into_iter()
        .chain(others.iter().flat_map(|other| [(*other, 0.0), (*other, gap)]));
    for (other, out) in met {
        let top = rect.top().min(other.top());
        let bottom = rect.bottom().max(other.bottom());
        for x in [other.left() - out, other.right() + out] {
            // Beside another zone only the facing sides meet across the gap.
            let across = out > 0.0;
            let meets = if across {
                (on(rect.right(), x) && x < other.left()) || (on(rect.left(), x) && x > other.right())
            } else {
                on(rect.left(), x) || on(rect.right(), x)
            };
            if meets {
                add(Guide { from: pos2(x, top), to: pos2(x, bottom) });
            }
        }
        let left = rect.left().min(other.left());
        let right = rect.right().max(other.right());
        for y in [other.top() - out, other.bottom() + out] {
            let across = out > 0.0;
            let meets = if across {
                (on(rect.bottom(), y) && y < other.top()) || (on(rect.top(), y) && y > other.bottom())
            } else {
                on(rect.top(), y) || on(rect.bottom(), y)
            };
            if meets {
                add(Guide { from: pos2(left, y), to: pos2(right, y) });
            }
        }
    }
    found
}

/// The default measures with those of the look: denser where the look is.
pub fn metrics_for(density: gevrek_ui::Density) -> Metrics {
    let usual = Metrics::default();
    match density {
        gevrek_ui::Density::Compact => Metrics { padding: 6, spacing: 2, title_height: 26, ..usual },
        _ => usual,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gevrek_zones::config::ZonesConfig;
    use gevrek_zones::core::Command;
    use gevrek_zones::host::Host;
    use gevrek_zones::testing::FakeBackend;
    use gevrek_zones::ZonesState;

    pub const NOW: u64 = 1_800_000_000;

    /// A desk of 1920 by 1080 with zones on it, for looking things up on.
    pub struct Desk {
        pub core: ZonesCore,
        pub backend: FakeBackend,
        pub host: Host,
    }

    impl Desk {
        pub fn new() -> Self {
            let config = ZonesConfig { enabled: true, ..ZonesConfig::default() };
            let mut desk = Self {
                core: ZonesCore::new(config, ZonesState::default()),
                backend: FakeBackend::new(),
                host: Host::new(),
            };
            desk.host.start(&mut desk.core, &mut desk.backend, NOW);
            desk
        }

        pub fn does(&mut self, command: Command) {
            let effects = self.host.command(&mut self.core, &mut self.backend, command.clone(), NOW);
            let refused = effects.iter().any(|effect| {
                matches!(effect, gevrek_zones::Effect::Notify(gevrek_zones::ZonesEvent::Refused { .. }))
            });
            assert!(!refused, "{command:?}: {effects:?}");
        }

        pub fn zone(&mut self, title: &str, rect: (i32, i32, i32, i32)) -> ZoneId {
            self.does(Command::CreateZone {
                title: title.to_string(),
                kind: ZoneKind::Manual,
                rect: Some(gevrek_zones::Rect::new(rect.0, rect.1, rect.2, rect.3)),
            });
            self.core.config().zones.last().unwrap().id.clone()
        }

        pub fn file(&mut self, name: &str, zone: Option<&ZoneId>) -> ItemId {
            let item = self.backend.add_file(name).id;
            self.host.pump(&mut self.core, &mut self.backend, NOW);
            if let Some(zone) = zone {
                self.does(Command::AddItem { zone: zone.clone(), item: item.clone(), index: None, cell: None });
            }
            item
        }

        pub fn layout(&self) -> Layout {
            Layout::of(&self.core, &BTreeMap::new(), NOW)
        }
    }

    #[test]
    fn a_point_is_looked_up_from_the_top_down() {
        let mut desk = Desk::new();
        let zone = desk.zone("Papers", (100, 100, 400, 300));
        let inside = desk.file("a.txt", Some(&zone));
        let loose = desk.file("b.txt", None);
        let layout = desk.layout();
        let display = DisplayId::new("fake-1");
        // The first cell of the zone starts below its title, inside the padding.
        let first = layout.zone(&zone).unwrap().items[0].rect;
        assert_eq!(first.min, pos2(108.0, 136.0));
        for (what, at, expected) in [
            ("the title", pos2(250.0, 112.0), Hit::Head(zone.clone())),
            ("an item in the zone", first.center(), Hit::Item { item: inside.clone(), zone: Some(zone.clone()) }),
            ("the zone beside its items", pos2(400.0, 300.0), Hit::Body(zone.clone())),
            (
                "the right side",
                pos2(499.0, 250.0),
                Hit::Edge(zone.clone(), Edges { right: true, ..Edges::default() }),
            ),
            (
                "just outside the left side",
                pos2(98.0, 250.0),
                Hit::Edge(zone.clone(), Edges { left: true, ..Edges::default() }),
            ),
            (
                "the bottom right corner",
                pos2(498.0, 398.0),
                Hit::Edge(zone.clone(), Edges::BOTTOM_RIGHT),
            ),
            (
                "the corner, a little along the bottom",
                pos2(488.0, 398.0),
                Hit::Edge(zone.clone(), Edges::BOTTOM_RIGHT),
            ),
            (
                "the top left corner",
                pos2(101.0, 101.0),
                Hit::Edge(zone.clone(), Edges { left: true, top: true, ..Edges::default() }),
            ),
            (
                "the top side, which is not the title",
                pos2(300.0, 102.0),
                Hit::Edge(zone.clone(), Edges { top: true, ..Edges::default() }),
            ),
            ("a loose item", pos2(20.0, 20.0), Hit::Item { item: loose.clone(), zone: None }),
            ("the bare desktop", pos2(900.0, 900.0), Hit::Desk(display.clone())),
            ("beyond every display", pos2(5000.0, 10.0), Hit::Nothing),
        ] {
            assert_eq!(layout.hit(at), expected, "{what}");
        }
        // A zone that is locked where it is has no rim: its side is its body.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(true), members: None });
        assert_eq!(desk.layout().hit(pos2(499.0, 250.0)), Hit::Body(zone.clone()));
        // Rolled up, there is the title and nothing under it.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(false), members: None });
        desk.does(Command::RollUp { zone: zone.clone(), rolled_up: Some(true) });
        let layout = desk.layout();
        assert_eq!(layout.hit(pos2(250.0, 112.0)), Hit::Head(zone.clone()));
        assert_eq!(layout.hit(pos2(250.0, 250.0)), Hit::Desk(display));
        assert!(layout.zone(&zone).unwrap().items.is_empty());
    }

    #[test]
    fn of_two_zones_that_overlap_the_later_one_is_hit() {
        let mut desk = Desk::new();
        let under = desk.zone("Under", (100, 100, 400, 300));
        let over = desk.zone("Over", (300, 200, 400, 300));
        let hidden = desk.file("hidden.txt", Some(&under));
        let layout = desk.layout();
        assert_eq!(layout.hit(pos2(400.0, 300.0)), Hit::Body(over.clone()));
        assert_eq!(layout.hit(pos2(200.0, 300.0)), Hit::Body(under.clone()));
        assert!(matches!(layout.target(pos2(400.0, 300.0)), Target::Zone { zone, .. } if zone == over));
        // An item of the lower zone under the upper one is not taken by a band over it.
        desk.does(Command::MoveZone { zone: over.clone(), x: 90, y: 120, snap: false });
        let layout = desk.layout();
        assert!(layout.items_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(1900.0, 1000.0))).is_empty());
        desk.does(Command::MoveZone { zone: over, x: 900, y: 600, snap: false });
        assert_eq!(desk.layout().items_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(1900.0, 1000.0))), [hidden]);
    }

    #[test]
    fn tabs_are_side_by_side_under_the_title() {
        let mut desk = Desk::new();
        let one = desk.zone("One", (100, 100, 400, 300));
        let two = desk.zone("Two", (600, 100, 300, 200));
        desk.does(Command::Tab { zone: two.clone(), with: one.clone() });
        let layout = desk.layout();
        assert_eq!(layout.zones.len(), 1, "only the tab in front is drawn");
        let shown = &layout.zones[0];
        assert_eq!(shown.zone, two);
        assert_eq!(shown.tabs.len(), 2);
        assert_eq!(shown.tabs[0].rect, Rect::from_min_size(pos2(100.0, 128.0), vec2(150.0, 26.0)));
        assert!(shown.tabs[1].front && !shown.tabs[0].front);
        assert_eq!(layout.hit(pos2(120.0, 140.0)), Hit::Tab(one));
        // The items begin below the tabs.
        assert_eq!(shown.content.top(), 100.0 + 28.0 + 26.0 + 8.0);
    }

    #[test]
    fn a_drop_knows_the_cell_or_the_line_it_is_on() {
        let mut desk = Desk::new();
        let zone = desk.zone("Z", (100, 100, 400, 300));
        let layout = desk.layout();
        // Cells are 76 by 82 with 4 between, from (108, 136).
        let target = layout.target(pos2(108.0 + 80.0 * 2.0 + 5.0, 136.0 + 86.0 + 5.0));
        assert_eq!(
            target,
            Target::Zone {
                zone: zone.clone(),
                fills: Fills::ByHand,
                locked: false,
                cell: Some(Cell { column: 2, row: 1 }),
                index: None
            }
        );
        // On the title: into the zone, at no place in particular.
        assert!(matches!(layout.target(pos2(200.0, 110.0)), Target::Zone { cell: None, index: None, .. }));
        // A list in the person's order knows the line.
        desk.does(Command::SetView { zone: zone.clone(), view: Some(View::List), sort: None, descending: None, icon_size: None });
        assert!(matches!(
            desk.layout().target(pos2(200.0, 136.0 + 24.0 * 3.0 + 2.0)),
            Target::Zone { index: Some(3), cell: None, .. }
        ));
        // Sorted by name, a drop has no place of its own.
        desk.does(Command::SetView { zone, view: None, sort: Some(Sort::Name), descending: None, icon_size: None });
        assert!(matches!(desk.layout().target(pos2(200.0, 200.0)), Target::Zone { index: None, cell: None, .. }));
        assert_eq!(
            desk.layout().target(pos2(900.0, 900.0)),
            Target::Desk { display: DisplayId::new("fake-1"), origin: pos2(0.0, 0.0) }
        );
        assert_eq!(desk.layout().target(pos2(-50.0, 10.0)), Target::Nowhere);
    }

    #[test]
    fn a_scrolled_zone_shows_its_later_items_and_no_further_than_it_has() {
        let mut desk = Desk::new();
        let zone = desk.zone("Z", (100, 100, 200, 160));
        for number in 0..9 {
            desk.file(&format!("file-{number}.txt"), Some(&zone));
        }
        let unscrolled = desk.layout();
        let shown = unscrolled.zone(&zone).unwrap();
        assert!(shown.scroll_extent > 0.0);
        let far = BTreeMap::from([(zone.clone(), 100_000.0)]);
        let scrolled = Layout::of(&desk.core, &far, NOW);
        let last = scrolled.zone(&zone).unwrap();
        assert_eq!(last.scroll, shown.scroll_extent);
        assert_eq!(last.items[0].rect.top(), shown.items[0].rect.top() - shown.scroll_extent);
        // What is scrolled out of sight is not hit.
        assert_eq!(scrolled.hit(shown.items[0].rect.center()), scrolled.hit(shown.items[0].rect.center()));
        let first_seen = scrolled.hit(pos2(120.0, 150.0));
        assert!(matches!(first_seen, Hit::Item { ref item, .. } if item != &shown.items[0].item), "{first_seen:?}");
    }

    #[test]
    fn a_name_is_broken_into_two_lines_at_most() {
        // Every letter 7 wide.
        let mut seven = |_: char| 7.0;
        for (name, width, expected) in [
            ("notes.txt", 76.0, vec!["notes.txt"]),
            ("Annual report 2026.pdf", 76.0, vec!["Annual", "report 20…"]),
            ("Annual report", 76.0, vec!["Annual", "report"]),
            ("Screenshot_2026-11-19_at_10.30.png", 70.0, vec!["Screenshot", "_2026-11-…"]),
            ("a b", 76.0, vec!["a b"]),
            ("", 76.0, vec![]),
            ("   spaced   out   ", 76.0, vec!["spaced out"]),
            ("ab", 5.0, vec!["a", "b"]),
            ("abc", 5.0, vec!["a", "…"]),
        ] {
            assert_eq!(label_lines(name, width, 2, &mut seven), expected, "{name:?} in {width}");
        }
        assert_eq!(label_lines("one two three four", 40.0, 1, &mut seven), ["one…"]);
        // No line is wider than it may be.
        for width in [20.0, 33.0, 50.0, 76.0, 120.0] {
            for line in label_lines("The quick brown fox jumps over the lazy dog.txt", width, 2, &mut seven) {
                assert!(line.chars().count() as f32 * 7.0 <= width.max(7.0), "{line:?} in {width}");
            }
        }
    }

    #[test]
    fn guides_are_where_a_zone_meets_something() {
        let display = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let area = Rect::from_min_size(pos2(0.0, 40.0), vec2(1920.0, 1040.0));
        let other = Rect::from_min_size(pos2(600.0, 300.0), vec2(300.0, 200.0));
        let lines = |rect: Rect| guides(rect, area, display, &[other], 8.0);
        let at = |x: f32, y: f32| Rect::from_min_size(pos2(x, y), vec2(200.0, 100.0));
        // In the middle of nowhere: none.
        assert_eq!(lines(at(1200.0, 700.0)), []);
        // At the free room's top: one line along it.
        assert_eq!(lines(at(1200.0, 40.0)), [Guide { from: pos2(0.0, 40.0), to: pos2(1920.0, 40.0) }]);
        // Right of the other zone with the gap, in a line with its top:
        // the gap's line, and the line along both tops.
        let both = lines(at(908.0, 300.0));
        assert_eq!(both.len(), 2, "{both:?}");
        assert!(both.contains(&Guide { from: pos2(908.0, 300.0), to: pos2(908.0, 500.0) }));
        assert!(both.contains(&Guide { from: pos2(600.0, 300.0), to: pos2(1108.0, 300.0) }));
        // The gap's line counts only between the two: a zone whose left
        // side happens to be at the other's left minus the gap meets nothing.
        assert_eq!(lines(at(592.0, 700.0)), []);
        // In the display's corner: two lines, each said once though the
        // free room and the display share the left side.
        assert_eq!(lines(at(0.0, 980.0)).len(), 2);
    }
}
