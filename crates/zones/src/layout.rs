//! Where things are inside a zone and on the bare desktop: the order of
//! the items, the grid or list they are laid out in, how far the zone
//! scrolls, what is left of a zone that is rolled up, and which zones
//! share a place as tabs. Numbers only; nothing is drawn here.

use crate::config::{Sort, View, ZoneDefinition};
use crate::ids::{ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::placement::Cell;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The measures of a zone that are not the person's to set one by one.
/// The renderer may give its own (a denser theme); the defaults are what
/// the tests use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    /// The bar with the title, which is what is left of a rolled-up zone.
    pub title_height: i32,
    /// The strip of tabs of zones that share a place.
    pub tab_height: i32,
    /// Between the zone's side and its items.
    pub padding: i32,
    /// Between two items.
    pub spacing: i32,
    /// The room for an item's name below its icon.
    pub label_height: i32,
    /// How much wider than its icon a cell is at least, for the name.
    pub label_margin: i32,
    /// The icon of a line in a list.
    pub list_icon: i32,
    /// No zone is narrower or lower than this.
    pub least_width: i32,
    pub least_height: i32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            title_height: 28,
            tab_height: 26,
            padding: 8,
            spacing: 4,
            label_height: 34,
            label_margin: 28,
            list_icon: 20,
            least_width: 120,
            least_height: 28,
        }
    }
}

impl Metrics {
    /// The smallest a zone may be made.
    pub fn least(&self) -> (i32, i32) {
        (self.least_width, self.least_height.max(self.title_height))
    }

    /// The size of one cell: of a grid, or a line of a list `width` wide.
    pub fn cell(&self, view: View, icon_size: u32, width: i32) -> (i32, i32) {
        let icon = i32::try_from(icon_size).unwrap_or(i32::MAX / 4).clamp(8, 1024);
        match view {
            View::Grid => (icon + self.label_margin, icon + self.label_height),
            View::List => (width.max(1), self.list_icon.max(1) + self.spacing),
        }
    }
}

/// What is left of a zone that is rolled up: its title, where it was.
pub fn rolled_up(rect: Rect, metrics: &Metrics) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, metrics.title_height.min(rect.height))
}

/// The part of a zone its items are laid out in: below the title (and the
/// tabs, when it shares its place), inside the padding.
pub fn content_area(rect: Rect, tabbed: bool, metrics: &Metrics) -> Rect {
    let head = metrics.title_height + if tabbed { metrics.tab_height } else { 0 };
    Rect::new(
        rect.x + metrics.padding,
        rect.y + head + metrics.padding,
        (rect.width - 2 * metrics.padding).max(0),
        (rect.height - head - 2 * metrics.padding).max(0),
    )
}

/// Names compared as a person reads them: without regard to case, and a
/// run of digits as the number it is (`file2` before `file10`).
pub fn compare_names(one: &str, other: &str) -> Ordering {
    let one: Vec<char> = one.to_lowercase().chars().collect();
    let other: Vec<char> = other.to_lowercase().chars().collect();
    let (mut at_one, mut at_other) = (0, 0);
    while at_one < one.len() && at_other < other.len() {
        if one[at_one].is_ascii_digit() && other[at_other].is_ascii_digit() {
            let digits = |text: &[char], from: usize| {
                let end = (from..text.len()).find(|at| !text[*at].is_ascii_digit()).unwrap_or(text.len());
                let run: String = text[from..end].iter().collect();
                (run.trim_start_matches('0').to_string(), end)
            };
            let (number_one, end_one) = digits(&one, at_one);
            let (number_other, end_other) = digits(&other, at_other);
            // Without leading zeros the longer number is the larger.
            let order = number_one
                .len()
                .cmp(&number_other.len())
                .then_with(|| number_one.cmp(&number_other));
            if order != Ordering::Equal {
                return order;
            }
            at_one = end_one;
            at_other = end_other;
        } else {
            let order = one[at_one].cmp(&other[at_other]);
            if order != Ordering::Equal {
                return order;
            }
            at_one += 1;
            at_other += 1;
        }
    }
    (one.len() - at_one).cmp(&(other.len() - at_other))
}

fn kind_rank(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Virtual => 0,
        ItemKind::Folder => 1,
        ItemKind::Launcher => 2,
        ItemKind::Shortcut => 3,
        ItemKind::File => 4,
    }
}

/// Items in the order a zone shows them. `Sort::Manual` leaves them as
/// they are given (the order the person made). Items that are equal by
/// what is sorted on are in the order of their names, then their ids, so
/// the order is the same every time.
pub fn sorted(mut items: Vec<&Item>, sort: Sort, descending: bool) -> Vec<&Item> {
    if sort == Sort::Manual {
        return items;
    }
    let by_name = |a: &Item, b: &Item| compare_names(&a.name, &b.name).then_with(|| a.id.cmp(&b.id));
    let key = |a: &Item, b: &Item| match sort {
        Sort::Manual | Sort::Name => Ordering::Equal,
        Sort::Type => kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.facts.extension.cmp(&b.facts.extension)),
        Sort::Date => a.facts.modified.cmp(&b.facts.modified),
        Sort::Size => a.facts.size.cmp(&b.facts.size),
    };
    items.sort_by(|a, b| {
        let order = key(a, b);
        let order = if descending { order.reverse() } else { order };
        order.then_with(|| {
            // By name alone the whole order turns; as a tie-break names
            // always run forward.
            let names = by_name(a, b);
            if descending && sort == Sort::Name {
                names.reverse()
            } else {
                names
            }
        })
    });
    items
}

/// One item, where it is drawn. The rectangle is the item's whole cell,
/// from the top left corner of what it is laid out in.
///
/// What follows the cell is what a screen reader says of the item, and
/// what a renderer needs to know of it without asking again: laying out
/// leaves it empty, and the core fills it in for the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedItem {
    pub item: ItemId,
    pub rect: Rect,
    pub cell: Cell,
    /// The name as it is shown.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: ItemKind,
    /// Which one it is among those shown with it, counted from 1.
    #[serde(default)]
    pub position: u32,
    /// How many are shown with it, itself among them.
    #[serde(default)]
    pub of: u32,
    /// It is among the chosen ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    /// The keyboard is on it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    /// It stays when the rest is put away with the one key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub always_visible: bool,
}

impl PlacedItem {
    /// An item at a place, with nothing said of it yet.
    pub fn at(item: ItemId, rect: Rect, cell: Cell) -> Self {
        Self {
            item,
            rect,
            cell,
            name: String::new(),
            kind: ItemKind::File,
            position: 0,
            of: 0,
            selected: false,
            focused: false,
            always_visible: false,
        }
    }
}

/// The items of a zone, laid out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemLayout {
    pub items: Vec<PlacedItem>,
    pub columns: u32,
    pub rows: u32,
    /// How high everything is together.
    pub content_height: i32,
    /// How far the zone can be scrolled: what does not fit.
    pub scroll_extent: i32,
}

/// Lay items out in a zone whose content area is `width` by `height`.
/// A grid fills rows from the left; a list has one item a line. `cells`
/// are the places items were put on by hand: such an item is in its cell
/// (while the zone is wide enough for it), the others fill what is free in
/// their order.
pub fn lay_out(
    view: View,
    icon_size: u32,
    metrics: &Metrics,
    (width, height): (i32, i32),
    items: &[ItemId],
    cells: &BTreeMap<ItemId, Cell>,
) -> ItemLayout {
    let (cell_width, cell_height) = metrics.cell(view, icon_size, width);
    let step_x = cell_width + metrics.spacing;
    let step_y = cell_height + if view == View::Grid { metrics.spacing } else { 0 };
    let columns = match view {
        View::Grid => (((width + metrics.spacing) / step_x).max(1)) as u32,
        View::List => 1,
    };
    let mut taken: BTreeSet<Cell> = BTreeSet::new();
    let mut at: Vec<Option<Cell>> = vec![None; items.len()];
    if view == View::Grid {
        for (index, item) in items.iter().enumerate() {
            if let Some(cell) = cells.get(item).filter(|cell| cell.column < columns) {
                if taken.insert(*cell) {
                    at[index] = Some(*cell);
                }
            }
        }
    }
    let mut next = 0u32;
    let mut placed = Vec::with_capacity(items.len());
    let mut rows = 0u32;
    for (index, item) in items.iter().enumerate() {
        let cell = at[index].unwrap_or_else(|| loop {
            let cell = Cell { column: next % columns, row: next / columns };
            next += 1;
            if taken.insert(cell) {
                break cell;
            }
        });
        rows = rows.max(cell.row.saturating_add(1));
        placed.push(PlacedItem::at(
            item.clone(),
            Rect::new(
                cell.column as i32 * step_x,
                (cell.row as i32).saturating_mul(step_y),
                cell_width,
                cell_height,
            ),
            cell,
        ));
    }
    let content_height = if rows == 0 {
        0
    } else {
        (rows as i32).saturating_mul(step_y) - (step_y - cell_height)
    };
    ItemLayout {
        items: placed,
        columns,
        rows,
        content_height,
        scroll_extent: (content_height - height).max(0),
    }
}

/// Lay out the items that are in no zone, on a display's free room
/// (`width` by `height`), as the desktop always did: down the first
/// column, then the next. An item with a place of its own (`own`, from
/// the room's top left corner) is there, brought back into the room if it
/// lies outside; the others take the cells that are free. Positions are
/// from the room's top left corner.
pub fn lay_out_loose(
    icon_size: u32,
    metrics: &Metrics,
    (width, height): (i32, i32),
    items: &[ItemId],
    own: &BTreeMap<ItemId, (i32, i32)>,
) -> Vec<PlacedItem> {
    let (cell_width, cell_height) = metrics.cell(View::Grid, icon_size, width);
    let step_x = cell_width + metrics.spacing;
    let step_y = cell_height + metrics.spacing;
    let rows = (((height + metrics.spacing) / step_y).max(1)) as u32;
    let cell_at = |x: i32, y: i32| Cell {
        column: ((x + cell_width / 2).max(0) / step_x) as u32,
        row: ((y + cell_height / 2).max(0) / step_y) as u32,
    };
    let mut taken: BTreeSet<Cell> = BTreeSet::new();
    let mut placed: Vec<Option<PlacedItem>> = vec![None; items.len()];
    for (index, item) in items.iter().enumerate() {
        if let Some((x, y)) = own.get(item) {
            let x = (*x).min(width - cell_width).max(0);
            let y = (*y).min(height - cell_height).max(0);
            let cell = cell_at(x, y);
            taken.insert(cell);
            placed[index] = Some(PlacedItem::at(item.clone(), Rect::new(x, y, cell_width, cell_height), cell));
        }
    }
    let mut next = 0u32;
    items
        .iter()
        .zip(placed)
        .map(|(item, placed)| {
            placed.unwrap_or_else(|| {
                let cell = loop {
                    let cell = Cell { column: next / rows, row: next % rows };
                    next += 1;
                    if taken.insert(cell) {
                        break cell;
                    }
                };
                PlacedItem::at(
                    item.clone(),
                    Rect::new(
                        (cell.column as i32).saturating_mul(step_x),
                        cell.row as i32 * step_y,
                        cell_width,
                        cell_height,
                    ),
                    cell,
                )
            })
        })
        .collect()
}

/// Zones that share one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    pub name: String,
    /// In the order of the configuration. The first one's place is the
    /// group's.
    pub members: Vec<ZoneId>,
}

/// The groups of tabs among the zones. A group of one is no group.
pub fn tab_groups(zones: &[ZoneDefinition]) -> Vec<TabGroup> {
    let mut groups: Vec<TabGroup> = Vec::new();
    for zone in zones {
        let Some(name) = &zone.tab_group else {
            continue;
        };
        match groups.iter_mut().find(|group| &group.name == name) {
            Some(group) => group.members.push(zone.id.clone()),
            None => groups.push(TabGroup { name: name.clone(), members: vec![zone.id.clone()] }),
        }
    }
    groups.retain(|group| group.members.len() > 1);
    groups
}

/// The group a zone is a tab of.
pub fn group_of<'a>(groups: &'a [TabGroup], zone: &ZoneId) -> Option<&'a TabGroup> {
    groups.iter().find(|group| group.members.contains(zone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemFacts;

    fn item(name: &str, kind: ItemKind, size: u64, modified: u64) -> Item {
        let mut item = Item::file(&format!("/desk/{name}"));
        item.kind = kind;
        item.facts = ItemFacts {
            size: Some(size),
            modified: Some(modified),
            ..item.facts
        };
        item
    }

    fn ids(names: &[&str]) -> Vec<ItemId> {
        names.iter().map(|name| ItemId::new(*name)).collect()
    }

    #[test]
    fn names_are_compared_as_a_person_reads_them() {
        for (one, other, order) in [
            ("file2", "file10", Ordering::Less),
            ("file10", "file2", Ordering::Greater),
            ("File", "file", Ordering::Equal),
            ("a", "B", Ordering::Less),
            ("file02", "file2", Ordering::Equal),
            ("file", "file1", Ordering::Less),
            ("2024-01", "2024-02", Ordering::Less),
            ("x99999999999999999999999", "x100000000000000000000000", Ordering::Less),
            ("", "a", Ordering::Less),
        ] {
            assert_eq!(compare_names(one, other), order, "{one} and {other}");
        }
    }

    #[test]
    fn items_are_sorted_by_what_is_asked() {
        let all = [
            item("b.txt", ItemKind::File, 30, 300),
            item("Folder", ItemKind::Folder, 0, 200),
            item("a.png", ItemKind::File, 10, 400),
            item("c10.txt", ItemKind::File, 20, 100),
            item("c9.txt", ItemKind::File, 20, 100),
        ];
        let names = |sort, descending| -> Vec<&str> {
            sorted(all.iter().collect(), sort, descending)
                .into_iter()
                .map(|item| item.name.as_str())
                .collect()
        };
        for (sort, descending, expected) in [
            (Sort::Manual, false, ["b.txt", "Folder", "a.png", "c10.txt", "c9.txt"]),
            (Sort::Manual, true, ["b.txt", "Folder", "a.png", "c10.txt", "c9.txt"]),
            (Sort::Name, false, ["a.png", "b.txt", "c9.txt", "c10.txt", "Folder"]),
            (Sort::Name, true, ["Folder", "c10.txt", "c9.txt", "b.txt", "a.png"]),
            (Sort::Type, false, ["Folder", "a.png", "b.txt", "c9.txt", "c10.txt"]),
            (Sort::Date, false, ["c9.txt", "c10.txt", "Folder", "b.txt", "a.png"]),
            (Sort::Date, true, ["a.png", "b.txt", "Folder", "c9.txt", "c10.txt"]),
            (Sort::Size, false, ["Folder", "a.png", "c9.txt", "c10.txt", "b.txt"]),
            (Sort::Size, true, ["b.txt", "c9.txt", "c10.txt", "a.png", "Folder"]),
        ] {
            assert_eq!(names(sort, descending), expected, "{sort:?} {descending}");
        }
    }

    #[test]
    fn a_grid_fills_rows_and_says_how_far_it_scrolls() {
        let metrics = Metrics::default();
        // A cell is 76 by 82; with the spacing of 4 three fit into 240.
        let items = ids(&["a", "b", "c", "d", "e", "f", "g"]);
        let layout = lay_out(View::Grid, 48, &metrics, (240, 150), &items, &BTreeMap::new());
        assert_eq!((layout.columns, layout.rows), (3, 3));
        assert_eq!(layout.items[0].rect, Rect::new(0, 0, 76, 82));
        assert_eq!(layout.items[2].rect, Rect::new(160, 0, 76, 82));
        assert_eq!(layout.items[3].rect, Rect::new(0, 86, 76, 82));
        assert_eq!(layout.items[6].cell, Cell { column: 0, row: 2 });
        assert_eq!(layout.content_height, 3 * 82 + 2 * 4);
        assert_eq!(layout.scroll_extent, 254 - 150);
        // Everything fits: nothing to scroll.
        let roomy = lay_out(View::Grid, 48, &metrics, (800, 600), &items, &BTreeMap::new());
        assert_eq!((roomy.columns, roomy.rows, roomy.scroll_extent), (10, 1, 0));
        // Narrower than one cell: one column all the same.
        let narrow = lay_out(View::Grid, 48, &metrics, (10, 100), &items, &BTreeMap::new());
        assert_eq!((narrow.columns, narrow.rows), (1, 7));
        // Nothing in it.
        let empty = lay_out(View::Grid, 48, &metrics, (240, 150), &[], &BTreeMap::new());
        assert_eq!((empty.rows, empty.content_height, empty.scroll_extent), (0, 0, 0));
        // Larger icons, fewer columns.
        let large = lay_out(View::Grid, 96, &metrics, (240, 150), &items, &BTreeMap::new());
        assert_eq!(large.columns, 1);
    }

    #[test]
    fn a_list_has_one_item_a_line() {
        let metrics = Metrics::default();
        let items = ids(&["a", "b", "c"]);
        let layout = lay_out(View::List, 48, &metrics, (240, 40), &items, &BTreeMap::new());
        assert_eq!((layout.columns, layout.rows), (1, 3));
        assert_eq!(layout.items[1].rect, Rect::new(0, 24, 240, 24));
        assert_eq!(layout.content_height, 72);
        assert_eq!(layout.scroll_extent, 32);
    }

    #[test]
    fn an_item_put_on_a_cell_is_there_and_the_others_go_around_it() {
        let metrics = Metrics::default();
        let items = ids(&["a", "b", "c", "d"]);
        let cells = BTreeMap::from([
            (ItemId::new("c"), Cell { column: 0, row: 0 }),
            (ItemId::new("d"), Cell { column: 9, row: 0 }),
        ]);
        let layout = lay_out(View::Grid, 48, &metrics, (240, 300), &items, &cells);
        let cell = |name: &str| {
            layout.items.iter().find(|placed| placed.item.as_str() == name).unwrap().cell
        };
        assert_eq!(cell("c"), Cell { column: 0, row: 0 });
        assert_eq!(cell("a"), Cell { column: 1, row: 0 });
        assert_eq!(cell("b"), Cell { column: 2, row: 0 });
        // The zone has three columns: a cell in the tenth is not kept to.
        assert_eq!(cell("d"), Cell { column: 0, row: 1 });
        let all: BTreeSet<Cell> = layout.items.iter().map(|placed| placed.cell).collect();
        assert_eq!(all.len(), items.len());
    }

    #[test]
    fn loose_items_run_down_the_columns_around_those_with_a_place() {
        let metrics = Metrics::default();
        let items = ids(&["a", "b", "c", "d"]);
        // Rows of 86: two fit into 200.
        let own = BTreeMap::from([(ItemId::new("b"), (3, 90)), (ItemId::new("d"), (5000, 5000))]);
        let placed = lay_out_loose(48, &metrics, (400, 200), &items, &own);
        let rect = |name: &str| placed.iter().find(|placed| placed.item.as_str() == name).unwrap().rect;
        assert_eq!(rect("b"), Rect::new(3, 90, 76, 82));
        assert_eq!(rect("a"), Rect::new(0, 0, 76, 82));
        // The second cell of the first column is b's: c starts the next.
        assert_eq!(rect("c"), Rect::new(80, 0, 76, 82));
        // Brought back into the room.
        assert_eq!(rect("d"), Rect::new(324, 118, 76, 82));
    }

    #[test]
    fn a_rolled_up_zone_is_its_title() {
        let metrics = Metrics::default();
        assert_eq!(rolled_up(Rect::new(10, 20, 300, 200), &metrics), Rect::new(10, 20, 300, 28));
        assert_eq!(
            content_area(Rect::new(10, 20, 300, 200), false, &metrics),
            Rect::new(18, 56, 284, 156)
        );
        assert_eq!(
            content_area(Rect::new(10, 20, 300, 200), true, &metrics),
            Rect::new(18, 82, 284, 130)
        );
        assert_eq!(content_area(Rect::new(0, 0, 10, 10), true, &metrics).height, 0);
    }

    #[test]
    fn zones_with_one_group_name_are_tabs_of_each_other() {
        let zone = |id: &str, group: Option<&str>| ZoneDefinition {
            tab_group: group.map(str::to_string),
            ..ZoneDefinition::manual(id, id)
        };
        let zones = [
            zone("a", Some("one")),
            zone("b", None),
            zone("c", Some("one")),
            zone("d", Some("alone")),
        ];
        let groups = tab_groups(&zones);
        assert_eq!(
            groups,
            [TabGroup { name: "one".to_string(), members: vec![ZoneId::new("a"), ZoneId::new("c")] }]
        );
        assert!(group_of(&groups, &ZoneId::new("c")).is_some());
        assert!(group_of(&groups, &ZoneId::new("d")).is_none());
    }
}
