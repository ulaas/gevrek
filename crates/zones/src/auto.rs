//! Zones made by themselves, one for each type of item on the desktop:
//! folders in one, pictures in another, documents in a third. For a
//! desktop that was never sorted this is the first tidying, and it has to
//! be safe to try: so there is a plan first, which changes nothing and
//! says which zones would be made, where, and what would go into each;
//! carrying the plan out is remembered whole, and can be taken back.
//!
//! What is here is the part that only reckons: of which type an item is,
//! what a plan holds, how large a new zone is and where it goes. Making
//! the zones and moving the items is the core's (`core/auto.rs`), which
//! does exactly what a plan says.
//!
//! The types are the person's to change (`desktop.zones.auto.categories`).
//! A zone that was made for a type carries that type's id
//! (`ZoneDefinition.auto`), so asking again makes no second zone: what is
//! new of the type goes into the one there is, and a desktop that is
//! sorted already gives an empty plan.

use crate::config::{AutoCategory, Matcher, Rule, RuleAction, ZoneKind, ZonesConfig, DEFAULT_ICON_SIZE};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::layout::{self, Metrics};
use crate::placement::{Placements, Whereabouts};
use crate::rules::Scope;
use crate::snap;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// The fewest and the most columns a new zone has, and the most rows it
/// shows at once (what is more is scrolled to). Enough to see what the
/// zone is for at a glance, and never so large that three of them fill a
/// display.
const COLUMNS: (usize, usize) = (2, 5);
const MOST_ROWS: usize = 4;

/// How far the next zone lies from the last where there is no room left
/// and they are laid over what is there.
const CASCADE: i32 = 24;

/// One zone of a plan: an existing one that takes more, or one to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedZone {
    /// The type it is for, by its id.
    pub category: String,
    pub title: String,
    /// The zone that was made for this type before and takes these items;
    /// none where one would be made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// Where it is, or would be: on the desktop, in logical units, as
    /// every zone's rectangle.
    pub rect: Rect,
    /// What would go into it, in the order of the names.
    pub items: Vec<ItemId>,
    /// A rule would be added that sends what comes later and is of this
    /// type into it.
    #[serde(default)]
    pub rule: bool,
}

/// What making zones by type would do.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoPlan {
    /// In the order of the types. None: there is nothing to do.
    pub zones: Vec<PlannedZone>,
    /// The items nothing is done with: too few of their type for a zone
    /// of its own, of no type at all, or of a type whose zone is locked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<ItemId>,
    /// The display new zones would be made on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayId>,
}

/// A zone that was made by type, and for which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoMade {
    pub zone: ZoneId,
    pub category: String,
}

/// An item that was put into a zone by type, and where it was before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoMoved {
    pub item: ItemId,
    pub before: Whereabouts,
    /// The zone it was put into.
    pub zone: ZoneId,
}

/// What the last making of zones by type did, kept so that it can be
/// taken back: the zones it made, where every item it moved was before,
/// and the rules it added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoUndo {
    /// When, in seconds since 1970.
    pub made_at: u64,
    pub zones: Vec<AutoMade>,
    pub entries: Vec<AutoMoved>,
    pub rules: Vec<Rule>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: gevrek_model::Extra,
}

/// The type an item is of: the first that names its kind; if none does,
/// the first that names its extension; if none does, the first whose MIME
/// beginnings fit; and at last the first that names nothing, which takes
/// the rest. What is no file at all (This PC, the bin) is of no type: it
/// is never sorted.
///
/// Kind before extension, because a folder called `photos.png` is a
/// folder; extension before MIME, because the extension is what the
/// person sees and what systems guess the MIME type from anyway.
pub fn category_of<'a>(categories: &'a [AutoCategory], item: &Item) -> Option<&'a AutoCategory> {
    if item.kind == ItemKind::Virtual {
        return None;
    }
    let by_kind = || categories.iter().find(|category| category.kinds.contains(&item.kind));
    let by_extension = || {
        let extension = item.facts.extension.as_ref()?;
        categories.iter().find(|category| {
            category
                .extensions
                .iter()
                .any(|wanted| wanted.trim_start_matches('.').eq_ignore_ascii_case(extension))
        })
    };
    let by_mime = || {
        let mime = item.facts.mime.as_ref()?.to_lowercase();
        categories.iter().find(|category| {
            category
                .mime_prefixes
                .iter()
                .any(|prefix| !prefix.is_empty() && mime.starts_with(&prefix.to_lowercase()))
        })
    };
    by_kind()
        .or_else(by_extension)
        .or_else(by_mime)
        .or_else(|| categories.iter().find(|category| category.takes_the_rest()))
}

/// What a rule made for a type is called, so that the person knows it in
/// the list of rules (and so that taking the zones back finds it).
pub fn rule_name(title: &str) -> String {
    format!("Auto: {title}")
}

/// The rule that sends what comes later and is of a type into its zone.
///
/// A rule holds when all its matchers do, and a type is any of several
/// things, so one rule says one of them: the type's extensions where it
/// has some (for a file that arrives, the extension is what tells);
/// else its kinds; else its one MIME beginning. A type that names
/// nothing (the one that takes the rest) has no rule: a rule that matches
/// everything would undo every other rule that stands after it. What such
/// a rule does not catch (a shortcut without one of the extensions, a
/// picture known by its MIME type alone) is caught by asking for the
/// zones by type again.
pub fn rule_for(category: &AutoCategory, zone: &ZoneId) -> Option<Rule> {
    let kinds: Vec<ItemKind> =
        category.kinds.iter().copied().filter(|kind| *kind != ItemKind::Virtual).collect();
    let matcher = if !category.extensions.is_empty() {
        Matcher::Extension { any_of: category.extensions.clone() }
    } else if !kinds.is_empty() {
        Matcher::Kind { any_of: kinds }
    } else if let [prefix] = category.mime_prefixes.as_slice() {
        Matcher::Mime { pattern: format!("{prefix}*") }
    } else {
        return None;
    };
    Some(Rule {
        name: rule_name(title_of(category)),
        matchers: vec![matcher],
        action: RuleAction::AssignToZone { zone: zone.clone() },
        ..Rule::default()
    })
}

/// What a type's zone is called: its title, or its id where it has none.
fn title_of(category: &AutoCategory) -> &str {
    if category.title.trim().is_empty() {
        &category.id
    } else {
        &category.title
    }
}

/// How large a new zone is: as it is wanted, and the least it is made
/// when the room is short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    pub full: (i32, i32),
    pub least: (i32, i32),
}

/// The size of a zone that holds `columns` by `rows` icons exactly.
fn size_of(columns: usize, rows: usize, icon_size: u32, metrics: &Metrics) -> (i32, i32) {
    let (cell_width, cell_height) = metrics.cell(crate::config::View::Grid, icon_size, 0);
    let (columns, rows) = (columns as i32, rows as i32);
    let (least_width, least_height) = metrics.least();
    (
        (2 * metrics.padding + columns * cell_width + (columns - 1) * metrics.spacing).max(least_width),
        (metrics.title_height + 2 * metrics.padding + rows * cell_height + (rows - 1) * metrics.spacing)
            .max(least_height),
    )
}

/// How large a zone for `count` items is made: about as many columns as
/// rows (2 to 5 columns), and at most 4 rows to be seen; more items than
/// that are scrolled to. At the least it is 2 columns and one row.
pub fn size_for(count: usize, icon_size: u32, metrics: &Metrics) -> Sizes {
    let mut columns = COLUMNS.0;
    while columns < COLUMNS.1 && columns * columns < count {
        columns += 1;
    }
    let rows = count.div_ceil(columns).clamp(1, MOST_ROWS);
    Sizes {
        full: size_of(columns, rows, icon_size, metrics),
        least: size_of(COLUMNS.0, 1, icon_size, metrics),
    }
}

/// Whether two rectangles are nearer than `gap` to each other (with no
/// gap: whether they overlap).
fn crowds(one: Rect, other: Rect, gap: i32) -> bool {
    one.x < other.x + other.width + gap
        && other.x - gap < one.x + one.width
        && one.y < other.y + other.height + gap
        && other.y - gap < one.y + one.height
}

/// Where a row of new zones has come to.
#[derive(Debug, Clone, Copy)]
struct Shelf {
    x: i32,
    y: i32,
    /// The lowest bottom of what was put into this row; `y` while it is
    /// empty.
    bottom: i32,
}

/// The first free place for a zone of a size from where the rows have
/// come to, and where they have come to with it: along the row, past
/// whatever is in the way, and into the next row where this one is full.
/// None when no row has room for it.
fn find(shelf: Shelf, (width, height): (i32, i32), inner: Rect, blocked: &[Rect], gap: i32) -> Option<(Rect, Shelf)> {
    let (right, bottom) = (inner.x + inner.width, inner.y + inner.height);
    let mut shelf = shelf;
    loop {
        if shelf.y + height > bottom {
            return None;
        }
        let mut x = shelf.x;
        while x + width <= right {
            let wanted = Rect::new(x, shelf.y, width, height);
            let past = blocked
                .iter()
                .filter(|other| crowds(wanted, **other, gap))
                .map(|other| other.x + other.width + gap)
                .max();
            match past {
                // (Further right every time: what crowds it ends right
                // of where it begins.)
                Some(past) => x = past.max(x + 1),
                None => {
                    let shelf = Shelf { x: x + width + gap, y: shelf.y, bottom: shelf.bottom.max(shelf.y + height) };
                    return Some((wanted, shelf));
                }
            }
        }
        // The next row: below what this one holds; where it holds nothing
        // of ours, below the nearest thing that was in the way.
        let next = if shelf.bottom > shelf.y {
            shelf.bottom + gap
        } else {
            let band = Rect::new(inner.x, shelf.y, inner.width, height);
            blocked
                .iter()
                .filter(|other| crowds(band, **other, gap))
                .map(|other| other.y + other.height + gap)
                .filter(|below| *below > shelf.y)
                .min()?
        };
        shelf = Shelf { x: inner.x, y: next, bottom: next };
    }
}

/// Where new zones of these sizes go in a free room (`area`, what the
/// panels leave of a display), in their order: in rows from the top left
/// corner, `gap` between them and to the room's sides, the tops of a row
/// in one line, never over what is there already (`taken`) and never over
/// each other, never outside the room.
///
/// Where a zone has no room as large as it is wanted, it is made as small
/// as a zone for two icons. Where there is no room even for that, the
/// zones that are left are laid over what is there, each a step down and
/// right of the last, still wholly inside the room: a zone the person has
/// to pull out from under another is better than one that was not made.
/// So every size gets a place, whatever the room.
pub fn place(area: Rect, gap: i32, taken: &[Rect], sizes: &[Sizes]) -> Vec<Rect> {
    let gap = gap.max(0);
    // (A room too small for its own margins has none.)
    let inner = if area.width > 2 * gap && area.height > 2 * gap {
        Rect::new(area.x + gap, area.y + gap, area.width - 2 * gap, area.height - 2 * gap)
    } else {
        area
    };
    let fitted = |(width, height): (i32, i32)| (width.min(inner.width).max(1), height.min(inner.height).max(1));
    let mut blocked: Vec<Rect> = taken.to_vec();
    let mut shelf = Shelf { x: inner.x, y: inner.y, bottom: inner.y };
    let mut cascaded = 0;
    let mut placed = Vec::with_capacity(sizes.len());
    for size in sizes {
        let found = [size.full, size.least]
            .into_iter()
            .find_map(|size| find(shelf, fitted(size), inner, &blocked, gap));
        let rect = match found {
            Some((rect, next)) => {
                shelf = next;
                rect
            }
            None => {
                let (width, height) = fitted(size.least);
                let step = CASCADE * cascaded;
                cascaded += 1;
                snap::keep_reachable(Rect::new(inner.x + step, inner.y + step, width, height), inner)
            }
        };
        blocked.push(rect);
        placed.push(rect);
    }
    placed
}

/// Where new zones would be made.
#[derive(Debug, Clone, Copy)]
pub struct Room<'a> {
    pub display: &'a DisplayId,
    /// The free room of that display, on the desktop.
    pub area: Rect,
    /// The zones that are there already, on the desktop.
    pub taken: &'a [Rect],
    pub metrics: &'a Metrics,
}

/// What is asked of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    /// `Loose`: the items that are in no zone. `All`: those, and the
    /// items of the zones the person filled by hand; what is in a zone
    /// that was made by type stays there.
    pub scope: Scope,
    /// A zone is made for a type with at least this many items.
    pub min_items: u32,
    /// Each zone that is made gets a rule for what comes later.
    pub add_rules: bool,
}

/// What making zones by type would do with `items` as the placements are
/// now. Nothing is changed.
///
/// A type that has a zone already (the first one filled by hand that
/// carries its id) gets what is new of it, however little; any other type
/// gets a zone when it has at least `min_items` items, and its items are
/// left where they are when it has fewer. Items in a zone that is locked
/// stay there, a zone that is locked takes nothing, and with the lock for
/// all zones nothing goes anywhere. `rect_of` says where an existing zone
/// is.
pub fn plan<'a>(
    config: &ZonesConfig,
    items: impl IntoIterator<Item = &'a Item>,
    placements: &Placements,
    asked: Asked,
    room: &Room,
    rect_of: &dyn Fn(&ZoneId) -> Option<Rect>,
) -> AutoPlan {
    let categories = &config.auto.categories;
    let mut plan = AutoPlan { display: Some(room.display.clone()), ..AutoPlan::default() };
    // The zone an item is shown in: one that is there and filled by hand.
    let zone_of = |item: &ItemId| {
        placements
            .zone_of(item)
            .and_then(|zone| config.zone(zone))
            .filter(|zone| zone.kind == ZoneKind::Manual)
    };
    let locked = |zone: &crate::config::ZoneDefinition| zone.lock.members || config.lock.members;
    let mut of_type: Vec<Vec<&Item>> = vec![Vec::new(); categories.len()];
    let mut of_none: Vec<&Item> = Vec::new();
    for item in items {
        if item.kind == ItemKind::Virtual {
            continue;
        }
        match zone_of(&item.id) {
            None => {}
            Some(zone) if asked.scope == Scope::All && zone.auto.is_none() && !locked(zone) => {}
            Some(_) => continue,
        }
        let place = category_of(categories, item)
            .and_then(|category| categories.iter().position(|other| std::ptr::eq(other, category)));
        match place {
            Some(place) => of_type[place].push(item),
            None => of_none.push(item),
        }
    }
    let by_name = |items: &mut Vec<&Item>| {
        items.sort_by(|a, b| layout::compare_names(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
    };
    let ids = |items: &[&Item]| items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    let mut wanted: Vec<Sizes> = Vec::new();
    let mut made: Vec<usize> = Vec::new();
    for (category, mut items) in categories.iter().zip(of_type) {
        if items.is_empty() {
            continue;
        }
        by_name(&mut items);
        let existing = config
            .zones
            .iter()
            .find(|zone| zone.kind == ZoneKind::Manual && zone.auto.as_deref() == Some(category.id.as_str()));
        match existing {
            _ if config.lock.members => plan.left.extend(ids(&items)),
            Some(zone) if locked(zone) => plan.left.extend(ids(&items)),
            Some(zone) => plan.zones.push(PlannedZone {
                category: category.id.clone(),
                title: zone.title.clone(),
                zone: Some(zone.id.clone()),
                rect: rect_of(&zone.id).unwrap_or_default(),
                items: ids(&items),
                rule: false,
            }),
            None if items.len() < asked.min_items.max(1) as usize => plan.left.extend(ids(&items)),
            None => {
                wanted.push(size_for(items.len(), DEFAULT_ICON_SIZE, room.metrics));
                made.push(plan.zones.len());
                plan.zones.push(PlannedZone {
                    category: category.id.clone(),
                    title: title_of(category).to_string(),
                    zone: None,
                    rect: Rect::default(),
                    items: ids(&items),
                    rule: asked.add_rules && rule_for(category, &ZoneId::default()).is_some(),
                });
            }
        }
    }
    by_name(&mut of_none);
    plan.left.extend(ids(&of_none));
    let gap = if config.snap.gap > 0 { config.snap.gap } else { 0 };
    for (place, rect) in made.into_iter().zip(place(room.area, gap, room.taken, &wanted)) {
        plan.zones[place].rect = rect;
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZoneDefinition;
    use crate::item::ItemFacts;

    fn file(name: &str) -> Item {
        Item::file(&format!("/desk/{name}"))
    }

    fn of_kind(name: &str, kind: ItemKind) -> Item {
        Item { kind, ..file(name) }
    }

    fn with_mime(name: &str, mime: &str) -> Item {
        let mut item = file(name);
        item.facts = ItemFacts { mime: Some(mime.to_string()), ..item.facts };
        item
    }

    fn id(name: &str) -> ItemId {
        ItemId::new(format!("/desk/{name}"))
    }

    fn config() -> ZonesConfig {
        ZonesConfig { enabled: true, ..ZonesConfig::default() }
    }

    const AREA: Rect = Rect { x: 0, y: 40, width: 1920, height: 1040 };

    fn planned(config: &ZonesConfig, items: &[Item], placements: &Placements, scope: Scope, taken: &[Rect]) -> AutoPlan {
        let display = DisplayId::new("d");
        let metrics = Metrics::default();
        let room = Room { display: &display, area: AREA, taken, metrics: &metrics };
        let asked = Asked { scope, min_items: config.auto.min_items, add_rules: config.auto.add_rules };
        plan(config, items, placements, asked, &room, &|_| Some(Rect::new(1000, 500, 300, 200)))
    }

    fn overlap(one: Rect, other: Rect) -> bool {
        crowds(one, other, 0)
    }

    #[test]
    fn an_item_is_of_the_type_its_kind_its_extension_or_its_mime_says() {
        let categories = AutoCategory::built_in();
        let type_of = |item: &Item| category_of(&categories, item).map(|category| category.id.as_str());
        for (item, expected) in [
            (Item::folder("/desk/Projects"), Some("folders")),
            // Kind before extension: a folder is a folder whatever it is called.
            (of_kind("photos.png", ItemKind::Folder), Some("folders")),
            (of_kind("Editor.lnk", ItemKind::Shortcut), Some("programs")),
            (of_kind("report.pdf", ItemKind::Shortcut), Some("programs")),
            (of_kind("Terminal", ItemKind::Launcher), Some("programs")),
            (file("setup.EXE"), Some("programs")),
            (file("tool.AppImage"), Some("programs")),
            (file("report.pdf"), Some("documents")),
            (file("Budget 2026.xlsx"), Some("documents")),
            (file("cat.JPG"), Some("pictures")),
            (file("film.mkv"), Some("video")),
            (file("song.flac"), Some("audio")),
            (file("old.tar.gz"), Some("archives")),
            (file("main.rs"), Some("code")),
            (file("page.html"), Some("code")),
            // Extension before MIME: `.svg` is a picture though it is text.
            (with_mime("drawing.svg", "text/xml"), Some("pictures")),
            // By MIME alone, where the name says nothing.
            (with_mime("scan", "image/png"), Some("pictures")),
            (with_mime("LICENSE", "text/plain"), Some("documents")),
            (with_mime("letter", "application/vnd.oasis.opendocument.text"), Some("documents")),
            (with_mime("clip", "VIDEO/mp4"), Some("video")),
            (with_mime("voice", "audio/ogg"), Some("audio")),
            // What nothing names goes to the type that takes the rest.
            (file("Makefile"), Some("other")),
            (file("data.xyz"), Some("other")),
            (with_mime("blob", "application/octet-stream"), Some("other")),
            // What is no file at all is of no type.
            (of_kind("This PC", ItemKind::Virtual), None),
            (of_kind("bin.png", ItemKind::Virtual), None),
        ] {
            assert_eq!(type_of(&item), expected, "{}", item.name);
        }
    }

    #[test]
    fn the_order_of_the_types_breaks_a_tie_and_without_a_rest_some_items_are_of_none() {
        let category = |id: &str, extensions: &[&str], mime: &[&str], kinds: &[ItemKind]| AutoCategory {
            id: id.to_string(),
            title: id.to_uppercase(),
            extensions: extensions.iter().map(|text| text.to_string()).collect(),
            mime_prefixes: mime.iter().map(|text| text.to_string()).collect(),
            kinds: kinds.to_vec(),
            ..AutoCategory::default()
        };
        let categories = vec![
            category("notes", &["txt", ".MD"], &["text/"], &[]),
            category("texts", &["txt", "rtf"], &["text/"], &[]),
            category("files", &[], &[], &[ItemKind::File]),
        ];
        let type_of = |item: &Item| category_of(&categories, item).map(|category| category.id.as_str());
        // Kind is asked first, so the last one takes every file.
        assert_eq!(type_of(&file("a.txt")), Some("files"));
        let categories = categories[..2].to_vec();
        let type_of = |item: &Item| category_of(&categories, item).map(|category| category.id.as_str());
        assert_eq!(type_of(&file("a.txt")), Some("notes"), "the first that names it");
        assert_eq!(type_of(&file("a.md")), Some("notes"), "written with a dot and in capitals");
        assert_eq!(type_of(&file("a.rtf")), Some("texts"));
        assert_eq!(type_of(&with_mime("a", "text/plain")), Some("notes"));
        assert_eq!(type_of(&file("a.bin")), None, "no type takes the rest");
        assert_eq!(category_of(&[], &file("a.txt")), None);
    }

    #[test]
    fn a_types_rule_asks_for_what_tells_the_type() {
        let categories = AutoCategory::built_in();
        let zone = ZoneId::new("zone-7");
        let rule = |id: &str| {
            let category = categories.iter().find(|category| category.id == id).unwrap();
            rule_for(category, &zone)
        };
        let pictures = rule("pictures").unwrap();
        assert_eq!(pictures.name, "Auto: Pictures");
        assert!(pictures.enabled);
        assert_eq!(pictures.action, RuleAction::AssignToZone { zone: zone.clone() });
        assert!(matches!(pictures.matchers.as_slice(), [Matcher::Extension { any_of }] if any_of.contains(&"png".to_string())));
        assert_eq!(rule("folders").unwrap().matchers, [Matcher::Kind { any_of: vec![ItemKind::Folder] }]);
        assert!(matches!(rule("programs").unwrap().matchers.as_slice(), [Matcher::Extension { .. }]));
        // A rule that would match everything is not made.
        assert_eq!(rule("other"), None);
        // By MIME where that is all the type names, and it names one.
        let by_mime = AutoCategory {
            id: "fonts".to_string(),
            mime_prefixes: vec!["font/".to_string()],
            ..AutoCategory::default()
        };
        let made = rule_for(&by_mime, &zone).unwrap();
        assert_eq!(made.name, "Auto: fonts", "called by its id where it has no title");
        assert_eq!(made.matchers, [Matcher::Mime { pattern: "font/*".to_string() }]);
        let two = AutoCategory { mime_prefixes: vec!["a/".to_string(), "b/".to_string()], ..by_mime };
        assert_eq!(rule_for(&two, &zone), None, "one rule cannot say either of two");
        // Each rule matches what its type takes by that fact.
        for (name, id) in [("cat.png", "pictures"), ("setup.exe", "programs"), ("a.pdf", "documents")] {
            assert!(crate::rules::matches(&rule(id).unwrap(), &file(name), 0), "{name}");
        }
        assert!(crate::rules::matches(&rule("folders").unwrap(), &Item::folder("/desk/x"), 0));
        assert!(!crate::rules::matches(&rule("folders").unwrap(), &file("x.png"), 0));
    }

    #[test]
    fn a_new_zone_is_as_large_as_what_it_holds_within_limits() {
        let metrics = Metrics::default();
        // 48 units an icon: a cell is 76 by 82, 4 apart, 8 around, 28 of title.
        let size = |columns: i32, rows: i32| (16 + columns * 76 + (columns - 1) * 4, 28 + 16 + rows * 82 + (rows - 1) * 4);
        for (count, columns, rows) in [
            (0, 2, 1),
            (1, 2, 1),
            (2, 2, 1),
            (3, 2, 2),
            (4, 2, 2),
            (5, 3, 2),
            (9, 3, 3),
            (10, 4, 3),
            (16, 4, 4),
            (17, 5, 4),
            (20, 5, 4),
            // More than is shown at once: it scrolls.
            (21, 5, 4),
            (500, 5, 4),
        ] {
            let sizes = size_for(count, DEFAULT_ICON_SIZE, &metrics);
            assert_eq!(sizes.full, size(columns, rows), "{count} items");
            assert_eq!(sizes.least, size(2, 1), "{count} items");
            // The zone laid out at that size has exactly those columns.
            let content = layout::content_area(Rect::new(0, 0, sizes.full.0, sizes.full.1), false, &metrics);
            let items: Vec<ItemId> = (0..count).map(|number| ItemId::new(number.to_string())).collect();
            let laid = layout::lay_out(
                crate::config::View::Grid,
                DEFAULT_ICON_SIZE,
                &metrics,
                (content.width, content.height),
                &items,
                &Default::default(),
            );
            assert_eq!(laid.columns as i32, columns, "{count} items");
            if count <= 20 {
                assert_eq!(laid.scroll_extent, 0, "{count} items are all seen");
            }
        }
        // Never smaller than a zone may be.
        let tiny = Metrics { least_width: 400, least_height: 300, ..metrics };
        assert_eq!(size_for(1, 16, &tiny).full, (400, 300));
    }

    #[test]
    fn new_zones_stand_in_rows_from_the_top_left_and_keep_off_what_is_there() {
        let one = Sizes { full: (300, 200), least: (172, 126) };
        let tall = Sizes { full: (300, 400), least: (172, 126) };
        let area = Rect::new(100, 50, 1000, 700);
        // (what it shows, gap, what is there, the sizes, where they go)
        type Case = (&'static str, i32, Vec<Rect>, Vec<Sizes>, Vec<Rect>);
        let cases: Vec<Case> = vec![
            (
                "a row, then the next below the tallest of it",
                8,
                vec![],
                vec![one, tall, one, one, one],
                vec![
                    Rect::new(108, 58, 300, 200),
                    Rect::new(416, 58, 300, 400),
                    Rect::new(724, 58, 300, 200),
                    Rect::new(108, 466, 300, 200),
                    Rect::new(416, 466, 300, 200),
                ],
            ),
            (
                "without a gap they touch",
                0,
                vec![],
                vec![one, one, one, one],
                vec![
                    Rect::new(100, 50, 300, 200),
                    Rect::new(400, 50, 300, 200),
                    Rect::new(700, 50, 300, 200),
                    Rect::new(100, 250, 300, 200),
                ],
            ),
            (
                "past a zone that is in the way, the gap kept to it",
                8,
                vec![Rect::new(300, 100, 200, 100)],
                vec![one, one],
                vec![Rect::new(508, 58, 300, 200), Rect::new(108, 266, 300, 200)],
            ),
            (
                "a row that is wholly taken is passed",
                8,
                vec![Rect::new(100, 50, 1000, 300)],
                vec![one],
                vec![Rect::new(108, 358, 300, 200)],
            ),
            (
                "what does not fit as wanted is made small",
                8,
                vec![Rect::new(100, 50, 1000, 520)],
                vec![one, one],
                vec![Rect::new(108, 578, 172, 126), Rect::new(288, 578, 172, 126)],
            ),
            (
                "a zone outside the room is in nobody's way",
                8,
                vec![Rect::new(2000, 50, 500, 500), Rect::new(100, -400, 500, 300)],
                vec![one],
                vec![Rect::new(108, 58, 300, 200)],
            ),
        ];
        for (name, gap, taken, sizes, expected) in cases {
            let placed = place(area, gap, &taken, &sizes);
            assert_eq!(placed, expected, "{name}");
            assert_eq!(place(area, gap, &taken, &sizes), placed, "{name}: the same every time");
        }
    }

    #[test]
    fn every_new_zone_has_a_place_inside_the_room_whatever_the_room() {
        let metrics = Metrics::default();
        let counts = [1usize, 2, 3, 5, 8, 13, 21, 40, 4, 6];
        let sizes: Vec<Sizes> = counts.iter().map(|count| size_for(*count, DEFAULT_ICON_SIZE, &metrics)).collect();
        // (area, gap, taken, whether there is room for all)
        let rooms: Vec<(Rect, i32, Vec<Rect>, bool)> = vec![
            (Rect::new(0, 0, 3840, 2160), 8, vec![], true),
            (Rect::new(0, 40, 1920, 1040), 8, vec![], true),
            (Rect::new(-1920, -200, 1920, 1080), 16, vec![Rect::new(-1900, -180, 320, 240)], true),
            (
                Rect::new(0, 40, 3840, 2120),
                8,
                vec![Rect::new(40, 80, 320, 240), Rect::new(900, 300, 400, 300), Rect::new(1500, 60, 300, 900)],
                true,
            ),
            // Zones in the way of nearly every row: no room for all.
            (
                Rect::new(0, 40, 1920, 1040),
                8,
                vec![Rect::new(40, 80, 320, 240), Rect::new(900, 300, 400, 300), Rect::new(1500, 60, 300, 900)],
                false,
            ),
            (Rect::new(0, 0, 3000, 2000), 200, vec![], true),
            // No room for all: the rest is laid over what is there.
            (Rect::new(0, 0, 800, 600), 8, vec![], false),
            (Rect::new(0, 0, 1920, 1080), 8, vec![Rect::new(0, 0, 1920, 1080)], false),
            (Rect::new(10, 10, 100, 60), 8, vec![], false),
            (Rect::new(0, 0, 1, 1), 8, vec![], false),
            (Rect::new(0, 0, 1920, 1080), -5, vec![], true),
        ];
        for (area, gap, taken, room_for_all) in rooms {
            let placed = place(area, gap, &taken, &sizes);
            assert_eq!(placed.len(), sizes.len(), "{area:?}: every zone has a place");
            assert_eq!(place(area, gap, &taken, &sizes), placed, "{area:?}: the same every time");
            for rect in &placed {
                assert!(rect.width >= 1 && rect.height >= 1 && snap::within(*rect, area), "{rect:?} in {area:?}");
            }
            if !room_for_all {
                continue;
            }
            let gap = gap.max(0);
            for (index, rect) in placed.iter().enumerate() {
                // The gap to the room's sides, to what is there, and to each other.
                assert!(
                    rect.x >= area.x + gap
                        && rect.y >= area.y + gap
                        && rect.x + rect.width <= area.x + area.width - gap
                        && rect.y + rect.height <= area.y + area.height - gap,
                    "{rect:?} keeps {gap} to the sides of {area:?}"
                );
                for other in taken.iter().chain(&placed[..index]) {
                    assert!(!crowds(*rect, *other, gap), "{rect:?} is on or too near {other:?} in {area:?}");
                }
                assert_eq!((rect.width, rect.height), sizes[index].full, "as large as wanted, where there is room");
            }
        }
    }

    #[test]
    fn a_plan_makes_a_zone_for_each_type_that_has_enough() {
        let items = vec![
            file("b.png"),
            file("a.png"),
            file("c.jpg"),
            file("report.pdf"),
            file("notes.txt"),
            file("song.mp3"),
            Item::folder("/desk/Projects"),
            Item::folder("/desk/Archive"),
            file("Makefile"),
            file("data.xyz"),
            of_kind("This PC", ItemKind::Virtual),
        ];
        let config = config();
        let plan = planned(&config, &items, &Placements::default(), Scope::Loose, &[]);
        assert_eq!(plan.display, Some(DisplayId::new("d")));
        let said: Vec<(&str, &str, Vec<&str>, bool)> = plan
            .zones
            .iter()
            .map(|zone| {
                assert_eq!(zone.zone, None);
                (
                    zone.category.as_str(),
                    zone.title.as_str(),
                    zone.items.iter().map(|item| item.as_str()).collect(),
                    zone.rule,
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("folders", "Folders", vec!["/desk/Archive", "/desk/Projects"], true),
                ("documents", "Documents", vec!["/desk/notes.txt", "/desk/report.pdf"], true),
                ("pictures", "Pictures", vec!["/desk/a.png", "/desk/b.png", "/desk/c.jpg"], true),
                // (The rest has a zone, and no rule: it would match everything.)
                ("other", "Other", vec!["/desk/data.xyz", "/desk/Makefile"], false),
            ]
        );
        // One song is too few for a zone; This PC is never sorted.
        assert_eq!(plan.left, [id("song.mp3")]);
        // In a row from the top left, the gap between and to the sides.
        let rects: Vec<Rect> = plan.zones.iter().map(|zone| zone.rect).collect();
        assert_eq!(rects[0], Rect::new(8, 48, 172, 126));
        assert_eq!((rects[1].x, rects[1].y), (188, 48));
        for (index, rect) in rects.iter().enumerate() {
            assert!(snap::within(*rect, AREA));
            assert!(rects[..index].iter().all(|other| !crowds(*rect, *other, 8)));
        }

        // With the least number at 1 every type has its zone; at 3, one.
        let mut eager = config.clone();
        eager.auto.min_items = 1;
        let plan = planned(&eager, &items, &Placements::default(), Scope::Loose, &[]);
        assert_eq!(plan.zones.len(), 5);
        assert_eq!(plan.left, []);
        let mut strict = config.clone();
        strict.auto.min_items = 3;
        strict.auto.add_rules = false;
        let plan = planned(&strict, &items, &Placements::default(), Scope::Loose, &[]);
        assert_eq!(plan.zones.len(), 1);
        assert_eq!((plan.zones[0].category.as_str(), plan.zones[0].rule), ("pictures", false));
        assert_eq!(plan.left.len(), 7);
        // Nothing asked for means one: a zone for no item is never made.
        strict.auto.min_items = 0;
        assert_eq!(planned(&strict, &items, &Placements::default(), Scope::Loose, &[]).zones.len(), 5);

        // New zones keep off the ones that are there.
        let there = [Rect::new(0, 40, 600, 300)];
        let plan = planned(&config, &items, &Placements::default(), Scope::Loose, &there);
        assert!(plan.zones.iter().all(|zone| !overlap(zone.rect, there[0])));
        // No types at all: nothing is planned, and everything is left.
        let mut none = config.clone();
        none.auto.categories.clear();
        let plan = planned(&none, &items, &Placements::default(), Scope::Loose, &[]);
        assert!(plan.zones.is_empty());
        assert_eq!(plan.left.len(), 10);
    }

    #[test]
    fn a_plan_reuses_the_zone_of_a_type_and_leaves_what_is_sorted() {
        let items = vec![file("a.png"), file("b.png"), file("c.png"), file("d.pdf"), file("e.pdf"), file("f.mp3")];
        let mut config = config();
        config.zones = vec![
            ZoneDefinition { auto: Some("pictures".to_string()), ..ZoneDefinition::manual("pics", "My pictures") },
            ZoneDefinition::manual("inbox", "Inbox"),
            ZoneDefinition { auto: Some("audio".to_string()), ..ZoneDefinition::manual("music", "Music") },
        ];
        let mut placements = Placements::default();
        placements.assign(&id("a.png"), &ZoneId::new("pics"), None, None);
        placements.assign(&id("b.png"), &ZoneId::new("inbox"), None, None);
        placements.assign(&id("d.pdf"), &ZoneId::new("inbox"), None, None);

        // Loose: what is in a zone stays; the one loose picture goes to
        // the zone there is, though it is one alone, and so the song.
        let plan = planned(&config, &items, &placements, Scope::Loose, &[]);
        let said: Vec<(&str, Option<&str>, Vec<&str>)> = plan
            .zones
            .iter()
            .map(|zone| {
                (
                    zone.title.as_str(),
                    zone.zone.as_ref().map(ZoneId::as_str),
                    zone.items.iter().map(|item| item.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("My pictures", Some("pics"), vec!["/desk/c.png"]),
                ("Music", Some("music"), vec!["/desk/f.mp3"]),
            ]
        );
        assert!(plan.zones.iter().all(|zone| !zone.rule && zone.rect == Rect::new(1000, 500, 300, 200)));
        assert_eq!(plan.left, [id("e.pdf")], "one document alone gets no zone");

        // All: also what was sorted by hand; not what is in a zone made by type.
        let plan = planned(&config, &items, &placements, Scope::All, &[]);
        let pictures = &plan.zones[1];
        assert_eq!(plan.zones[0].title, "Documents");
        assert_eq!(plan.zones[0].items, [id("d.pdf"), id("e.pdf")]);
        assert_eq!(pictures.items, [id("b.png"), id("c.png")], "a.png is in its type's zone already");
        assert_eq!(plan.left, []);

        // Locks: a locked zone gives nothing and takes nothing.
        config.zones[1].lock.members = true;
        let plan = planned(&config, &items, &placements, Scope::All, &[]);
        assert_eq!(plan.zones.len(), 2);
        assert_eq!(plan.zones[0].items, [id("c.png")]);
        assert_eq!(plan.left, [id("e.pdf")]);
        config.zones[0].lock.members = true;
        let plan = planned(&config, &items, &placements, Scope::Loose, &[]);
        assert_eq!(plan.zones.len(), 1, "only the song goes anywhere");
        assert_eq!(plan.left, [id("e.pdf"), id("c.png")]);
        // The lock for all zones: nothing goes anywhere.
        config.lock.members = true;
        let plan = planned(&config, &items, &placements, Scope::Loose, &[]);
        assert!(plan.zones.is_empty());
        assert_eq!(plan.left, [id("e.pdf"), id("c.png"), id("f.mp3")]);
    }

    #[test]
    fn a_zone_that_is_no_longer_filled_by_hand_is_not_its_types_zone() {
        let items = vec![file("a.png"), file("b.png")];
        let mut config = config();
        config.zones = vec![ZoneDefinition {
            auto: Some("pictures".to_string()),
            kind: ZoneKind::Portal { directory: "/p".to_string() },
            ..ZoneDefinition::manual("pics", "Pictures")
        }];
        let plan = planned(&config, &items, &Placements::default(), Scope::Loose, &[]);
        assert_eq!(plan.zones.len(), 1);
        assert_eq!(plan.zones[0].zone, None, "a portal takes nothing: a zone is made");
    }

    #[test]
    fn a_plan_and_what_is_kept_to_take_it_back_survive_their_json() {
        let items = vec![file("a.png"), file("b.png"), file("c.mp3")];
        let plan = planned(&config(), &items, &Placements::default(), Scope::Loose, &[]);
        let text = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<AutoPlan>(&text).unwrap(), plan);
        assert!(!text.contains("\"zone\""), "a zone that is not there yet is not named");
        let empty = serde_json::to_value(AutoPlan::default()).unwrap();
        assert_eq!(empty, serde_json::json!({"zones": []}));
        let undo: AutoUndo = serde_json::from_str(
            r#"{"made_at": 5, "later": true,
                "zones": [{"zone": "zone-1", "category": "pictures"}],
                "entries": [{"item": "/desk/a.png", "before": {"in": "loose"}, "zone": "zone-1"}],
                "rules": [{"name": "Auto: Pictures"}]}"#,
        )
        .unwrap();
        assert_eq!(undo.entries[0].before, Whereabouts::Loose { position: None });
        let written = serde_json::to_value(&undo).unwrap();
        assert_eq!(written["later"], true, "a key of a newer version is kept");
        assert_eq!(serde_json::from_value::<AutoUndo>(written).unwrap(), undo);
    }
}
