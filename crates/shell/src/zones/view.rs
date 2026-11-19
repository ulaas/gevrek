//! Where everything of the zones is on the desktop, as rectangles: what
//! is drawn from, and what a press is looked up in. Made anew from the
//! core's scene whenever something changed; nothing here draws or reads
//! the pointer, so all of it is tried in tests.
//!
//! Everything is in the desktop's own logical units (as the core's
//! rectangles are), not in a surface's: a zone dragged from one display
//! to the next is one movement.

use egui::{pos2, vec2, Pos2, Rect};
use gevrek_zones::config::{DeskGesture, ExpandOn, Sort, TitleBar, View, ZoneKind, ZoneLook};
use gevrek_zones::core::ZonesCore;
use gevrek_zones::item::ItemKind;
use gevrek_zones::portal::{Listing, PortalScene};
use gevrek_zones::layout::{self, Metrics};
use gevrek_zones::placement::Cell;
use gevrek_zones::snap::Edges;
use gevrek_zones::{DisplayId, ItemId, ZoneId};
use std::collections::BTreeMap;

/// How near a zone's side the pointer takes hold of it: this far inside,
/// and a little outside. Wide enough to be found without looking for it:
/// a hand on a mouse does not rest on a line three pixels thin.
pub const GRIP_INSIDE: f32 = 8.0;
pub const GRIP_OUTSIDE: f32 = 4.0;
/// How long a corner is along each of its sides.
pub const CORNER: f32 = 22.0;
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
    /// It is among the chosen ones (the core says which are).
    pub selected: bool,
    /// The keyboard is on it.
    pub focused: bool,
    /// It stays when the rest is put away with the one key.
    pub stays: bool,
    /// Its name as it is shown, and what sort of thing it is.
    pub name: String,
    pub kind: ItemKind,
}

impl ItemBox {
    fn of(placed: &gevrek_zones::layout::PlacedItem, rect: Rect) -> Self {
        Self {
            item: placed.item.clone(),
            rect,
            selected: placed.selected,
            focused: placed.focused,
            stays: placed.always_visible,
            name: placed.name.clone(),
            kind: placed.kind,
        }
    }
}

/// An item that stays where it was while its zone is put away with the
/// one key: drawn bare, as the loose items are.
#[derive(Debug, Clone, PartialEq)]
pub struct Stay {
    pub shown: ItemBox,
    pub zone: ZoneId,
    pub view: View,
    pub icon_size: u32,
}

/// One tab of zones that share a place.
#[derive(Debug, Clone, PartialEq)]
pub struct TabBox {
    pub zone: ZoneId,
    pub title: String,
    pub rect: Rect,
    pub front: bool,
}

/// Where a press in a portal's title goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nav {
    /// To the folder shown before.
    Back,
    /// To the folder above.
    Up,
    /// To a folder on the way from the portal's own to where it stands.
    To(String),
}

/// One thing in a portal's title that is pressed to go somewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct NavBox {
    pub rect: Rect,
    pub nav: Nav,
    /// A folder's name; nothing for back and up, which are drawn as figures.
    pub label: String,
    /// The folder the portal stands in: the end of the way, and no way to go.
    pub here: bool,
}

/// How wide a folder's name is given in a title, by how long it is: the
/// layout knows no fonts, and what is drawn is cut to fit this.
fn crumb_width(name: &str) -> f32 {
    (14.0 + 6.6 * name.chars().count() as f32).clamp(28.0, 150.0)
}

/// The way of a portal as boxes in its title, from `left` to `right`:
/// back and up where it can go there, then the folders from its own to
/// where it stands. Where they do not all fit, the first ones are left
/// out (the last is where it stands, and stays). Nothing for a portal
/// that stands in its own folder and came from nowhere.
fn way(portal: &PortalScene, head: Rect, left: f32, right: f32) -> Vec<NavBox> {
    if portal.crumbs.len() < 2 && !portal.can_go_back {
        return Vec::new();
    }
    let mut boxes = Vec::new();
    let mut at = left;
    let high = (head.height() - 6.0).max(8.0);
    let top = head.center().y - high / 2.0;
    for (can, nav) in [(portal.can_go_back, Nav::Back), (portal.can_go_up, Nav::Up)] {
        if can {
            boxes.push(NavBox { rect: Rect::from_min_size(pos2(at, top), vec2(20.0, high)), nav, label: String::new(), here: false });
            at += 22.0;
        }
    }
    at += 2.0;
    // As many of the last folders as fit.
    let mut shown = portal.crumbs.len();
    let mut wide = 0.0;
    for (index, crumb) in portal.crumbs.iter().enumerate().rev() {
        let needs = crumb_width(&crumb.name) + 10.0;
        if wide + needs > right - at && index + 1 < portal.crumbs.len() {
            break;
        }
        wide += needs;
        shown = index;
    }
    for (index, crumb) in portal.crumbs.iter().enumerate().skip(shown) {
        let here = index + 1 == portal.crumbs.len();
        let width = crumb_width(&crumb.name).min((right - at).max(28.0));
        boxes.push(NavBox {
            rect: Rect::from_min_size(pos2(at, top), vec2(width, high)),
            nav: Nav::To(crumb.directory.clone()),
            label: crumb.name.clone(),
            here,
        });
        at += width + 10.0;
    }
    boxes
}

/// One of the small things at the end of a zone's title that are pressed
/// to do what the zone's menu also does, without opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Cover what the zone holds from view, or show it again.
    Cover,
    /// How it is ordered: the menu for that, at the button.
    Sort,
    /// As a grid, or as a list.
    View,
    /// Held where it is; then nothing in or out as well; then neither.
    Lock,
    /// Rolled up to its title, or down again.
    Roll,
    /// The zone's menu.
    More,
}

impl Button {
    /// In the order they stand, from the left. The ones a narrow zone
    /// keeps are the last: its menu, and rolling it up.
    pub const ALL: [Button; 6] = [Button::Cover, Button::Sort, Button::View, Button::Lock, Button::Roll, Button::More];
}

/// One button of a title, where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonBox {
    pub rect: Rect,
    pub button: Button,
}

/// What pressing a button of a zone's title does now, in a few words:
/// said under the button when the pointer rests on it, and to a screen
/// reader.
pub fn button_says(zone: &ZoneBox, button: Button) -> &'static str {
    match button {
        Button::Roll if zone.rolled_up => "Roll down",
        Button::Roll => "Roll up",
        Button::Lock => match zone.own_lock {
            (false, false) => "Hold it where it is",
            (true, false) => "Held where it is. Press: nothing in or out either",
            (true, true) => "Held, and nothing in or out. Press: let go",
            (false, true) => "Nothing in or out. Press: let go",
        },
        Button::View if zone.view == View::Grid => "Show as a list",
        Button::View => "Show as a grid",
        Button::Sort => "Shown and ordered",
        Button::Cover if zone.masked => "Show what it holds",
        Button::Cover if zone.private => "Cover it again",
        Button::Cover => "Cover what it holds",
        Button::More => "Menu of the zone",
    }
}

/// How wide a button of a title is, and the room between two.
const BUTTON: f32 = 22.0;
const BUTTON_GAP: f32 = 2.0;
/// The least that is left of a title for the zone's name.
const NAME_LEAST: f32 = 76.0;

/// The buttons at the end of a title, from the right: as many as leave
/// the name its room. A title too narrow for any has none; the menu is
/// then a press of the other button away, as it always is.
fn buttons(head: Rect) -> Vec<ButtonBox> {
    let high = (head.height() - 4.0).clamp(8.0, BUTTON);
    let room = head.width() - NAME_LEAST - 8.0;
    let fit = ((room + BUTTON_GAP) / (BUTTON + BUTTON_GAP)).floor().max(0.0) as usize;
    let shown = &Button::ALL[Button::ALL.len() - fit.min(Button::ALL.len())..];
    let mut right = head.right() - 8.0;
    let mut boxes: Vec<ButtonBox> = shown
        .iter()
        .rev()
        .map(|button| {
            let rect = Rect::from_min_size(pos2(right - BUTTON, head.center().y - high / 2.0), vec2(BUTTON, high));
            right -= BUTTON + BUTTON_GAP;
            ButtonBox { rect, button: *button }
        })
        .collect();
    boxes.reverse();
    boxes
}

/// How far something that takes long has come.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// What it is called off by (`Command::CancelOperation`).
    pub request: u64,
    /// From 0 to 1; none where the backend cannot say.
    pub part: Option<f32>,
    /// What is done, in a few words: "Copying film.mkv".
    pub what: String,
}

impl Progress {
    fn of(operation: &gevrek_zones::core::OperationScene) -> Self {
        use gevrek_zones::backend::FileOperation;
        let name = |path: &str| path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        let what = match &operation.operation {
            FileOperation::Copy { from, .. } => format!("Copying {}", name(from)),
            FileOperation::Move { from, .. } => format!("Moving {}", name(from)),
            FileOperation::Trash { path } => format!("Putting {} into the bin", name(path)),
            FileOperation::Restore { path } => format!("Bringing {} back", name(path)),
            FileOperation::Rename { path, .. } => format!("Renaming {}", name(path)),
            FileOperation::NewFolder { name, .. } => format!("Making {name}"),
        };
        Self {
            request: operation.request,
            part: (operation.total > 0).then(|| (operation.done as f32 / operation.total as f32).clamp(0.0, 1.0)),
            what,
        }
    }
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
    /// Rolled up, and open for now: it lies over what is under it.
    pub expanded: bool,
    /// What opens it while it is rolled up.
    pub expand_on: ExpandOn,
    pub fills: Fills,
    /// The buttons at the end of its title, from the left. They are
    /// there whether they are drawn or not (they are drawn under the
    /// pointer, and a press is where the pointer is).
    pub buttons: Vec<ButtonBox>,
    /// By its own lock or the one for all zones.
    pub locked_place: bool,
    pub locked_items: bool,
    /// Its own lock alone (held where it is, nothing in or out): what
    /// the button of its title goes on from.
    pub own_lock: (bool, bool),
    /// What it holds is covered: there are no items, only how many.
    pub masked: bool,
    pub private: bool,
    /// Tiled windows keep off its place.
    pub reserves: bool,
    /// It has the keyboard.
    pub focused: bool,
    pub always_visible: bool,
    /// How many it holds, whatever is shown of them.
    pub item_count: usize,
    /// What is typed to show only some of its items.
    pub filter: Option<String>,
    /// For a portal: where it stands.
    pub portal: Option<PortalScene>,
    /// The way of a portal, in its title, where it has gone anywhere.
    pub nav: Vec<NavBox>,
    /// Something that takes long is done with files of it (a large
    /// copy): how far it has come, where that is known.
    pub busy: Option<Progress>,
    /// What it shows is not of the desktop (files of a folder, programs,
    /// what was used last): taken out of it, a file is brought to the
    /// desktop, not merely put elsewhere.
    pub outside: bool,
    /// How far the listing of what it shows has come, where that is not
    /// the desktop itself.
    pub listing: Option<Listing>,
    /// How much of a title bar it has. (Rolled up and shut, the title is
    /// all there is of it, and is shown whatever this says.)
    pub title_bar: TitleBar,
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
    /// What stays of the zones that are put away, each where it was.
    pub stays: Vec<Stay>,
    /// The loose items are covered; `item_count` says how many there are.
    pub masked: bool,
    pub item_count: usize,
    /// The keyboard is among the loose items of this display.
    pub focused: bool,
}

/// Everything, as it is now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// In the order they are drawn: a later one lies over an earlier.
    pub zones: Vec<ZoneBox>,
    pub desks: Vec<DeskBox>,
    /// Everything was put away with the one key.
    pub hidden: bool,
    /// The zones are looked at above the windows.
    pub peeking: bool,
    /// What takes long and is not done to a zone that is shown (a large
    /// file brought to the bare desktop): said at the desktop's foot.
    pub under_way: Vec<Progress>,
    /// Every icon is drawn grey (asked for at once for all of them; what
    /// each zone says of its own icons is kept under it).
    pub grey: bool,
    /// What pressing the bare desktop twice with the middle button does.
    pub twice_middle: DeskGesture,
}

/// What lies at a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Item { item: ItemId, zone: Option<ZoneId> },
    /// A zone's title bar.
    Head(ZoneId),
    /// A part of a portal's way in its title.
    Nav(ZoneId, Nav),
    /// A button at the end of a zone's title.
    Button(ZoneId, Button),
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
        /// A folder among the zone's items that lies under the point,
        /// where the zone shows the files of a folder: what is let go
        /// there goes into that folder.
        into: Option<ItemId>,
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
        let mut layout = Layout {
            hidden: scene.quick_hidden,
            peeking: scene.peeking,
            grey: scene.grey_icons,
            twice_middle: core.config().gestures.double_middle_click,
            ..Layout::default()
        };
        let shown_zone = |zone: &ZoneId| scene.zones.iter().any(|shown| &shown.zone == zone && shown.shown && shown.front);
        layout.under_way = scene
            .operations
            .iter()
            .filter(|operation| !operation.zones.iter().any(shown_zone))
            .map(Progress::of)
            .collect();
        // What stays of zones that are put away, by display.
        let mut stays: BTreeMap<DisplayId, Vec<Stay>> = BTreeMap::new();
        let put_away = scene.quick_hidden && !scene.peeking;
        // (Only of zones that belong to what is shown here at all: not
        // hidden on these displays, and of the workspace that is shown.)
        for zone in scene.zones.iter().filter(|zone| put_away && !zone.shown && zone.front && zone.here) {
            // (Where it would be if its zone were there: not scrolled.)
            let content = rect_of(zone.content);
            let seen = |placed: &&gevrek_zones::layout::PlacedItem| {
                placed.always_visible && !zone.rolled_up && placed.rect.y + placed.rect.height <= zone.content.height
            };
            stays.entry(zone.display.clone()).or_default().extend(zone.items.items.iter().filter(seen).map(|placed| Stay {
                shown: ItemBox::of(placed, rect_of(placed.rect).translate(content.min.to_vec2())),
                zone: zone.zone.clone(),
                view: zone.view,
                icon_size: zone.icon_size,
            }));
        }
        // Of zones that lie over each other, one that is open for now is
        // on top: it opened over what is under it.
        let floating = |zone: &&gevrek_zones::core::ZoneScene| zone.rolled_up && zone.expanded;
        let resting = scene.zones.iter().filter(|zone| !floating(zone));
        for zone in resting.chain(scene.zones.iter().filter(floating)).filter(|zone| zone.shown && zone.front) {
            let Some(definition) = core.config().zone(&zone.zone) else {
                continue;
            };
            let rect = rect_of(zone.rect);
            let tabbed = zone.tab_group.is_some();
            let head = Rect::from_min_size(
                rect.min,
                vec2(rect.width(), (metrics.title_height as f32).min(rect.height())),
            );
            // Rolled up and not open for now: the title is all there is.
            let shut = zone.rolled_up && !zone.expanded;
            let tabs = match &zone.tab_group {
                Some(group) if !shut => {
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
            let content = if shut {
                Rect::from_min_size(rect.left_bottom(), vec2(rect.width(), 0.0))
            } else {
                rect_of(layout::content_area(zone.rect, tabbed, &metrics))
            };
            let extent = if shut { 0.0 } else { zone.items.scroll_extent as f32 };
            let scroll = scrolls.get(&zone.zone).copied().unwrap_or(0.0).clamp(0.0, extent);
            let (cell_width, cell_height) =
                metrics.cell(zone.view, zone.icon_size, content.width() as i32);
            let step = vec2(
                (cell_width + metrics.spacing) as f32,
                (cell_height + if zone.view == View::Grid { metrics.spacing } else { 0 }) as f32,
            );
            let items = if shut {
                Vec::new()
            } else {
                zone.items
                    .items
                    .iter()
                    .map(|placed| {
                        ItemBox::of(placed, rect_of(placed.rect).translate(content.min.to_vec2() - vec2(0.0, scroll)))
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
                expanded: zone.rolled_up && zone.expanded,
                expand_on: zone.expand_on,
                fills: match definition.kind {
                    ZoneKind::Manual => Fills::ByHand,
                    ZoneKind::Portal { .. } => Fills::Folder,
                    ZoneKind::Smart { .. } => Fills::Query,
                },
                buttons: buttons(head),
                locked_place: zone.lock.geometry,
                locked_items: zone.lock.members,
                own_lock: (definition.lock.geometry, definition.lock.members),
                masked: zone.masked,
                private: zone.private,
                reserves: zone.reserves_space,
                focused: zone.focused,
                always_visible: zone.always_visible,
                item_count: zone.item_count,
                filter: zone.filter.clone().filter(|typed| !typed.is_empty()),
                portal: zone.portal.clone(),
                // (The end of the title is kept for what the zone says of
                // itself, and for its buttons.)
                nav: zone
                    .portal
                    .as_ref()
                    .map(|portal| {
                        let kept = buttons(head).first().map_or(96.0, |first| (rect.right() - first.rect.left() + 8.0).max(96.0));
                        way(portal, head, rect.left() + 22.0, rect.right() - kept)
                    })
                    .unwrap_or_default(),
                busy: scene.operations.iter().find(|operation| operation.zones.contains(&zone.zone)).map(Progress::of),
                outside: matches!(definition.kind, ZoneKind::Portal { .. })
                    || (matches!(definition.kind, ZoneKind::Smart { .. })
                        && definition.source != gevrek_zones::config::SmartSource::Desktop),
                listing: zone.portal.as_ref().map(|portal| portal.listing.clone()).or_else(|| zone.source.clone()),
                title_bar: zone.look.title_bar.unwrap_or_default(),
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

        for display in core.displays() {
            let loose = scene.loose.iter().find(|loose| loose.display == display.id);
            let area = loose.map_or(rect_of(display.rect), |loose| rect_of(loose.area));
            layout.desks.push(DeskBox {
                display: display.id.clone(),
                rect: rect_of(display.rect),
                area,
                items: loose
                    .filter(|_| core.enabled())
                    .map(|loose| {
                        loose
                            .items
                            .iter()
                            // (Put away, what stays of the loose ones is
                            // there still; and all of them where only the
                            // zones are put away.)
                            .filter(|placed| !scene.loose_hidden || placed.always_visible)
                            .map(|placed| ItemBox::of(placed, rect_of(placed.rect).translate(area.min.to_vec2())))
                            .collect()
                    })
                    .unwrap_or_default(),
                icon_size: core.config().loose_icon_size,
                stays: stays.remove(&display.id).unwrap_or_default(),
                masked: loose.is_some_and(|loose| loose.masked),
                item_count: loose.map_or(0, |loose| loose.item_count),
                focused: loose.is_some_and(|loose| loose.focused),
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
                if let Some(pressed) = zone.buttons.iter().find(|shown| shown.rect.contains(at)) {
                    return Hit::Button(zone.zone.clone(), pressed.button);
                }
                // (Where it stands is no way to go: that part is its title.)
                return match zone.nav.iter().find(|part| !part.here && part.rect.contains(at)) {
                    Some(part) => Hit::Nav(zone.zone.clone(), part.nav.clone()),
                    None => Hit::Head(zone.zone.clone()),
                };
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
            if let Some(stay) = desk.stays.iter().find(|stay| stay.shown.rect.contains(at)) {
                return Hit::Item { item: stay.shown.item.clone(), zone: Some(stay.zone.clone()) };
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
                into: zone
                    .items
                    .iter()
                    .find(|item| within && zone.fills == Fills::Folder && item.kind == ItemKind::Folder && item.rect.contains(at))
                    .map(|item| item.item.clone()),
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
            .chain(self.desks.iter().flat_map(|desk| desk.stays.iter().map(|stay| &stay.shown)))
    }

    /// The chosen items, in the order they are shown.
    pub fn selected(&self) -> Vec<ItemId> {
        self.every_item().filter(|shown| shown.selected).map(|shown| shown.item.clone()).collect()
    }

    /// The zone an item is shown in; none for a loose one.
    pub fn zone_of(&self, item: &ItemId) -> Option<&ZoneBox> {
        self.zones.iter().find(|zone| zone.items.iter().any(|shown| &shown.item == item))
    }

    /// The zone drawn at a zone's place: itself, or the tab in front of
    /// the tabs it is one of.
    pub fn shown_for(&self, zone: &ZoneId) -> Option<&ZoneBox> {
        self.zones.iter().find(|shown| &shown.zone == zone || shown.tabs.iter().any(|tab| &tab.zone == zone))
    }

    /// The zones that share a zone's place, itself among them.
    pub fn together(&self, zone: &ZoneId) -> Vec<ZoneId> {
        self.shown_for(zone)
            .map(|shown| shown.tabs.iter().map(|tab| tab.zone.clone()).collect::<Vec<_>>())
            .filter(|tabs| !tabs.is_empty())
            .unwrap_or_else(|| vec![zone.clone()])
    }

    /// The zone whose title bar or tabs lie at a point, leaving out some
    /// (the one that is dragged lies under the pointer itself): where a
    /// zone that is let go becomes a tab. A zone held where it is takes
    /// none.
    pub fn tab_target(&self, at: Pos2, but: &[ZoneId]) -> Option<&ZoneBox> {
        self.zones
            .iter()
            .rev()
            .filter(|zone| !but.contains(&zone.zone))
            .find(|zone| zone.head.contains(at) || zone.tabs.iter().any(|tab| tab.rect.contains(at)))
            .filter(|zone| !zone.locked_place)
    }

    /// Which place among a zone's tabs a point is at, counted from 0.
    pub fn tab_index(&self, zone: &ZoneId, at: Pos2) -> Option<usize> {
        let shown = self.shown_for(zone)?;
        let first = shown.tabs.first()?;
        let along = ((at.x - first.rect.left()) / first.rect.width().max(1.0)).floor();
        Some((along.max(0.0) as usize).min(shown.tabs.len() - 1))
    }
}

/// The pointer's shape for the sides of a zone that would be moved: the
/// arrows along the way they go.
pub fn pointer_for_edges(edges: Edges) -> egui::CursorIcon {
    use egui::CursorIcon;
    let across = edges.left || edges.right;
    let down = edges.top || edges.bottom;
    match (across, down) {
        (true, false) => CursorIcon::ResizeHorizontal,
        (false, true) => CursorIcon::ResizeVertical,
        // The two corners that lie on the line from top left to bottom right.
        (true, true) if edges.left == edges.top => CursorIcon::ResizeNwSe,
        (true, true) => CursorIcon::ResizeNeSw,
        (false, false) => CursorIcon::Default,
    }
}

impl Layout {
    /// The pointer's shape over what lies at a point: it says what a
    /// press there would take hold of, before anything is pressed. A side
    /// or corner of a zone that can be sized has the arrows of its way; a
    /// zone held where it is has none (taken by its rim it says that it
    /// is held, and the pointer does not promise otherwise); what is
    /// pressed like a button has the hand.
    pub fn pointer_at(&self, at: Pos2) -> egui::CursorIcon {
        match self.hit(at) {
            Hit::Edge(zone, edges) if self.shown_for(&zone).is_some_and(|shown| !shown.locked_place) => pointer_for_edges(edges),
            Hit::Button(..) | Hit::Nav(..) => egui::CursorIcon::PointingHand,
            _ => egui::CursorIcon::Default,
        }
    }
}

/// The sides of a zone a point takes hold of, if it is on its rim. One
/// that is rolled up is only made wider or narrower. (A zone that is held
/// where it is has its rim too: taken by it, it says that it is held.)
fn grip(zone: &ZoneBox, at: Pos2) -> Option<Edges> {
    if !zone.rect.expand(GRIP_OUTSIDE).contains(at) || zone.rect.shrink(GRIP_INSIDE).contains(at) {
        return None;
    }
    let rect = zone.rect;
    let mut left = at.x <= rect.left() + GRIP_INSIDE;
    let mut right = at.x >= rect.right() - GRIP_INSIDE;
    let mut top = at.y <= rect.top() + GRIP_INSIDE;
    let mut bottom = at.y >= rect.bottom() - GRIP_INSIDE;
    if zone.rolled_up && !zone.expanded {
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

/// The crossings of the grid a zone's corners rest on (`grid` units
/// apart, counted from the free room's corner): shown while it is moved
/// or sized, so that the grid is seen where it holds. None without a
/// grid.
pub fn grid_marks(rect: Rect, area: Rect, grid: f32) -> Vec<Pos2> {
    if grid < 2.0 {
        return Vec::new();
    }
    let on = |value: f32, from: f32| {
        let steps = (value - from) / grid;
        (steps - steps.round()).abs() * grid < 0.5
    };
    let mut marks = Vec::new();
    for x in [rect.left(), rect.right()] {
        for y in [rect.top(), rect.bottom()] {
            if on(x, area.left()) && on(y, area.top()) {
                marks.push(pos2(x, y));
            }
        }
    }
    marks
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

        /// Carry out what a gesture asked for, as the zones do: a
        /// refusal is the core's answer, not a fault of the test.
        pub fn tries(&mut self, command: Command) {
            self.host.command(&mut self.core, &mut self.backend, command, NOW);
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
        // A zone that is held where it is has its rim all the same (taken
        // by it, it says that it is held), and is known to be held.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(true), members: None });
        assert!(matches!(desk.layout().hit(pos2(499.0, 250.0)), Hit::Edge(..)));
        assert!(desk.layout().zone(&zone).unwrap().locked_place);
        // Rolled up, there is the title and nothing under it.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(false), members: None });
        desk.does(Command::RollUp { zone: zone.clone(), rolled_up: Some(true) });
        let layout = desk.layout();
        assert_eq!(layout.hit(pos2(250.0, 112.0)), Hit::Head(zone.clone()));
        assert_eq!(layout.hit(pos2(250.0, 250.0)), Hit::Desk(display.clone()));
        assert!(layout.zone(&zone).unwrap().items.is_empty());
        // Open for now, all of it is there again, and it is rolled up still.
        desk.does(Command::Expand { zone: zone.clone(), expanded: Some(true) });
        let layout = desk.layout();
        let open = layout.zone(&zone).unwrap();
        assert!(open.rolled_up && open.expanded && open.items.len() == 1);
        assert_eq!(layout.hit(first.center()), Hit::Item { item: inside.clone(), zone: Some(zone.clone()) });
        assert_eq!(layout.hit(pos2(498.0, 398.0)), Hit::Edge(zone.clone(), Edges::BOTTOM_RIGHT));
        desk.does(Command::Expand { zone: zone.clone(), expanded: Some(false) });
        assert_eq!(desk.layout().hit(pos2(250.0, 250.0)), Hit::Desk(display));
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
                index: None,
                into: None,
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
    fn the_way_of_a_portal_is_laid_out_in_its_title_and_the_first_folders_give_way() {
        use gevrek_zones::portal::Crumb;
        let head = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 28.0));
        let crumb = |name: &str, directory: &str| Crumb { name: name.to_string(), directory: directory.to_string() };
        let mut portal = PortalScene {
            root: "/d".to_string(),
            directory: "/d".to_string(),
            crumbs: vec![crumb("Downloads", "/d")],
            can_go_up: false,
            can_go_back: false,
            can_go_forward: false,
            listing: Listing::Ready,
        };
        // In its own folder, having gone nowhere: its title is its name.
        assert_eq!(way(&portal, head, 122.0, 404.0), []);
        // One folder in: back, up, and the two folders; the last is where it stands.
        portal.directory = "/d/2026".to_string();
        portal.crumbs.push(crumb("2026", "/d/2026"));
        (portal.can_go_up, portal.can_go_back) = (true, true);
        let parts = way(&portal, head, 122.0, 404.0);
        let navs: Vec<&Nav> = parts.iter().map(|part| &part.nav).collect();
        assert_eq!(navs, [&Nav::Back, &Nav::Up, &Nav::To("/d".to_string()), &Nav::To("/d/2026".to_string())]);
        assert_eq!(parts.iter().map(|part| part.here).collect::<Vec<_>>(), [false, false, false, true]);
        // One after the other, none over another, all within the title.
        for pair in parts.windows(2) {
            assert!(pair[0].rect.right() <= pair[1].rect.left(), "{pair:?}");
        }
        assert!(parts.iter().all(|part| head.contains_rect(part.rect) && part.rect.right() <= 404.0));
        // Deep in: the folders nearest to where it stands are kept.
        for name in ["Invoices", "Paid in the first quarter", "From the council"] {
            let directory = format!("{}/{name}", portal.directory);
            portal.crumbs.push(crumb(name, &directory));
            portal.directory = directory;
        }
        let parts = way(&portal, head, 122.0, 404.0);
        let shown: Vec<&str> = parts.iter().filter(|part| matches!(part.nav, Nav::To(_))).map(|part| part.label.as_str()).collect();
        assert_eq!(shown.last(), Some(&"From the council"));
        assert!(shown.len() < 5 && !shown.contains(&"Downloads"), "{shown:?}");
        assert!(parts.iter().all(|part| part.rect.right() <= 404.5), "{parts:?}");
        // Back again at its own folder, with a way back: only that.
        portal.crumbs.truncate(1);
        portal.directory = "/d".to_string();
        portal.can_go_up = false;
        let parts = way(&portal, head, 122.0, 404.0);
        assert_eq!(parts.iter().map(|part| &part.nav).collect::<Vec<_>>(), [&Nav::Back, &Nav::To("/d".to_string())]);
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

    #[test]
    fn a_title_has_its_buttons_at_its_end_and_a_narrow_one_keeps_the_last() {
        let mut desk = Desk::new();
        let wide = desk.zone("Papers", (100, 100, 400, 300));
        let narrow = desk.zone("N", (600, 100, 150, 200));
        let tiny = desk.zone("T", (800, 100, 100, 200));
        let layout = desk.layout();
        let buttons = |zone: &ZoneId| -> Vec<Button> { layout.zone(zone).unwrap().buttons.iter().map(|placed| placed.button).collect() };
        // All of them, from the left, the last at the title's end.
        assert_eq!(buttons(&wide), Button::ALL);
        let shown = layout.zone(&wide).unwrap();
        let last = shown.buttons.last().unwrap();
        assert_eq!(last.rect.right(), shown.head.right() - 8.0);
        assert!(shown.buttons.windows(2).all(|pair| pair[0].rect.right() < pair[1].rect.left()), "none over another");
        assert!(shown.buttons.iter().all(|placed| shown.head.contains_rect(placed.rect)));
        // The name keeps its room: what does not fit is left out from the
        // left, so that the menu and rolling up are the last to go.
        assert_eq!(buttons(&narrow), [Button::Roll, Button::More]);
        assert!(layout.zone(&narrow).unwrap().buttons[0].rect.left() - layout.zone(&narrow).unwrap().head.left() >= NAME_LEAST);
        // (As small as a zone can be made: its menu at most.)
        assert!(matches!(buttons(&tiny).as_slice(), [] | [Button::More]), "{:?}", buttons(&tiny));
        // A press on one is that button; beside them, the title.
        let roll = shown.buttons.iter().find(|placed| placed.button == Button::Roll).unwrap().rect.center();
        assert_eq!(layout.hit(roll), Hit::Button(wide.clone(), Button::Roll));
        assert_eq!(layout.hit(pos2(250.0, 114.0)), Hit::Head(wide.clone()));
        // The rim is still the rim where a button reaches it.
        assert!(matches!(layout.hit(pos2(last.rect.center().x, 101.0)), Hit::Edge(..)));
        // What each says it does goes by how the zone stands.
        assert_eq!(button_says(shown, Button::Roll), "Roll up");
        assert_eq!(button_says(shown, Button::View), "Show as a list");
        assert_eq!(button_says(shown, Button::Lock), "Hold it where it is");
        assert_eq!(button_says(shown, Button::Cover), "Cover what it holds");
        desk.does(Command::RollUp { zone: wide.clone(), rolled_up: Some(true) });
        desk.does(Command::Lock { zone: wide.clone(), geometry: Some(true), members: None });
        let layout = desk.layout();
        let shown = layout.zone(&wide).unwrap();
        assert_eq!(button_says(shown, Button::Roll), "Roll down");
        assert!(button_says(shown, Button::Lock).starts_with("Held where it is"));
        // Rolled up, the title is all there is, and its buttons are there.
        assert_eq!(shown.buttons.len(), Button::ALL.len());
    }

    #[test]
    fn a_portals_way_leaves_the_buttons_their_room() {
        let mut desk = Desk::new();
        desk.backend.add_folder("/work");
        desk.backend.add_folder("/work/a-folder-with-a-long-name");
        desk.backend.add_folder("/work/a-folder-with-a-long-name/and-another-long-one");
        desk.does(Command::CreateZone {
            title: "Work".to_string(),
            kind: ZoneKind::Portal { directory: "/work".to_string() },
            rect: Some(gevrek_zones::Rect::new(100, 100, 420, 300)),
        });
        let zone = desk.core.config().zones.last().unwrap().id.clone();
        desk.host.pump(&mut desk.core, &mut desk.backend, NOW);
        desk.does(Command::NavigateTo { zone: zone.clone(), directory: "/work/a-folder-with-a-long-name/and-another-long-one".to_string() });
        desk.host.pump(&mut desk.core, &mut desk.backend, NOW);
        let layout = desk.layout();
        let shown = layout.zone(&zone).unwrap();
        assert!(!shown.nav.is_empty(), "it shows its way");
        let first = shown.buttons.first().unwrap().rect.left();
        // (The way's last part may be cut short when it is drawn; where
        // it begins is what must be clear of the buttons.)
        assert!(shown.nav.iter().all(|part| part.rect.left() < first), "{:?} / {first}", shown.nav);
        assert!(shown.nav.iter().filter(|part| !part.here).all(|part| part.rect.right() <= first));
    }

    #[test]
    fn the_rim_is_wide_enough_to_be_found_and_the_pointer_says_what_it_would_take() {
        use egui::CursorIcon;
        let mut desk = Desk::new();
        let zone = desk.zone("Papers", (100, 100, 400, 300));
        let layout = desk.layout();
        // Eight inside and four outside, all around.
        for (what, at, is_rim) in [
            ("four outside the right side", pos2(503.5, 250.0), true),
            ("eight inside it", pos2(492.5, 250.0), true),
            ("nine inside it", pos2(490.5, 250.0), false),
            ("five outside it", pos2(505.0, 250.0), false),
            ("eight inside the bottom", pos2(300.0, 392.5), true),
            ("four above the top", pos2(300.0, 96.5), true),
        ] {
            assert_eq!(matches!(layout.hit(at), Hit::Edge(..)), is_rim, "{what}");
        }
        // (where, the pointer there)
        for (what, at, shape) in [
            ("the right side", pos2(498.0, 250.0), CursorIcon::ResizeHorizontal),
            ("the left side", pos2(101.0, 250.0), CursorIcon::ResizeHorizontal),
            ("the bottom", pos2(300.0, 398.0), CursorIcon::ResizeVertical),
            ("the top", pos2(300.0, 101.0), CursorIcon::ResizeVertical),
            ("the lower right corner", pos2(498.0, 398.0), CursorIcon::ResizeNwSe),
            ("the upper left corner", pos2(101.0, 101.0), CursorIcon::ResizeNwSe),
            ("the upper right corner", pos2(498.0, 101.0), CursorIcon::ResizeNeSw),
            ("the lower left corner", pos2(101.0, 398.0), CursorIcon::ResizeNeSw),
            ("along the bottom, within the corner's reach", pos2(480.0, 398.0), CursorIcon::ResizeNwSe),
            ("the title", pos2(250.0, 114.0), CursorIcon::Default),
            ("the zone", pos2(300.0, 250.0), CursorIcon::Default),
            ("the bare desktop", pos2(900.0, 900.0), CursorIcon::Default),
        ] {
            assert_eq!(layout.pointer_at(at), shape, "{what}");
        }
        let more = layout.zone(&zone).unwrap().buttons.last().unwrap().rect.center();
        assert_eq!(layout.pointer_at(more), CursorIcon::PointingHand);
        // A zone held where it is promises nothing: taken by its rim it
        // says that it is held.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(true), members: None });
        let layout = desk.layout();
        assert!(matches!(layout.hit(pos2(498.0, 250.0)), Hit::Edge(..)));
        assert_eq!(layout.pointer_at(pos2(498.0, 250.0)), CursorIcon::Default);
        // Rolled up, only its sides: wider or narrower.
        desk.does(Command::Lock { zone: zone.clone(), geometry: Some(false), members: None });
        desk.does(Command::RollUp { zone: zone.clone(), rolled_up: Some(true) });
        let layout = desk.layout();
        let head = layout.zone(&zone).unwrap().head;
        assert_eq!(layout.pointer_at(pos2(head.right() - 1.0, head.center().y)), CursorIcon::ResizeHorizontal);
        assert_eq!(pointer_for_edges(Edges::default()), CursorIcon::Default);
    }

    #[test]
    fn what_the_middle_button_does_twice_and_whether_icons_are_grey_is_in_the_layout() {
        use gevrek_zones::config::DeskGesture;
        use serde_json::json;
        let mut desk = Desk::new();
        desk.file("a.txt", None);
        let layout = desk.layout();
        assert_eq!((layout.twice_middle, layout.grey), (DeskGesture::GreyIcons, false));
        desk.does(Command::GreyIcons { on: None });
        assert!(desk.layout().grey);
        desk.does(Command::ModifySettings { changes: json!({"gestures": {"double_middle_click": "peek"}}) });
        assert_eq!(desk.layout().twice_middle, DeskGesture::Peek);
        // Put away, the loose items go with the zones.
        desk.does(Command::QuickHide { hidden: Some(true) });
        assert!(desk.layout().desks[0].items.is_empty() && desk.layout().hidden);
    }
}
