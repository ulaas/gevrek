//! Long runs of things nobody thought of in that order, with what must
//! always hold looked at after each step.
//!
//! One kind of run is for the rules: items, rules and placements made by
//! chance, then a dry run, applying and taking back, over and over. The
//! other is for the layout: displays come, go, change places, sizes and
//! scales, and come back, while zones are made, moved, tabbed and
//! deleted between. After every step: no item is in two zones, no zone is
//! lost, every zone can be reached on a display that is there, a set of
//! displays that returns has its zones where they were, and the state
//! comes back from its file as it was.
//!
//! The runs are the same every time (the numbers come from a seed);
//! `GEVREK_RUNS=<n>` makes n times as many, others. A run that fails
//! names its seed: `GEVREK_SEED=0x...` runs that one alone.

use crate::backend::{BackendEvent, FileOperation, Location};
use crate::config::{
    DateField, Matcher, Rule, RuleAction, Sort, View, ZoneDefinition, ZoneKind, ZonesConfig,
};
use crate::core::{Command, Effect, ZonesCore, ZonesEvent};
use crate::geometry::{work_area, Display, Edge, ReservedArea, ZoneGeometry};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::{Item, ItemKind};
use crate::placement::{Cell, LoosePosition, Placements, Whereabouts};
use crate::profile::{Profile, ProfileKey};
use crate::rules::{self, Decided, Scope};
use crate::snap::{self, Edges};
use crate::state::ZonesState;
use crate::store::{self, LoadReport, Store};
use gevrek_model::geometry::Rect;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

/// How often the runs came by the things they are there to look at, so
/// that a run which passes by never getting there is noticed.
#[derive(Default)]
struct Tally {
    planned: AtomicUsize,
    held: AtomicUsize,
    files_moved: AtomicUsize,
    taken_back: AtomicUsize,
    commands_done: AtomicUsize,
    commands_refused: AtomicUsize,
    zones_sent_over: AtomicUsize,
    returns_looked_at: AtomicUsize,
    read_from_disk: AtomicUsize,
    tabs_seen: AtomicUsize,
}

impl Tally {
    const fn new() -> Self {
        Self {
            planned: AtomicUsize::new(0),
            held: AtomicUsize::new(0),
            files_moved: AtomicUsize::new(0),
            taken_back: AtomicUsize::new(0),
            commands_done: AtomicUsize::new(0),
            commands_refused: AtomicUsize::new(0),
            zones_sent_over: AtomicUsize::new(0),
            returns_looked_at: AtomicUsize::new(0),
            read_from_disk: AtomicUsize::new(0),
            tabs_seen: AtomicUsize::new(0),
        }
    }
}

pub(crate) fn count(counter: &AtomicUsize, by: usize) {
    counter.fetch_add(by, Ordering::Relaxed);
}

pub(crate) fn counted(counter: &AtomicUsize) -> usize {
    counter.load(Ordering::Relaxed)
}

static RULES: Tally = Tally::new();
static LAYOUT: Tally = Tally::new();

pub(crate) const START: u64 = 1_800_000_000;
const DAY: u64 = 24 * 60 * 60;

/// Numbers that look like chance and are the same in every run.
pub(crate) struct Chance(pub(crate) u64);

impl Chance {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, count: usize) -> usize {
        (self.next() % count.max(1) as u64) as usize
    }

    /// True this many times in a hundred.
    pub(crate) fn one_in_a_hundred(&mut self, times: usize) -> bool {
        self.below(100) < times
    }

    pub(crate) fn between(&mut self, least: i32, most: i32) -> i32 {
        least + self.below((most - least + 1).max(1) as usize) as i32
    }

    pub(crate) fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
        &of[self.below(of.len())]
    }
}

/// How many times as many runs as usual, and the one seed asked for.
pub(crate) fn asked() -> (u64, Option<u64>) {
    let times = std::env::var("GEVREK_RUNS")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1);
    let seed = std::env::var("GEVREK_SEED")
        .ok()
        .and_then(|text| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok());
    (times, seed)
}

/// Run `run` with many seeds and say which went wrong.
pub(crate) fn many(rounds: u64, run: fn(u64) -> Result<(), String>) {
    let (times, seed) = asked();
    if let Some(seed) = seed {
        run(seed).expect("the run of that seed");
        return;
    }
    let mut wrong: Vec<String> = Vec::new();
    for round in 0..rounds * times {
        let seed = (round + 1)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(times.wrapping_mul(0x2545_f491_4f6c_dd1d));
        match std::panic::catch_unwind(move || run(seed)) {
            Ok(Ok(())) => {}
            Ok(Err(what)) => wrong.push(format!("seed {seed:#x}: {what}")),
            Err(panic) => wrong.push(format!(
                "seed {seed:#x}: the program ended: {}",
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                    .unwrap_or_default()
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of the runs went wrong; the first:\n{}",
        wrong.len(),
        wrong
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

const EXTENSIONS: [&str; 7] = ["png", "jpg", "doc", "txt", "pdf", "lnk", ""];
const WORDS: [&str; 8] = [
    "report",
    "photo",
    "notes",
    "Screenshot",
    "budget",
    "setup",
    "old",
    "x",
];

/// An item as chance makes it: some facts known, some not.
fn item(chance: &mut Chance, number: usize, now: u64) -> Item {
    let extension = *chance.pick(&EXTENSIONS);
    let dot = if extension.is_empty() { "" } else { "." };
    let name = format!("{} {number}{dot}{extension}", chance.pick(&WORDS));
    let mut item = Item::file(&format!("/desk/{name}"));
    let time = |chance: &mut Chance| {
        chance.one_in_a_hundred(80).then(|| {
            now.saturating_sub(chance.below(400) as u64 * DAY + chance.below(DAY as usize) as u64)
        })
    };
    item.facts.created = time(chance);
    item.facts.modified = time(chance);
    item.facts.accessed = time(chance);
    item.facts.size = chance
        .one_in_a_hundred(85)
        .then(|| chance.below(5_000_000) as u64);
    item.facts.mime = match extension {
        "png" | "jpg" if chance.one_in_a_hundred(90) => Some(format!("image/{extension}")),
        "txt" => Some("text/plain".to_string()),
        _ => None,
    };
    match chance.below(12) {
        0 => {
            item.kind = ItemKind::Shortcut;
            item.facts.shortcut_target = Some(format!("C:\\Programs\\{}.exe", chance.pick(&WORDS)));
        }
        1 => {
            // No file: This PC, the bin.
            item.kind = ItemKind::Virtual;
            item.path = None;
            item.id = ItemId::new(format!("::virtual-{number}"));
        }
        2 => item.kind = ItemKind::Folder,
        _ => {}
    }
    item
}

fn matcher(chance: &mut Chance, now: u64) -> Matcher {
    match chance.below(9) {
        0 | 1 => Matcher::Extension {
            any_of: (0..chance.between(1, 3))
                .map(|_| chance.pick(&EXTENSIONS).to_string())
                .collect(),
        },
        2 => Matcher::Mime {
            pattern: chance
                .pick(&["image/*", "text/*", "*/png", "*"])
                .to_string(),
        },
        3 => Matcher::Name {
            pattern: match chance.below(3) {
                0 => format!("{}*", chance.pick(&WORDS)),
                1 => format!("*{}*", chance.between(0, 9)),
                _ => "*".to_string(),
            },
        },
        4 => Matcher::Kind {
            any_of: vec![*chance.pick(&[
                ItemKind::File,
                ItemKind::Folder,
                ItemKind::Shortcut,
                ItemKind::Virtual,
            ])],
        },
        5 => Matcher::Date {
            field: *chance.pick(&[DateField::Created, DateField::Modified, DateField::Accessed]),
            after: chance
                .one_in_a_hundred(30)
                .then(|| now.saturating_sub(chance.below(300) as u64 * DAY)),
            before: chance
                .one_in_a_hundred(30)
                .then(|| now.saturating_sub(chance.below(300) as u64 * DAY)),
            newer_than_days: chance
                .one_in_a_hundred(40)
                .then(|| chance.below(300) as u32),
            older_than_days: chance
                .one_in_a_hundred(40)
                .then(|| chance.below(300) as u32),
        },
        6 => Matcher::Size {
            at_least: chance
                .one_in_a_hundred(60)
                .then(|| chance.below(3_000_000) as u64),
            at_most: chance
                .one_in_a_hundred(60)
                .then(|| chance.below(5_000_000) as u64),
        },
        7 => Matcher::ShortcutTarget {
            pattern: format!("*{}*", chance.pick(&WORDS)),
        },
        _ => Matcher::Untouched {
            days: chance.below(400) as u32,
        },
    }
}

fn action(chance: &mut Chance, zones: &[&str]) -> RuleAction {
    if chance.one_in_a_hundred(25) {
        RuleAction::MoveToFolder {
            folder: chance
                .pick(&["/archive", "/desk", "/old/papers"])
                .to_string(),
        }
    } else {
        RuleAction::AssignToZone {
            zone: ZoneId::new(*chance.pick(zones)),
        }
    }
}

fn rule_list(chance: &mut Chance, zones: &[&str], now: u64) -> Vec<Rule> {
    (0..chance.below(7))
        .map(|number| Rule {
            name: format!("rule {number}"),
            enabled: !chance.one_in_a_hundred(10),
            matchers: (0..chance.below(4)).map(|_| matcher(chance, now)).collect(),
            action: action(chance, zones),
            ..Rule::default()
        })
        .collect()
}

/// One run of the rules.
fn rules_run(seed: u64) -> Result<(), String> {
    let mut chance = Chance(seed | 1);
    let mut now = START;
    // Five zones filled by hand, one that fills itself, one that is not there.
    let names = ["z0", "z1", "z2", "z3", "z4", "portal", "nowhere"];
    let mut config = ZonesConfig {
        zones: names[..5]
            .iter()
            .map(|name| ZoneDefinition::manual(name, name))
            .collect(),
        ..ZonesConfig::default()
    };
    config.zones.push(ZoneDefinition {
        kind: ZoneKind::Portal {
            directory: "/downloads".to_string(),
        },
        ..ZoneDefinition::manual("portal", "portal")
    });
    let mut items: Vec<Item> = Vec::new();
    let mut made = 0;
    let mut placements = Placements::default();

    for round in 0..160 {
        // The world moves on.
        for _ in 0..chance.below(4) {
            match chance.below(8) {
                0..=2 => {
                    made += 1;
                    items.push(item(&mut chance, made, now));
                }
                3 if !items.is_empty() => {
                    let gone = items.remove(chance.below(items.len()));
                    placements.take(&gone.id);
                }
                4 if !items.is_empty() => {
                    let id = chance.pick(&items).id.clone();
                    let zone = ZoneId::new(*chance.pick(&names));
                    let cell = chance.one_in_a_hundred(20).then(|| Cell {
                        column: chance.below(4) as u32,
                        row: chance.below(4) as u32,
                    });
                    placements.assign(&id, &zone, Some(chance.below(6)), cell);
                }
                5 if !items.is_empty() => {
                    let id = chance.pick(&items).id.clone();
                    let position = chance.one_in_a_hundred(50).then(|| LoosePosition {
                        display: DisplayId::new("d"),
                        x: chance.between(0, 1900),
                        y: chance.between(0, 1000),
                    });
                    placements.loosen(&id, position);
                }
                6 => {
                    let zone = chance.below(config.zones.len());
                    config.zones[zone].lock.members = chance.one_in_a_hundred(30);
                }
                _ => now += chance.below(60) as u64 * DAY,
            }
        }
        if round == 0 || chance.one_in_a_hundred(25) {
            config.rules = rule_list(&mut chance, &names, now);
            config.catch_all = chance
                .one_in_a_hundred(30)
                .then(|| action(&mut chance, &names));
        }
        let scope = if chance.one_in_a_hundred(50) {
            Scope::Loose
        } else {
            Scope::All
        };
        let wrong = |what: String| Err(format!("round {round}, {scope:?}: {what}"));
        let before = placements.clone();

        // A dry run changes nothing and says the same twice.
        let plan = rules::plan(&config, &items, &placements, scope, now);
        if placements != before {
            return wrong("the dry run changed the placements".to_string());
        }
        if plan != rules::plan(&config, &items, &placements, scope, now) {
            return wrong("two dry runs of the same differ".to_string());
        }
        // Each step was decided by the first rule that matches, looked
        // for here once more, the plain way.
        for step in &plan.steps {
            let Some(item) = items.iter().find(|item| item.id == step.item) else {
                return wrong(format!("{} is planned and not there", step.item));
            };
            let first = config.rules.iter().position(|rule| {
                rule.enabled
                    && !rule.matchers.is_empty()
                    && rule
                        .matchers
                        .iter()
                        .all(|matcher| rules::holds(matcher, item, now))
            });
            let expected = match first {
                Some(index) => (Decided::Rule(index), Some(&config.rules[index].action)),
                None => (Decided::CatchAll, config.catch_all.as_ref()),
            };
            if (step.decided, Some(&step.action)) != expected {
                return wrong(format!(
                    "{} was decided by {:?}, not by {:?}",
                    step.item, step.decided, expected.0
                ));
            }
            if scope == Scope::Loose
                && placements
                    .zone_of(&step.item)
                    .is_some_and(|zone| config.zone(zone).is_some())
            {
                return wrong(format!("{} is in a zone and was planned for", step.item));
            }
        }
        let planned: BTreeSet<&ItemId> = plan.steps.iter().map(|step| &step.item).collect();
        if planned.len() != plan.steps.len() {
            return wrong("an item is planned for twice".to_string());
        }
        count(&RULES.planned, plan.steps.len());
        count(&RULES.held, plan.held.len());
        let text = serde_json::to_string(&plan).map_err(|error| error.to_string())?;
        if serde_json::from_str::<rules::Plan>(&text).ok().as_ref() != Some(&plan) {
            return wrong(format!("the plan does not come back from its JSON: {text}"));
        }

        // Applying does what the dry run said, and nothing else.
        let path_of = |id: &ItemId| {
            items
                .iter()
                .find(|item| &item.id == id)
                .and_then(|item| item.path.clone())
        };
        let applied = rules::apply(&plan, &path_of, &mut placements, now);
        if let Some(broken) = placements.broken() {
            return wrong(format!("after applying: {broken}"));
        }
        let mut moves = 0;
        for step in &plan.steps {
            match &step.action {
                RuleAction::AssignToZone { zone } => {
                    if placements.zone_of(&step.item) != Some(zone) {
                        return wrong(format!("{} is not in {zone} after applying", step.item));
                    }
                    if config
                        .zone(zone)
                        .is_none_or(|zone| zone.lock.members || zone.kind != ZoneKind::Manual)
                    {
                        return wrong(format!("{zone} took an item and may not"));
                    }
                }
                RuleAction::MoveToFolder { folder } => {
                    moves += 1;
                    let from = path_of(&step.item).unwrap_or_default();
                    let operation = FileOperation::Move {
                        from,
                        to_folder: folder.clone(),
                    };
                    if !applied.operations.contains(&operation) {
                        return wrong(format!("{operation:?} was planned and not asked for"));
                    }
                    if placements.whereabouts(&step.item) != (Whereabouts::Loose { position: None })
                    {
                        return wrong(format!("{} is moved away and still shown", step.item));
                    }
                }
            }
        }
        if applied.operations.len() != moves || applied.undo.entries.len() != plan.steps.len() {
            return wrong("what was done is not what was planned".to_string());
        }
        for item in items.iter().filter(|item| !planned.contains(&item.id)) {
            let was = before.whereabouts(&item.id);
            let is = placements.whereabouts(&item.id);
            let same = match (&was, &is) {
                // (Its place in the order may be another: items before it left.)
                (
                    Whereabouts::Zone {
                        zone: a,
                        cell: cell_a,
                        ..
                    },
                    Whereabouts::Zone {
                        zone: b,
                        cell: cell_b,
                        ..
                    },
                ) => a == b && cell_a == cell_b,
                _ => was == is,
            };
            if !same {
                return wrong(format!(
                    "{} was not planned for and went from {was:?} to {is:?}",
                    item.id
                ));
            }
        }
        // The files that are moved away are gone from the desktop; for
        // what is left there is nothing more to do.
        let moved: BTreeSet<&ItemId> = applied.undo.moved.iter().map(|moved| &moved.item).collect();
        let left: Vec<Item> = items
            .iter()
            .filter(|item| !moved.contains(&item.id))
            .cloned()
            .collect();
        let again = rules::plan(&config, &left, &placements, scope, now);
        if !again.steps.is_empty() {
            return wrong(format!(
                "after applying there is more to do: {:?}",
                again.steps
            ));
        }

        // Taking back gives exactly what was.
        let record: rules::UndoRecord = serde_json::to_string(&applied.undo)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or("the undo record does not come back from its JSON")?;
        if record != applied.undo {
            return wrong("the undo record is another after its JSON".to_string());
        }
        let kept = placements.clone();
        let back = rules::undo(&record, &mut placements);
        if placements != before {
            return wrong(format!(
                "taking back gave {placements:?}, and before was {before:?}"
            ));
        }
        let expected: Vec<FileOperation> = applied
            .operations
            .iter()
            .rev()
            .filter_map(FileOperation::reversed)
            .collect();
        if back != expected {
            return wrong(format!("the files come back by {back:?}, not {expected:?}"));
        }
        count(&RULES.files_moved, back.len());
        count(&RULES.taken_back, record.entries.len());
        // Half of the time it stays applied, and the run goes on from there.
        if chance.one_in_a_hundred(50) {
            placements = kept;
            items = left;
        }
    }
    Ok(())
}

#[test]
fn long_runs_of_rules_plan_apply_and_take_back_alike() {
    many(48, rules_run);
    if asked().1.is_none() {
        let seen = [
            &RULES.planned,
            &RULES.held,
            &RULES.files_moved,
            &RULES.taken_back,
        ]
        .map(counted);
        assert!(
            seen.iter().all(|times| *times > 500),
            "the runs saw too little: {seen:?}"
        );
    }
}

/// The displays there are to plug in, by what they call themselves.
const DISPLAYS: [&str; 4] = ["edid:DEL-A1", "edid:SAM-B2", "edid:LEN-C3", "edid:BNQ-D4"];
const SIZES: [(i32, i32); 7] = [
    (1920, 1080),
    (2560, 1440),
    (1280, 720),
    (3840, 2160),
    (1024, 768),
    (800, 600),
    (1080, 1920),
];
const SCALES: [f64; 4] = [1.0, 1.25, 1.5, 2.0];

/// Put the displays side by side in the order they are in, as a system
/// would after one of them changed.
fn side_by_side(displays: &mut [Display]) {
    let mut x = displays
        .first()
        .map_or(0, |display| display.rect.x.clamp(-4000, 0));
    for display in displays {
        display.rect.x = x;
        x += display.rect.width;
    }
}

fn a_display(chance: &mut Chance, id: &str) -> Display {
    let (width, height) = *chance.pick(&SIZES);
    Display {
        id: DisplayId::new(id),
        rect: Rect::new(
            chance.between(-3000, 0),
            chance.between(-600, 600),
            width,
            height,
        ),
        scale: *chance.pick(&SCALES),
        primary: false,
    }
}

/// Where a zone was seen on a set of displays: which zone's place it
/// had, on which display, and where from that display's corner.
type Seen = BTreeMap<ZoneId, (ZoneId, DisplayId, i32, i32, i32)>;

/// What is outside the core in a run of the layout.
struct World {
    displays: Vec<Display>,
    reserved: Vec<ReservedArea>,
    workspace: Option<String>,
    items: Vec<Item>,
    items_made: usize,
    /// Sets of displays that were there before, to come back to.
    earlier: Vec<Vec<Display>>,
    /// The zones there must be.
    zones: BTreeSet<ZoneId>,
    /// The zones there were when each snapshot was taken.
    snapshots: BTreeMap<String, BTreeSet<ZoneId>>,
    /// What was seen on each set of displays when it was last in use, and
    /// the room that was kept then.
    seen: BTreeMap<ProfileKey, (Vec<ReservedArea>, Seen)>,
}

impl World {
    fn key(&self, core: &ZonesCore) -> ProfileKey {
        let workspace = if core.config().profile_per_workspace {
            self.workspace.as_deref()
        } else {
            None
        };
        ProfileKey::of(&self.displays, workspace)
    }

    fn displays_event(&self) -> BackendEvent {
        BackendEvent::Displays {
            displays: self.displays.clone(),
            reserved: self.reserved.clone(),
        }
    }

    fn a_zone(&self, chance: &mut Chance) -> ZoneId {
        let zones: Vec<&ZoneId> = self.zones.iter().collect();
        if zones.is_empty() || chance.one_in_a_hundred(4) {
            ZoneId::new("no-such-zone")
        } else {
            (*chance.pick(&zones)).clone()
        }
    }

    fn an_item(&self, chance: &mut Chance) -> ItemId {
        if self.items.is_empty() || chance.one_in_a_hundred(4) {
            ItemId::new("/desk/no-such-item")
        } else {
            chance.pick(&self.items).id.clone()
        }
    }

    /// What is seen of the zones now.
    fn look(&self, core: &ZonesCore) -> Seen {
        let leaders: BTreeMap<String, ZoneId> = {
            let mut first = BTreeMap::new();
            for group in crate::layout::tab_groups(&core.config().zones) {
                first.insert(group.name.clone(), group.members[0].clone());
            }
            first
        };
        core.config()
            .zones
            .iter()
            .filter_map(|zone| {
                let leader = zone
                    .tab_group
                    .as_ref()
                    .and_then(|group| leaders.get(group))
                    .unwrap_or(&zone.id)
                    .clone();
                let geometry = core.profile()?.geometry.get(&leader)?;
                let display = self
                    .displays
                    .iter()
                    .find(|display| display.id == geometry.display)?;
                let rect = core.zone_rect(&zone.id)?;
                Some((
                    zone.id.clone(),
                    (
                        leader,
                        display.id.clone(),
                        rect.x - display.rect.x,
                        rect.y - display.rect.y,
                        rect.width,
                    ),
                ))
            })
            .collect()
    }
}

/// What a step was, for telling which things may have changed in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The displays, the room kept on them, or the workspace.
    Outside,
    /// Something the person asked for.
    Command,
    /// A snapshot was put back: every arrangement is another.
    Restored,
}

/// One step of chance. Says what it was.
fn step(
    chance: &mut Chance,
    core: &mut ZonesCore,
    world: &mut World,
    now: u64,
) -> (Kind, String, Vec<Effect>) {
    let outside = |world: &mut World, core: &mut ZonesCore, what: String| {
        side_by_side(&mut world.displays);
        world.earlier.push(world.displays.clone());
        if world.earlier.len() > 12 {
            world.earlier.remove(0);
        }
        let effects = core.event(world.displays_event(), now);
        (Kind::Outside, what, effects)
    };
    match chance.below(40) {
        0 | 1 => {
            let absent: Vec<&str> = DISPLAYS
                .iter()
                .filter(|id| {
                    !world
                        .displays
                        .iter()
                        .any(|display| display.id.as_str() == **id)
                })
                .copied()
                .collect();
            let Some(id) = absent.first().map(|_| *chance.pick(&absent)) else {
                return (
                    Kind::Outside,
                    "no display left to add".to_string(),
                    Vec::new(),
                );
            };
            let at = chance.below(world.displays.len() + 1);
            world.displays.insert(at, a_display(chance, id));
            outside(world, core, format!("{id} comes"))
        }
        2 | 3 => {
            if world.displays.len() == 1 {
                // The last one goes (the machine sleeps): told, and
                // nothing may change.
                let before = core.state().clone();
                let effects = core.event(
                    BackendEvent::Displays {
                        displays: Vec::new(),
                        reserved: Vec::new(),
                    },
                    now,
                );
                assert_eq!(core.state(), &before, "no display, and the state changed");
                return (Kind::Outside, "every display goes".to_string(), effects);
            }
            let gone = world.displays.remove(chance.below(world.displays.len()));
            outside(world, core, format!("{} goes", gone.id))
        }
        4 => {
            // Another order, and with it other places.
            for at in (1..world.displays.len()).rev() {
                world.displays.swap(at, chance.below(at + 1));
            }
            world.displays[0].rect.x = chance.between(-3000, 0);
            outside(world, core, "the displays change places".to_string())
        }
        5 => {
            let at = chance.below(world.displays.len());
            let (width, height) = *chance.pick(&SIZES);
            world.displays[at].rect.width = width;
            world.displays[at].rect.height = height;
            outside(
                world,
                core,
                format!("{} is {width} by {height}", world.displays[at].id),
            )
        }
        6 => {
            let at = chance.below(world.displays.len());
            world.displays[at].scale = *chance.pick(&SCALES);
            outside(
                world,
                core,
                format!(
                    "{} is scaled {}",
                    world.displays[at].id, world.displays[at].scale
                ),
            )
        }
        7..=9 => {
            // A set that was there before is there again.
            let Some(earlier) =
                (!world.earlier.is_empty()).then(|| chance.pick(&world.earlier).clone())
            else {
                return (
                    Kind::Outside,
                    "nothing earlier to return to".to_string(),
                    Vec::new(),
                );
            };
            world.displays = earlier;
            outside(
                world,
                core,
                "an earlier set of displays returns".to_string(),
            )
        }
        10 => {
            let mut reserved = Vec::new();
            for display in &world.displays {
                if chance.one_in_a_hundred(60) {
                    reserved.push(ReservedArea {
                        display: display.id.clone(),
                        edge: *chance.pick(&[Edge::Left, Edge::Top, Edge::Right, Edge::Bottom]),
                        thickness: chance.between(0, 80),
                    });
                }
            }
            world.reserved = reserved;
            let effects = core.event(world.displays_event(), now);
            (
                Kind::Outside,
                "the room panels keep changes".to_string(),
                effects,
            )
        }
        11 => {
            let primary = chance.below(world.displays.len() + 1);
            for (at, display) in world.displays.iter_mut().enumerate() {
                display.primary = at == primary;
            }
            let effects = core.event(world.displays_event(), now);
            (
                Kind::Outside,
                "another display is the primary one".to_string(),
                effects,
            )
        }
        12 | 13 => {
            world.workspace = Some(chance.between(1, 3).to_string());
            let effects = core.event(
                BackendEvent::Workspace {
                    name: world.workspace.clone(),
                },
                now,
            );
            (
                Kind::Outside,
                format!("workspace {:?}", world.workspace),
                effects,
            )
        }
        14 | 15 => {
            world.items_made += 1;
            let item = item(chance, world.items_made, now);
            world.items.push(item.clone());
            let effects = core.event(
                BackendEvent::ItemAdded {
                    location: Location::Desktop,
                    item,
                },
                now,
            );
            (Kind::Command, "an item comes".to_string(), effects)
        }
        16 if !world.items.is_empty() => {
            let gone = world.items.remove(chance.below(world.items.len()));
            let effects = core.event(
                BackendEvent::ItemRemoved {
                    location: Location::Desktop,
                    item: gone.id,
                },
                now,
            );
            (Kind::Command, "an item goes".to_string(), effects)
        }
        17 if !world.items.is_empty() => {
            let at = chance.below(world.items.len());
            let from = world.items[at].id.clone();
            world.items_made += 1;
            let mut renamed = Item::file(&format!("/desk/renamed {}.txt", world.items_made));
            renamed.facts.modified = world.items[at].facts.modified;
            world.items[at] = renamed.clone();
            let event = BackendEvent::ItemRenamed {
                location: Location::Desktop,
                from,
                item: renamed,
            };
            (
                Kind::Command,
                "an item is renamed".to_string(),
                core.event(event, now),
            )
        }
        other => {
            let zone = world.a_zone(chance);
            let command = match other {
                18..=20 => Command::CreateZone {
                    title: format!("Zone {}", chance.below(100)),
                    kind: match chance.below(8) {
                        0 => ZoneKind::Portal {
                            directory: format!("/folder-{}", chance.below(3)),
                        },
                        1 => ZoneKind::Smart {
                            query: vec![matcher(chance, now)],
                        },
                        _ => ZoneKind::Manual,
                    },
                    rect: chance.one_in_a_hundred(70).then(|| {
                        Rect::new(
                            chance.between(-4000, 9000),
                            chance.between(-1000, 3000),
                            chance.between(-10, 5000),
                            chance.between(-10, 3000),
                        )
                    }),
                },
                21 => Command::DeleteZone { zone },
                22..=25 => Command::MoveZone {
                    zone,
                    x: chance.between(-5000, 10_000),
                    y: chance.between(-2000, 4000),
                    snap: chance.one_in_a_hundred(70),
                },
                26 | 27 => Command::ResizeZone {
                    zone,
                    rect: Rect::new(
                        chance.between(-5000, 10_000),
                        chance.between(-2000, 4000),
                        chance.between(-100, 6000),
                        chance.between(-100, 4000),
                    ),
                    edges: Edges {
                        left: chance.one_in_a_hundred(40),
                        top: chance.one_in_a_hundred(40),
                        right: chance.one_in_a_hundred(40),
                        bottom: chance.one_in_a_hundred(40),
                    },
                    snap: chance.one_in_a_hundred(70),
                },
                28 | 29 => Command::AddItem {
                    zone,
                    item: world.an_item(chance),
                    index: chance.one_in_a_hundred(50).then(|| chance.below(8)),
                    cell: chance.one_in_a_hundred(30).then(|| Cell {
                        column: chance.below(6) as u32,
                        row: chance.below(6) as u32,
                    }),
                },
                30 => Command::RemoveItem {
                    item: world.an_item(chance),
                },
                31 => Command::PlaceLooseItem {
                    item: world.an_item(chance),
                    display: DisplayId::new(*chance.pick(&DISPLAYS)),
                    x: chance.between(-100, 4000),
                    y: chance.between(-100, 2500),
                },
                32 => Command::RollUp {
                    zone,
                    rolled_up: chance
                        .one_in_a_hundred(50)
                        .then(|| chance.one_in_a_hundred(50)),
                },
                33 => Command::Lock {
                    zone,
                    geometry: chance
                        .one_in_a_hundred(50)
                        .then(|| chance.one_in_a_hundred(30)),
                    members: chance
                        .one_in_a_hundred(50)
                        .then(|| chance.one_in_a_hundred(30)),
                },
                34 => Command::Tab {
                    zone,
                    with: world.a_zone(chance),
                },
                35 => {
                    if chance.one_in_a_hundred(50) {
                        Command::Untab { zone }
                    } else {
                        Command::ActivateTab { zone }
                    }
                }
                36 => match chance.below(4) {
                    0 => Command::ShowZone {
                        zone,
                        shown: chance.one_in_a_hundred(50),
                    },
                    1 => Command::QuickHide { hidden: None },
                    2 => Command::Peek {
                        on: chance.one_in_a_hundred(50),
                    },
                    _ => Command::SetView {
                        zone,
                        view: Some(*chance.pick(&[View::Grid, View::List])),
                        sort: Some(*chance.pick(&[
                            Sort::Manual,
                            Sort::Name,
                            Sort::Type,
                            Sort::Date,
                            Sort::Size,
                        ])),
                        descending: Some(chance.one_in_a_hundred(50)),
                        icon_size: Some(chance.below(400) as u32),
                    },
                },
                37 => {
                    if chance.one_in_a_hundred(60) {
                        Command::ApplyRules {
                            scope: *chance.pick(&[Scope::Loose, Scope::All]),
                        }
                    } else {
                        Command::UndoRules
                    }
                }
                38 => Command::SaveSnapshot {
                    name: format!("s{}", chance.below(3)),
                },
                _ => Command::RestoreSnapshot {
                    name: format!("s{}", chance.below(3)),
                },
            };
            let what = format!("{command:?}");
            let effects = core.command(command.clone(), now);
            let refused = effects
                .iter()
                .any(|effect| matches!(effect, Effect::Notify(ZonesEvent::Refused { .. })));
            let mut kind = Kind::Command;
            count(
                if refused {
                    &LAYOUT.commands_refused
                } else {
                    &LAYOUT.commands_done
                },
                1,
            );
            if !refused {
                match &command {
                    Command::DeleteZone { zone } => {
                        world.zones.remove(zone);
                    }
                    Command::SaveSnapshot { name } => {
                        world.snapshots.insert(name.clone(), world.zones.clone());
                    }
                    Command::RestoreSnapshot { name } => {
                        world.zones = world.snapshots[name].clone();
                        world.seen.clear();
                        kind = Kind::Restored;
                    }
                    _ => {}
                }
            }
            for effect in &effects {
                if let Effect::Notify(ZonesEvent::ZoneCreated { zone }) = effect {
                    world.zones.insert(zone.clone());
                }
            }
            (kind, what, effects)
        }
    }
}

/// The places of the zones that are still there, in every arrangement.
fn places(
    profiles: &[Profile],
    config: &ZonesConfig,
) -> BTreeMap<ProfileKey, BTreeMap<ZoneId, ZoneGeometry>> {
    profiles
        .iter()
        .map(|profile| {
            let still: BTreeMap<ZoneId, ZoneGeometry> = profile
                .geometry
                .iter()
                .filter(|(zone, _)| config.zone(zone).is_some())
                .map(|(zone, geometry)| (zone.clone(), geometry.clone()))
                .collect();
            (profile.key.clone(), still)
        })
        .collect()
}

/// One run of the layout.
fn layout_run(seed: u64) -> Result<(), String> {
    let folder =
        std::env::temp_dir().join(format!("gevrek-zones-run-{}-{seed:x}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let outcome = layout_steps(seed, &Store::new(&folder).keeping(3));
    let _ = std::fs::remove_dir_all(&folder);
    outcome
}

fn layout_steps(seed: u64, store: &Store) -> Result<(), String> {
    let mut chance = Chance(seed | 1);
    let now = START;
    let config = ZonesConfig {
        enabled: true,
        profile_per_workspace: chance.one_in_a_hundred(40),
        rules: vec![
            Rule {
                name: "pictures".to_string(),
                matchers: vec![Matcher::Extension {
                    any_of: vec!["png".to_string(), "jpg".to_string()],
                }],
                action: RuleAction::AssignToZone {
                    zone: ZoneId::new("zone-1"),
                },
                ..Rule::default()
            },
            Rule {
                name: "papers".to_string(),
                matchers: vec![Matcher::Extension {
                    any_of: vec!["doc".to_string(), "pdf".to_string()],
                }],
                action: RuleAction::AssignToZone {
                    zone: ZoneId::new("zone-2"),
                },
                ..Rule::default()
            },
        ],
        catch_all: chance
            .one_in_a_hundred(30)
            .then(|| RuleAction::AssignToZone {
                zone: ZoneId::new("zone-3"),
            }),
        ..ZonesConfig::default()
    };
    let mut world = World {
        displays: vec![Display {
            primary: true,
            ..a_display(&mut chance, DISPLAYS[0])
        }],
        reserved: Vec::new(),
        workspace: None,
        items: Vec::new(),
        items_made: 0,
        earlier: Vec::new(),
        zones: BTreeSet::new(),
        snapshots: BTreeMap::new(),
        seen: BTreeMap::new(),
    };
    side_by_side(&mut world.displays);
    for _ in 0..10 {
        world.items_made += 1;
        world.items.push(item(&mut chance, world.items_made, now));
    }
    let mut core = ZonesCore::new(config, ZonesState::default());
    core.event(world.displays_event(), now);
    core.event(
        BackendEvent::Listed {
            location: Location::Desktop,
            items: world.items.clone(),
        },
        now,
    );

    for number in 0..260 {
        let before = places(&core.state().profiles, core.config());
        let key_before = world.key(&core);
        let (kind, what, effects) = step(&mut chance, &mut core, &mut world, now);
        for effect in &effects {
            if let Effect::Notify(ZonesEvent::ProfileChanged { moved, .. }) = effect {
                count(&LAYOUT.zones_sent_over, moved.len());
            }
        }
        let wrong = |problem: String| Err(format!("step {number} ({what}): {problem}"));

        // Whole: no item in two places, every zone at a place that can be
        // reached on a display that is there.
        if let Some(broken) = core.broken() {
            return wrong(broken);
        }
        // No zone is lost, and none is there that should not be.
        let there: BTreeSet<ZoneId> = core
            .config()
            .zones
            .iter()
            .map(|zone| zone.id.clone())
            .collect();
        if there != world.zones {
            return wrong(format!(
                "the zones are {there:?} and should be {:?}",
                world.zones
            ));
        }
        let key = world.key(&core);
        if core.profile().map(|profile| &profile.key) != Some(&key) {
            return wrong("the arrangement in use is not the one for these displays".to_string());
        }
        for zone in &there {
            let Some(rect) = core.zone_rect(zone) else {
                return wrong(format!("{zone} has no rectangle"));
            };
            let on_one = world
                .displays
                .iter()
                .any(|display| snap::within(rect, work_area(display, &world.reserved)));
            if !on_one {
                return wrong(format!(
                    "{zone} at {rect:?} is not wholly on a display's free room"
                ));
            }
        }
        // What is not in use is not touched. A step from outside may make
        // an arrangement and give zones a place in it that had none; it
        // moves nothing that has one. A command changes the arrangement
        // in use and no other.
        if kind != Kind::Restored {
            let after = places(&core.state().profiles, core.config());
            for (profile, zones) in &before {
                if kind == Kind::Command && *profile == key_before {
                    continue;
                }
                let Some(now_there) = after.get(profile) else {
                    // (Only the one used longest ago, and only for there
                    // being too many.)
                    if after.len() == crate::profile::MOST && before.len() == crate::profile::MOST {
                        world.seen.remove(profile);
                        continue;
                    }
                    return wrong(format!("the arrangement for {profile:?} is gone"));
                };
                for (zone, geometry) in zones {
                    if core.config().zone(zone).is_some() && now_there.get(zone) != Some(geometry) {
                        return wrong(format!(
                            "{zone} was at {geometry:?} for {profile:?} and is at {:?}",
                            now_there.get(zone)
                        ));
                    }
                }
            }
        }
        // A set of displays that is back shows its zones where they were
        // seen on it last (those that still hold the place they held, with
        // the same room kept by panels).
        let look = world.look(&core);
        if kind == Kind::Outside && key != key_before {
            if let Some((reserved, seen)) = world.seen.get(&key) {
                for (zone, was) in seen.iter().filter(|_| *reserved == world.reserved) {
                    if let Some(is) = look.get(zone).filter(|is| is.0 == was.0) {
                        count(&LAYOUT.returns_looked_at, 1);
                        if is != was {
                            return wrong(format!(
                                "{zone} was seen at {was:?} on these displays and is at {is:?}"
                            ));
                        }
                    }
                }
            }
        }
        count(
            &LAYOUT.tabs_seen,
            look.iter().filter(|(zone, seen)| **zone != seen.0).count(),
        );
        world.seen.insert(key, (world.reserved.clone(), look));

        // The state comes back from its text as it is, and (now and
        // then, for the disk is slow) from its file.
        let text = store::encode(core.state());
        let read = match store::decode(&text) {
            Ok((state, None)) => state,
            other => {
                return wrong(format!(
                    "the state does not come back from its text: {other:?}"
                ))
            }
        };
        if &read != core.state() {
            return wrong("the state is another after its text".to_string());
        }
        if number % 32 == 0 {
            store
                .save(core.state())
                .map_err(|error| error.to_string())?;
            let loaded = store.load().map_err(|error| error.to_string())?;
            if loaded.report != LoadReport::Read || &loaded.state != core.state() {
                return wrong(format!(
                    "the state does not come back from its file: {:?}",
                    loaded.report
                ));
            }
            count(&LAYOUT.read_from_disk, 1);
        }
        let config: ZonesConfig = serde_json::to_string(core.config())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or("the configuration does not come back from its JSON")?;
        if &config != core.config() {
            return wrong("the configuration is another after its JSON".to_string());
        }
        // And a program started anew with what was written shows the same.
        let mut again = ZonesCore::new(config, read);
        again.event(
            BackendEvent::Workspace {
                name: world.workspace.clone(),
            },
            now,
        );
        again.event(world.displays_event(), now);
        again.event(
            BackendEvent::Listed {
                location: Location::Desktop,
                items: world.items.clone(),
            },
            now,
        );
        let scene = core.scene(now);
        again.command(
            Command::QuickHide {
                hidden: Some(scene.quick_hidden),
            },
            now,
        );
        again.command(Command::Peek { on: scene.peeking }, now);
        if again.scene(now) != scene {
            return wrong("started anew, the zones are shown otherwise".to_string());
        }
    }
    Ok(())
}

#[test]
fn long_runs_of_display_changes_leave_every_zone_where_it_belongs() {
    many(24, layout_run);
    if asked().1.is_none() {
        let seen = [
            &LAYOUT.commands_done,
            &LAYOUT.commands_refused,
            &LAYOUT.zones_sent_over,
            &LAYOUT.returns_looked_at,
            &LAYOUT.read_from_disk,
            &LAYOUT.tabs_seen,
        ]
        .map(counted);
        assert!(
            seen.iter().all(|times| *times > 150),
            "the runs saw too little: {seen:?}"
        );
    }
}
