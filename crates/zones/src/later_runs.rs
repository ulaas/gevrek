//! Long runs of what came with milestones 3 to 5, in an order nobody
//! thought of: zones rolled up and opened by a pointer that comes and
//! goes, tabs put in order and split off, the one key that puts things
//! away and what is said to stay, portals walking about in a tree of
//! folders that changes under them (and is slow, and goes away), zones
//! that fill themselves from a source that changes, rules on a schedule,
//! layouts carried out and in, the keyboard going everywhere, things
//! covered from view, and zones made by type and taken back. The core is
//! driven as the shell drives it: through the host, on the backend in
//! memory.
//!
//! After every step:
//! - nothing is broken (no item in two zones, every zone at a place that
//!   can be reached, nothing that is gone open, chosen or focused);
//! - no zone is lost by anything but being deleted, and whatever replaces
//!   the whole layout has kept the one there was as a snapshot;
//! - the keyboard is on a zone that is shown and an item it shows, or on
//!   nothing;
//! - a portal whose folder is listed shows exactly what the folder holds;
//! - nothing a zone shows by itself is written to the state;
//! - a dry run changes nothing and says what applying then does, and
//!   taking that back gives the placements there were;
//! - what is covered is not in the scene and is not found;
//! - what is so only for the moment asks for nothing to be saved;
//! - the room kept free of windows is what was last told;
//! - everything that is told survives its JSON, and a program started
//!   anew from what was written has the zones where they are;
//! - file operations that take long (they are begun, say how far they
//!   are, end, fail, are called off, and the program is started anew
//!   while some are under way): every one that was begun is under way
//!   until it ends or is called off and not after; the desktop the core
//!   knows is the desktop there is, no item more and none less; and a
//!   file brought in and taken back leaves the desktop as it was;
//! - zones made by type: asking what would be made changes nothing and
//!   says what making then does; asked again there is nothing left to
//!   do; and taking it back gives the zones, the rules and the
//!   placements there were.
//!
//! Seeds as in `random_runs`: `GEVREK_RUNS`, `GEVREK_SEED`.

use crate::backend::BackendError;
use crate::backend::BackendEvent;
use crate::config::{
    ExpandOn, Matcher, RollUpSettings, Rule, RuleAction, SmartSource, ZoneKind, ZonesConfig,
};
use crate::core::{Command, Effect, Scene, ZonesCore, ZonesEvent};
use crate::export;
use crate::focus::{Direction, SelectMode, Step, DESK};
use crate::geometry::Display;
use crate::host::Host;
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::ipc::ZonesSnapshot;
use crate::item::{Item, ItemKind};
use crate::portal::Listing;
use crate::random_runs::{asked, count, counted, many, Chance, START};
use crate::reserve::ReservedRegion;
use crate::rules::{self, Decided, Scope};
use crate::state::ZonesState;
use crate::store;
use crate::testing::fake_backend::{FakeBackend, DESKTOP};
use gevrek_model::geometry::Rect;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicUsize;

/// How often the runs came by what they are there to look at.
struct Tally {
    navigated: AtomicUsize,
    mirrored: AtomicUsize,
    opened_for_now: AtomicUsize,
    keyboard_moved: AtomicUsize,
    layouts_replaced: AtomicUsize,
    covered: AtomicUsize,
    dry_runs: AtomicUsize,
    timed: AtomicUsize,
    tabs: AtomicUsize,
    put_away: AtomicUsize,
    sources: AtomicUsize,
    not_saved: AtomicUsize,
    room_kept: AtomicUsize,
    unavailable: AtomicUsize,
    on_the_desk: AtomicUsize,
    begun: AtomicUsize,
    ended: AtomicUsize,
    called_off: AtomicUsize,
    failed_later: AtomicUsize,
    started_anew: AtomicUsize,
    taken_back: AtomicUsize,
    made_by_type: AtomicUsize,
    unmade: AtomicUsize,
    grey: AtomicUsize,
}

static SEEN: Tally = Tally {
    navigated: AtomicUsize::new(0),
    mirrored: AtomicUsize::new(0),
    opened_for_now: AtomicUsize::new(0),
    keyboard_moved: AtomicUsize::new(0),
    layouts_replaced: AtomicUsize::new(0),
    covered: AtomicUsize::new(0),
    dry_runs: AtomicUsize::new(0),
    timed: AtomicUsize::new(0),
    tabs: AtomicUsize::new(0),
    put_away: AtomicUsize::new(0),
    sources: AtomicUsize::new(0),
    not_saved: AtomicUsize::new(0),
    room_kept: AtomicUsize::new(0),
    unavailable: AtomicUsize::new(0),
    on_the_desk: AtomicUsize::new(0),
    begun: AtomicUsize::new(0),
    ended: AtomicUsize::new(0),
    called_off: AtomicUsize::new(0),
    failed_later: AtomicUsize::new(0),
    started_anew: AtomicUsize::new(0),
    taken_back: AtomicUsize::new(0),
    made_by_type: AtomicUsize::new(0),
    unmade: AtomicUsize::new(0),
    grey: AtomicUsize::new(0),
};

/// The folders portals walk about in.
const TREE: [&str; 4] = ["/tree", "/tree/a", "/tree/a/b", "/tree/c"];
const HOUR: u64 = 60 * 60;

/// What a step was, for telling what may have changed in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Something happened outside, or a command that may change anything
    /// but the set of zones.
    Change,
    /// Something that is so for the moment only: nothing is to be saved.
    View,
    /// A zone was to be deleted.
    Deletes,
    /// A whole layout was to be put in the place of the one there is.
    Replaces,
}

struct World {
    core: ZonesCore,
    host: Host,
    backend: FakeBackend,
    now: u64,
    at_ms: u64,
    made: usize,
    recent: Vec<Item>,
    /// Layouts that were carried out, to be carried in again.
    exported: Vec<serde_json::Value>,
    told_reserved: Vec<ReservedRegion>,
    second: bool,
    shared: bool,
    slow: bool,
    /// The operations that were begun and have not ended, by request.
    begun: BTreeSet<u64>,
}

impl World {
    /// Take note of what was said of operations and of the room kept.
    fn hear(&mut self, effects: &[Effect]) {
        for event in heard(effects) {
            match event {
                ZonesEvent::OperationBegun { request, .. } => {
                    self.begun.insert(*request);
                    count(&SEEN.begun, 1);
                }
                ZonesEvent::OperationEnded { request } => {
                    self.begun.remove(request);
                    count(&SEEN.ended, 1);
                }
                ZonesEvent::OperationFailed {
                    error: BackendError::Cancelled,
                    ..
                } => count(&SEEN.called_off, 1),
                ZonesEvent::ReservedChanged { regions } => self.told_reserved = regions.clone(),
                _ => {}
            }
        }
    }

    fn desktop(&self) -> BTreeSet<ItemId> {
        self.backend
            .in_folder(DESKTOP)
            .into_iter()
            .map(|item| item.id)
            .collect()
    }

    /// The program ends and is started again from what it wrote. What it
    /// had under way ends without it, or not at all.
    fn start_anew(&mut self, chance: &mut Chance) -> Result<Vec<Effect>, String> {
        let written = store::encode(self.core.state());
        let (state, _) = store::decode(&written).map_err(|error| format!("{error:?}"))?;
        let config = self.core.config().clone();
        self.host.stop(&mut self.backend);
        self.backend.abandon_operations(chance.one_in_a_hundred(50));
        self.backend.awaited.clear();
        self.begun.clear();
        self.told_reserved.clear();
        self.core = ZonesCore::new(config, state);
        self.host = Host::new();
        let effects = self.host.start(&mut self.core, &mut self.backend, self.now);
        count(&SEEN.started_anew, 1);
        Ok(effects)
    }

    /// A file is brought in and taken back, each as slowly as the backend
    /// is just now: the desktop is as it was.
    fn in_and_back(&mut self, chance: &mut Chance) -> Result<Vec<Effect>, String> {
        // (Only with nothing else under way: ending these ends everything.)
        if !self.backend.under_way.is_empty() {
            return Ok(Vec::new());
        }
        self.backend.fail_next = None;
        let before = self.desktop();
        let zone = chance.one_in_a_hundred(50).then(|| self.a_zone(chance));
        let zone = zone.filter(|zone| self.core.config().zone(zone).is_some());
        let mut effects = self.ask(Command::BringIn {
            paths: vec!["/outside/o1.txt".to_string()],
            folder: DESKTOP.to_string(),
            copy: true,
            zone,
            at: None,
        });
        self.backend.finish_operations();
        effects.extend(self.pump());
        if self.desktop().len() != before.len() + 1 {
            return Err("a file was brought in and is not on the desktop".to_string());
        }
        effects.extend(self.ask(Command::UndoFileOperation));
        self.backend.finish_operations();
        effects.extend(self.pump());
        if self.desktop() != before {
            return Err(format!(
                "brought in and taken back, the desktop is {:?}, not {before:?}",
                self.desktop()
            ));
        }
        count(&SEEN.taken_back, 1);
        Ok(effects)
    }

    fn ask(&mut self, command: Command) -> Vec<Effect> {
        self.host
            .command(&mut self.core, &mut self.backend, command, self.now)
    }

    fn pump(&mut self) -> Vec<Effect> {
        self.host.pump(&mut self.core, &mut self.backend, self.now)
    }

    fn zones(&self) -> Vec<ZoneId> {
        self.core
            .config()
            .zones
            .iter()
            .map(|zone| zone.id.clone())
            .collect()
    }

    fn a_zone(&self, chance: &mut Chance) -> ZoneId {
        let zones = self.zones();
        if zones.is_empty() || chance.one_in_a_hundred(3) {
            ZoneId::new("no-such-zone")
        } else {
            chance.pick(&zones).clone()
        }
    }

    fn portals(&self) -> Vec<ZoneId> {
        let zones = self.core.config().zones.iter();
        zones
            .filter(|zone| matches!(zone.kind, ZoneKind::Portal { .. }))
            .map(|zone| zone.id.clone())
            .collect()
    }

    fn a_portal(&self, chance: &mut Chance) -> ZoneId {
        let portals = self.portals();
        if portals.is_empty() || chance.one_in_a_hundred(5) {
            self.a_zone(chance)
        } else {
            chance.pick(&portals).clone()
        }
    }

    fn a_desktop_item(&self, chance: &mut Chance) -> ItemId {
        let items = self.backend.in_folder(DESKTOP);
        if items.is_empty() || chance.one_in_a_hundred(3) {
            ItemId::new("/desk/no-such-item")
        } else {
            chance.pick(&items).id.clone()
        }
    }

    /// An item of anywhere: mostly the desktop, sometimes what a portal
    /// or a zone that fills itself shows.
    fn an_item(&self, chance: &mut Chance) -> ItemId {
        if chance.one_in_a_hundred(60) {
            return self.a_desktop_item(chance);
        }
        let zone = self.a_zone(chance);
        let shown = self.core.zone_holds(&zone, self.now);
        if shown.is_empty() {
            self.a_desktop_item(chance)
        } else {
            chance.pick(&shown).id.clone()
        }
    }

    fn a_file(&mut self, folder: &str, chance: &mut Chance) -> Item {
        self.made += 1;
        let extension = *chance.pick(&["png", "txt", "pdf", "jpg"]);
        let mut item = Item::file(&crate::paths::join(
            folder,
            &format!("f{}.{extension}", self.made),
        ));
        item.facts.modified = Some(self.now.saturating_sub(chance.below(6) as u64 * 24 * HOUR));
        item
    }

    fn fresh_recent(&mut self, chance: &mut Chance) -> Vec<Item> {
        let count = chance.below(7);
        (0..count)
            .map(|_| {
                self.made += 1;
                Item::file(&format!("/recent/r{}.txt", self.made))
            })
            .collect()
    }
}

fn refused(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::Notify(ZonesEvent::Refused { .. })))
}

fn heard(effects: &[Effect]) -> impl Iterator<Item = &ZonesEvent> {
    effects.iter().filter_map(|effect| match effect {
        Effect::Notify(event) => Some(event),
        _ => None,
    })
}

/// The rules of the run: what was not touched for two days goes to the
/// second zone (a rule that asks the time), pictures to the first.
fn the_rules() -> Vec<Rule> {
    vec![
        Rule {
            name: "stale".to_string(),
            matchers: vec![Matcher::Untouched { days: 2 }],
            action: RuleAction::AssignToZone {
                zone: ZoneId::new("zone-2"),
            },
            ..Rule::default()
        },
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
    ]
}

/// Preview, apply and (mostly) take back, each held against the others.
fn rules_step(world: &mut World, chance: &mut Chance) -> Result<(String, Vec<Effect>), String> {
    let scope = *chance.pick(&[Scope::Loose, Scope::All]);
    let before = (world.core.config().clone(), world.core.state().clone());
    let effects = world.ask(Command::PreviewRules { scope });
    let Some(ZonesEvent::RulesPreviewed { plan, lines }) = heard(&effects).next().cloned() else {
        return Err("a dry run said nothing".to_string());
    };
    if (world.core.config(), world.core.state()) != (&before.0, &before.1) {
        return Err("a dry run changed something".to_string());
    }
    if effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::SaveConfig | Effect::SaveState | Effect::Ask(_)
        )
    }) {
        return Err("a dry run asked for something to be done".to_string());
    }
    if lines.len() != plan.steps.len() + plan.held.len() {
        return Err("the lines of a dry run are not its plan".to_string());
    }
    for (step, line) in plan.steps.iter().zip(&lines) {
        let from = world.core.zone_of(&step.item);
        if line.item != step.item
            || line.from.as_ref() != from
            || line.decided != step.decided
            || line.held.is_some()
        {
            return Err(format!(
                "a line of the dry run says other than its step: {line:?}"
            ));
        }
    }
    count(&SEEN.dry_runs, 1);
    let mut effects = world.ask(Command::ApplyRules { scope });
    let applied = heard(&effects).find_map(|event| match event {
        ZonesEvent::RulesApplied { plan, .. } => Some(plan.clone()),
        _ => None,
    });
    if applied.as_ref() != Some(&plan) {
        return Err(format!(
            "what was applied is not what was previewed: {applied:?} for {plan:?}"
        ));
    }
    let left = world.core.plan(scope, world.now);
    if !left.steps.is_empty() {
        return Err(format!(
            "after applying, the rules would still do {:?}",
            left.steps
        ));
    }
    if !plan.steps.is_empty() && chance.one_in_a_hundred(70) {
        effects.extend(world.ask(Command::UndoRules));
        if world.core.state().placements != before.1.placements {
            return Err("taking the rules back did not give the placements there were".to_string());
        }
    }
    Ok((
        format!("the rules, previewed and applied ({scope:?})"),
        effects,
    ))
}

/// Zones by type: asked for, made and (mostly) taken back, each held
/// against the others.
fn auto_step(world: &mut World, chance: &mut Chance) -> Result<(String, Vec<Effect>), String> {
    let scope = *chance.pick(&[Scope::Loose, Scope::All]);
    let min_items = chance
        .one_in_a_hundred(50)
        .then(|| chance.between(1, 4) as u32);
    let add_rules = chance
        .one_in_a_hundred(50)
        .then(|| chance.one_in_a_hundred(50));
    let display = (world.second && chance.one_in_a_hundred(40)).then(|| DisplayId::new("fake-2"));
    let ask = |dry_run: bool| Command::AutoCreate {
        scope,
        min_items,
        display: display.clone(),
        add_rules,
        dry_run,
    };
    let what = format!("zones by type ({scope:?}, at least {min_items:?})");
    let before = (world.core.config().clone(), world.core.state().clone());
    let effects = world.ask(ask(true));
    if (world.core.config(), world.core.state()) != (&before.0, &before.1) {
        return Err("asking what zones would be made changed something".to_string());
    }
    if effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::SaveConfig | Effect::SaveState | Effect::Ask(_)
        )
    }) {
        return Err("asking what zones would be made asked for something to be done".to_string());
    }
    let Some(ZonesEvent::AutoPlanned { plan }) = heard(&effects).next().cloned() else {
        // (Every zone is locked: refused, and said so.)
        return if refused(&effects) && world.core.config().lock.members {
            Ok((what, effects))
        } else {
            Err("asking what zones would be made said nothing".to_string())
        };
    };
    // (The core, asked without a command, gives rules as the
    // configuration has it; the command may have said otherwise.)
    let mut asked = world
        .core
        .auto_plan(scope, min_items, display.as_ref(), world.now);
    for (given, said) in asked.zones.iter_mut().zip(&plan.zones) {
        given.rule = if add_rules.is_some() {
            said.rule
        } else {
            given.rule
        };
    }
    if plan != asked {
        return Err("the plan that was said is not the plan the core gives".to_string());
    }
    // Nothing of a plan is outside its display, and no item is in it twice.
    let area = world
        .core
        .displays()
        .iter()
        .find(|shown| Some(&shown.id) == plan.display.as_ref())
        .map(|shown| shown.rect);
    let mut seen = BTreeSet::new();
    for planned in &plan.zones {
        let inside = area.is_some_and(|area| crate::snap::within(planned.rect, area));
        if planned.zone.is_none() && !inside {
            return Err(format!(
                "a zone would be made at {:?}, outside its display",
                planned.rect
            ));
        }
        if planned.items.is_empty() || planned.items.iter().any(|item| !seen.insert(item.clone())) {
            return Err(format!(
                "{} would be made for nothing, or for an item twice",
                planned.title
            ));
        }
    }
    let mut effects = world.ask(ask(false));
    let made = heard(&effects).find_map(|event| match event {
        ZonesEvent::AutoCreated {
            zones,
            items,
            rules,
        } => Some((zones.clone(), *items, *rules)),
        _ => None,
    });
    let Some((made, items, rules)) = made else {
        return Err("making the zones said nothing".to_string());
    };
    let new: Vec<_> = plan
        .zones
        .iter()
        .filter(|planned| planned.zone.is_none())
        .collect();
    if made.len() != new.len()
        || items != seen.len()
        || rules != new.iter().filter(|planned| planned.rule).count()
    {
        return Err(format!("what was made ({made:?}, {items} items, {rules} rules) is not what was planned: {plan:?}"));
    }
    let mut fresh = made.iter();
    for planned in &plan.zones {
        let zone = planned
            .zone
            .as_ref()
            .or_else(|| fresh.next())
            .ok_or("a planned zone was not made")?;
        let definition = world
            .core
            .config()
            .zone(zone)
            .ok_or("a zone that was planned is not there")?;
        if definition.auto.as_deref() != Some(planned.category.as_str()) {
            return Err(format!("{zone} is not the zone of {}", planned.category));
        }
        if planned.zone.is_none() && world.core.zone_rect(zone) != Some(planned.rect) {
            return Err(format!(
                "{zone} is at {:?}, and {:?} was planned",
                world.core.zone_rect(zone),
                planned.rect
            ));
        }
        if let Some(stray) = planned
            .items
            .iter()
            .find(|item| world.core.zone_of(item) != Some(zone))
        {
            return Err(format!("{stray} was planned for {zone} and is not in it"));
        }
    }
    let left = world
        .core
        .auto_plan(scope, min_items, display.as_ref(), world.now);
    if !left.zones.is_empty() {
        return Err(format!(
            "after making the zones, more would be made: {:?}",
            left.zones
        ));
    }
    if plan.zones.is_empty() {
        return Ok((what, effects));
    }
    count(&SEEN.made_by_type, 1);
    if chance.one_in_a_hundred(70) {
        effects.extend(world.ask(Command::UndoAutoCreate));
        let (config, state) = (world.core.config(), world.core.state());
        if config.zones != before.0.zones || config.rules != before.0.rules {
            return Err(
                "taking the zones by type back did not give the zones and rules there were"
                    .to_string(),
            );
        }
        if state.placements != before.1.placements || state.auto_undo.is_some() {
            return Err(
                "taking the zones by type back did not give the placements there were".to_string(),
            );
        }
        count(&SEEN.unmade, 1);
    }
    Ok((what, effects))
}

/// One step of chance.
fn step(world: &mut World, chance: &mut Chance) -> Result<(Kind, String, Vec<Effect>), String> {
    let zone = world.a_zone(chance);
    let outside =
        |world: &mut World, what: &str| Ok((Kind::Change, what.to_string(), world.pump()));
    let command = match chance.below(81) {
        78 => {
            let (what, effects) = auto_step(world, chance)?;
            return Ok((Kind::Change, what, effects));
        }
        79 => Command::GreyIcons {
            on: chance
                .one_in_a_hundred(60)
                .then(|| chance.one_in_a_hundred(50)),
        },
        // (Whatever was last made by type and not taken back in its step.)
        80 => Command::UndoAutoCreate,
        68 => {
            world.backend.slow_operations = chance.one_in_a_hundred(75);
            return outside(world, "file operations take long, or no longer");
        }
        69 | 70 => {
            let name = *chance.pick(&["o1.txt", "o2.txt", "o3.txt", "not-there.txt"]);
            let zone = chance.one_in_a_hundred(60).then(|| world.a_zone(chance));
            let effects = world.ask(Command::BringIn {
                paths: vec![format!("/outside/{name}")],
                folder: DESKTOP.to_string(),
                copy: true,
                zone,
                at: None,
            });
            return Ok((Kind::Change, format!("{name} is brought in"), effects));
        }
        71 | 72 => {
            if chance.one_in_a_hundred(60) && !world.backend.under_way.is_empty() {
                world.backend.fail_next = Some(BackendError::Refused {
                    what: "the disk is full".to_string(),
                });
                count(&SEEN.failed_later, 1);
            }
            world.backend.finish_operations();
            world.backend.fail_next = None;
            return outside(world, "what was under way ends");
        }
        73 | 76 | 77 => {
            let under_way: Vec<u64> = world.begun.iter().copied().collect();
            let request = if under_way.is_empty() {
                99
            } else {
                *chance.pick(&under_way)
            };
            if chance.one_in_a_hundred(30) {
                world
                    .backend
                    .progress(request, chance.below(50) as u64, 100);
                return outside(world, "an operation says how far it is");
            }
            let effects = world.ask(Command::CancelOperation { request });
            return Ok((
                Kind::Change,
                format!("operation {request} is called off"),
                effects,
            ));
        }
        74 => {
            let effects = world.start_anew(chance)?;
            return Ok((
                Kind::Change,
                "the program is started anew".to_string(),
                effects,
            ));
        }
        75 => {
            let effects = world.in_and_back(chance)?;
            return Ok((
                Kind::Change,
                "a file is brought in and taken back".to_string(),
                effects,
            ));
        }
        0 => {
            let item = world.a_file(DESKTOP, chance);
            world.backend.add(DESKTOP, item);
            return outside(world, "a file comes to the desktop");
        }
        1 => {
            let gone = world.a_desktop_item(chance);
            world.backend.remove(&gone);
            return outside(world, "a file goes from the desktop");
        }
        2 | 3 => {
            let folder = *chance.pick(&TREE);
            let item = world.a_file(folder, chance);
            world.backend.add(folder, item);
            return outside(world, "a file comes into the tree");
        }
        4 => {
            let folder = *chance.pick(&TREE);
            let files: Vec<Item> = world
                .backend
                .in_folder(folder)
                .into_iter()
                .filter(|item| item.kind == ItemKind::File)
                .collect();
            if let Some(gone) = (!files.is_empty()).then(|| chance.pick(&files).id.clone()) {
                world.backend.remove(&gone);
            }
            return outside(world, "a file goes from the tree");
        }
        5 => {
            let reachable = world.backend.is_reachable("/tree/c");
            world.backend.set_reachable("/tree/c", !reachable);
            return outside(world, "a folder goes away, or is back");
        }
        6 => {
            world.slow = !world.slow;
            world.backend.set_slow("/tree/a", world.slow);
            world.backend.set_slow("/tree/a/b", world.slow);
            return outside(world, "folders are slow, or no longer");
        }
        7 | 8 => {
            world.backend.answer();
            return outside(world, "slow folders answer");
        }
        9 => {
            world.recent = world.fresh_recent(chance);
            world
                .backend
                .set_source(SmartSource::Recent, world.recent.clone());
            return outside(world, "other files were used last");
        }
        10 => {
            world.shared = !world.shared;
            world.backend.share_screen(world.shared);
            return outside(world, "the screen is shared, or no longer");
        }
        11 => {
            let name = Some(chance.between(1, 2).to_string());
            let mut effects = world
                .core
                .event(BackendEvent::Workspace { name }, world.now);
            effects.extend(world.pump());
            return Ok((Kind::Change, "another workspace".to_string(), effects));
        }
        12 => {
            world.second = !world.second;
            if world.second {
                world
                    .backend
                    .plug(Display::new("fake-2", Rect::new(1920, 0, 1600, 900)));
            } else {
                world.backend.unplug(&DisplayId::new("fake-2"));
            }
            return outside(world, "a display comes, or goes");
        }
        13 => {
            world.now += chance.below(30) as u64 * HOUR;
            return outside(world, "time passes");
        }
        14 => {
            let (what, effects) = rules_step(world, chance)?;
            return Ok((Kind::Change, what, effects));
        }
        15 | 16 if world.zones().len() < 9 => {
            let kind = match chance.below(10) {
                0..=2 => ZoneKind::Portal {
                    directory: chance.pick(&TREE).to_string(),
                },
                3 | 4 => ZoneKind::Smart { query: Vec::new() },
                _ => ZoneKind::Manual,
            };
            let smart = matches!(kind, ZoneKind::Smart { .. });
            let rect = chance.one_in_a_hundred(70).then(|| {
                Rect::new(
                    chance.between(-200, 3600),
                    chance.between(-100, 1100),
                    chance.between(100, 700),
                    chance.between(60, 500),
                )
            });
            let mut effects = world.ask(Command::CreateZone {
                title: format!("Zone {}", chance.below(50)),
                kind,
                rect,
            });
            let made = heard(&effects).find_map(|event| match event {
                ZonesEvent::ZoneCreated { zone } => Some(zone.clone()),
                _ => None,
            });
            if let Some(zone) = made.filter(|_| smart) {
                let changes = json!({"source": {"from": "recent"}, "limit": chance.below(4)});
                effects.extend(world.ask(Command::ModifyZone { zone, changes }));
            }
            return Ok((Kind::Change, "a zone is made".to_string(), effects));
        }
        15 | 16 => Command::DeleteZone { zone },
        17 => Command::DeleteZone { zone },
        18 | 19 => Command::MoveZone {
            zone,
            x: chance.between(-300, 3700),
            y: chance.between(-200, 1200),
            snap: chance.one_in_a_hundred(60),
        },
        20..=22 => Command::AddItem {
            zone,
            item: world.a_desktop_item(chance),
            index: None,
            cell: None,
        },
        23 => Command::RemoveItem {
            item: world.a_desktop_item(chance),
        },
        24 | 25 => Command::RollUp {
            zone,
            rolled_up: chance
                .one_in_a_hundred(60)
                .then(|| chance.one_in_a_hundred(70)),
        },
        26..=28 => {
            world.at_ms += chance.below(300) as u64;
            Command::PointerOver {
                zone: chance.one_in_a_hundred(75).then_some(zone),
                at_ms: world.at_ms,
                dragging: chance.one_in_a_hundred(25),
            }
        }
        29 | 30 => {
            world.at_ms += chance.below(600) as u64;
            Command::Tick { at_ms: world.at_ms }
        }
        31 => Command::Expand {
            zone,
            expanded: chance
                .one_in_a_hundred(50)
                .then(|| chance.one_in_a_hundred(60)),
        },
        32 | 33 => Command::Tab {
            zone,
            with: world.a_zone(chance),
        },
        34 => {
            if chance.one_in_a_hundred(40) {
                Command::Untab { zone }
            } else {
                Command::ActivateTab { zone }
            }
        }
        35 => Command::MoveTab {
            zone,
            index: chance.below(4),
        },
        36 => Command::SplitTab {
            zone,
            x: chance.between(0, 1500),
            y: chance.between(0, 800),
            snap: chance.one_in_a_hundred(50),
        },
        37 => Command::NextTab {
            zone,
            backwards: chance.one_in_a_hundred(50),
        },
        38 => Command::AlwaysVisible { zone, on: None },
        39 => Command::ItemAlwaysVisible {
            item: world.a_desktop_item(chance),
            on: None,
        },
        40 => Command::QuickHide { hidden: None },
        41 => {
            if chance.one_in_a_hundred(50) {
                Command::TogglePeek
            } else {
                Command::Peek {
                    on: chance.one_in_a_hundred(50),
                }
            }
        }
        42 => Command::Dismiss,
        43 => Command::Open {
            items: vec![world.an_item(chance)],
            with: None,
            in_place: chance.one_in_a_hundred(80),
        },
        44 => Command::Lock {
            zone,
            geometry: chance
                .one_in_a_hundred(50)
                .then(|| chance.one_in_a_hundred(25)),
            members: chance
                .one_in_a_hundred(50)
                .then(|| chance.one_in_a_hundred(25)),
        },
        45 => {
            // Mostly unlocked again: a run in which everything is locked
            // for long sees little.
            let lock = chance.one_in_a_hundred(25);
            Command::LockAll {
                geometry: Some(lock && chance.one_in_a_hundred(50)),
                members: Some(lock),
            }
        }
        46 | 47 => {
            let changes = match chance.below(7) {
                0 => json!({"private": chance.one_in_a_hundred(50)}),
                1 => {
                    json!({"workspaces": if chance.one_in_a_hundred(50) { vec!["1"] } else { vec![] }})
                }
                2 => json!({"reserves_space": chance.one_in_a_hundred(60)}),
                3 => json!({"limit": chance.below(5)}),
                4 => json!({"expand_on": *chance.pick(&[Some("click"), Some("hover"), None])}),
                5 => json!({"look": {"opacity": chance.below(140)}}),
                _ => json!({"title": format!("Renamed {}", chance.below(9)), "sort": "name"}),
            };
            Command::ModifyZone { zone, changes }
        }
        48..=50 => {
            let portal = world.a_portal(chance);
            let folders: Vec<ItemId> = world
                .core
                .zone_holds(&portal, world.now)
                .into_iter()
                .filter(|item| item.kind == ItemKind::Folder)
                .map(|item| item.id.clone())
                .collect();
            let item = if folders.is_empty() {
                world.an_item(chance)
            } else {
                chance.pick(&folders).clone()
            };
            Command::EnterFolder { zone: portal, item }
        }
        51 => Command::NavigateUp {
            zone: world.a_portal(chance),
        },
        52 => Command::NavigateBack {
            zone: world.a_portal(chance),
        },
        53 => Command::NavigateForward {
            zone: world.a_portal(chance),
        },
        54 => Command::NavigateTo {
            zone: world.a_portal(chance),
            directory: chance
                .pick(&["/tree", "/tree/a", "/tree/a/b", "/tree/c", "/elsewhere"])
                .to_string(),
        },
        55 => Command::RefreshZone { zone },
        56 => {
            if chance.one_in_a_hundred(40) {
                world.made += 1;
                Command::NewFolder {
                    zone: world.a_portal(chance),
                    name: format!("n{}", world.made),
                }
            } else if chance.one_in_a_hundred(50) {
                Command::UndoFileOperation
            } else {
                let folder = *chance.pick(&TREE);
                let files: Vec<ItemId> = world
                    .backend
                    .in_folder(folder)
                    .into_iter()
                    .filter(|item| item.kind == ItemKind::File)
                    .map(|item| item.id)
                    .collect();
                Command::MoveItemsToFolder {
                    items: files.into_iter().take(1).collect(),
                    folder: chance.pick(&TREE).to_string(),
                    copy: chance.one_in_a_hundred(50),
                }
            }
        }
        57 | 58 => Command::Filter {
            zone: chance.one_in_a_hundred(70).then_some(zone),
            typed: chance
                .pick(&["", "f", "t", "1", "png", "nothing fits this"])
                .to_string(),
        },
        59 => Command::CheckTimedRules,
        60 => match chance.below(5) {
            0 | 1 => Command::SaveSnapshot {
                name: format!("s{}", chance.below(3)),
            },
            2 => Command::RenameSnapshot {
                name: format!("s{}", chance.below(3)),
                to: format!("s{}", chance.below(3)),
            },
            3 => Command::DeleteSnapshot {
                name: format!("s{}", chance.below(3)),
            },
            _ => Command::ExportLayout {
                name: chance
                    .one_in_a_hundred(30)
                    .then(|| format!("s{}", chance.below(3))),
            },
        },
        61 | 66 | 67 => {
            if world.exported.is_empty() || chance.one_in_a_hundred(40) {
                Command::RestoreSnapshot {
                    name: format!("s{}", chance.below(3)),
                }
            } else {
                let mut document = chance.pick(&world.exported).clone();
                // Now and then one that cannot be right.
                if chance.one_in_a_hundred(15) {
                    document["version"] = 99.into();
                }
                Command::ImportLayout {
                    document,
                    apply: chance.one_in_a_hundred(70),
                }
            }
        }
        62 => match chance.below(3) {
            0 => {
                let zone = if chance.one_in_a_hundred(35) {
                    ZoneId::new(DESK)
                } else {
                    zone
                };
                Command::FocusZone {
                    zone: chance.one_in_a_hundred(85).then_some(zone),
                }
            }
            1 => Command::FocusItem {
                item: chance.one_in_a_hundred(85).then(|| world.an_item(chance)),
            },
            _ => Command::FocusNeighbour {
                direction: *chance.pick(&Direction::ALL),
            },
        },
        63 => Command::MoveItemFocus {
            step: *chance.pick(&[
                Step::Left,
                Step::Right,
                Step::Up,
                Step::Down,
                Step::First,
                Step::Last,
            ]),
            extend: chance.one_in_a_hundred(30),
        },
        64 => match chance.below(4) {
            0 => Command::Select {
                items: vec![world.an_item(chance), world.an_item(chance)],
                mode: *chance.pick(&[
                    SelectMode::Replace,
                    SelectMode::Add,
                    SelectMode::Remove,
                    SelectMode::Toggle,
                ]),
            },
            1 => Command::OpenFocused,
            2 => Command::MoveFocusedItems {
                direction: Some(*chance.pick(&Direction::ALL)),
                zone: None,
            },
            _ => {
                let zone = if chance.one_in_a_hundred(35) {
                    ZoneId::new(DESK)
                } else {
                    zone
                };
                Command::MoveFocusedItems {
                    direction: None,
                    zone: Some(zone),
                }
            }
        },
        _ => match chance.below(4) {
            0 => Command::Mask { masked: None },
            1 => Command::Reveal {
                zone: Some(zone),
                revealed: None,
            },
            2 => Command::Reveal {
                zone: None,
                revealed: chance
                    .one_in_a_hundred(50)
                    .then(|| chance.one_in_a_hundred(50)),
            },
            _ => Command::Search {
                typed: chance.pick(&["f", "t", "r", "png", "n"]).to_string(),
            },
        },
    };
    let kind = match &command {
        Command::DeleteZone { .. } | Command::UndoAutoCreate => Kind::Deletes,
        Command::RestoreSnapshot { .. } | Command::ImportLayout { apply: true, .. } => {
            Kind::Replaces
        }
        Command::PointerOver { .. }
        | Command::Tick { .. }
        | Command::Expand { .. }
        | Command::Filter { .. }
        | Command::Select { .. }
        | Command::Mask { .. }
        | Command::Reveal { .. }
        | Command::EnterFolder { .. }
        | Command::NavigateTo { .. }
        | Command::NavigateUp { .. }
        | Command::NavigateBack { .. }
        | Command::NavigateForward { .. }
        | Command::RefreshZone { .. }
        | Command::Search { .. }
        | Command::ExportLayout { .. }
        | Command::Dismiss
        | Command::TogglePeek
        | Command::Peek { .. } => Kind::View,
        _ => Kind::Change,
    };
    let what = format!("{command:?}");
    let effects = world.ask(command.clone());
    // What each kind of command must have come to, where it was carried out.
    if !refused(&effects) {
        match &command {
            Command::Tick { at_ms } => {
                if world.core.wake_at_ms().is_some_and(|due| due <= *at_ms) {
                    return Err(format!(
                        "after a tick at {at_ms}, something still waits for {:?}",
                        world.core.wake_at_ms()
                    ));
                }
            }
            Command::ImportLayout {
                document,
                apply: true,
            } => {
                let read = export::read(document)?;
                if world.core.config().zones != read.layout.zones {
                    return Err(
                        "a layout was brought in, and the zones are not its zones".to_string()
                    );
                }
            }
            Command::CheckTimedRules => {
                for event in heard(&effects) {
                    let ZonesEvent::RulesApplied { plan, .. } = event else {
                        continue;
                    };
                    count(&SEEN.timed, plan.steps.len());
                    let timed = |decided: &Decided| match decided {
                        Decided::Rule(index) => rules::is_timed(&world.core.config().rules[*index]),
                        Decided::CatchAll => false,
                    };
                    if plan.steps.iter().any(|step| !timed(&step.decided)) {
                        return Err(
                            "the timed check carried out a rule that asks nothing of the time"
                                .to_string(),
                        );
                    }
                }
            }
            Command::MoveTab { .. } | Command::SplitTab { .. } | Command::NextTab { .. } => {
                count(&SEEN.tabs, 1)
            }
            Command::GreyIcons { on } => {
                count(&SEEN.grey, 1);
                let grey = world.core.grey_icons();
                let told = heard(&effects).any(
                    |event| matches!(event, ZonesEvent::GreyIconsChanged { on } if *on == grey),
                );
                if on.is_some_and(|on| on != grey) || told != effects.contains(&Effect::SaveState) {
                    return Err("icons are grey otherwise than was asked, or it was not told as it was saved".to_string());
                }
            }
            _ => {}
        }
    }
    for event in heard(&effects) {
        match event {
            ZonesEvent::LayoutExported { document } => {
                world.exported.push(document.to_value());
                if world.exported.len() > 4 {
                    world.exported.remove(0);
                }
            }
            ZonesEvent::Navigated { .. } => count(&SEEN.navigated, 1),
            ZonesEvent::ZoneExpanded { expanded: true, .. } => count(&SEEN.opened_for_now, 1),
            ZonesEvent::FocusChanged { zone: Some(_), .. } => count(&SEEN.keyboard_moved, 1),
            _ => {}
        }
    }
    Ok((kind, what, effects))
}

/// What must hold of the scene and of what is so for the moment.
fn look_over(world: &World, scene: &Scene) -> Result<(), String> {
    let core = &world.core;
    let now = world.now;
    // The keyboard: on a zone that is shown and an item it shows.
    match core.focus() {
        // On the bare desktop: where items lie loose and are to be seen,
        // and on one of them.
        Some(focus) if focus.desk.is_some() => {
            count(&SEEN.on_the_desk, 1);
            let lying = scene
                .loose
                .iter()
                .find(|loose| Some(&loose.display) == focus.desk.as_ref());
            let Some(lying) = lying.filter(|loose| loose.focused && !loose.items.is_empty()) else {
                return Err("the desktop has the keyboard and nothing lies loose there".to_string());
            };
            if focus.zone.as_str() != DESK || scene.loose_hidden {
                return Err(
                    "the desktop has the keyboard and is put away, or is called otherwise"
                        .to_string(),
                );
            }
            if let Some(item) = &focus.item {
                if !lying
                    .items
                    .iter()
                    .any(|placed| &placed.item == item && placed.focused)
                {
                    return Err(format!("{item} has the keyboard and does not lie loose"));
                }
            }
        }
        Some(focus) => {
            if !core.is_shown(&focus.zone) {
                return Err(format!("{} has the keyboard and is not shown", focus.zone));
            }
            if let Some(item) = &focus.item {
                if core.is_masked(&focus.zone)
                    || !core
                        .zone_items(&focus.zone, now)
                        .iter()
                        .any(|shown| &shown.id == item)
                {
                    return Err(format!(
                        "{item} has the keyboard and {} does not show it",
                        focus.zone
                    ));
                }
            }
        }
        None => {}
    }
    // The desktop the core knows is the desktop there is: nothing that is
    // not there, and nothing missing. (Each item once: loose, or in the
    // one zone it was put into.)
    let mut known: Vec<ItemId> = core
        .loose_items()
        .into_iter()
        .map(|item| item.id.clone())
        .collect();
    for zone in core
        .config()
        .zones
        .iter()
        .filter(|zone| zone.kind == ZoneKind::Manual)
    {
        known.extend(
            core.zone_holds(&zone.id, now)
                .into_iter()
                .map(|item| item.id.clone()),
        );
    }
    let there = world.desktop();
    if known.len() != there.len() || known.iter().cloned().collect::<BTreeSet<_>>() != there {
        return Err(format!(
            "the core knows {known:?} on the desktop, and there is {there:?}"
        ));
    }
    // Every operation that was begun is under way until it has ended.
    let shown: BTreeSet<u64> = scene
        .operations
        .iter()
        .map(|operation| operation.request)
        .collect();
    let under_way: BTreeSet<u64> = world
        .backend
        .under_way
        .iter()
        .map(|(request, _)| *request)
        .collect();
    if shown != world.begun || shown != under_way {
        return Err(format!(
            "under way: the scene says {shown:?}, {:?} were begun and not ended, the backend has {under_way:?}",
            world.begun
        ));
    }
    if let Some(stray) = core
        .selection()
        .iter()
        .find(|item| core.item(item).is_none())
    {
        return Err(format!("{stray} is chosen and is not there"));
    }
    let hidden_here = |zone: &ZoneId| {
        core.profile()
            .is_some_and(|profile| profile.hidden.contains(zone))
    };
    for shown in &scene.zones {
        let Some(definition) = core.config().zone(&shown.zone) else {
            return Err(format!("{} is in the scene and is no zone", shown.zone));
        };
        // Put away with the one key, only what was said to stay is shown;
        // otherwise everything that belongs here is.
        let belongs = !hidden_here(&shown.zone)
            && (definition.workspaces.is_empty()
                || core.workspace().is_none_or(|workspace| {
                    definition.workspaces.iter().any(|name| name == workspace)
                }));
        let expected =
            belongs && (scene.peeking || !scene.quick_hidden || definition.always_visible);
        if shown.shown != expected {
            return Err(format!(
                "{} is shown: {}, and should be: {expected}",
                shown.zone, shown.shown
            ));
        }
        if scene.quick_hidden && !scene.peeking {
            count(&SEEN.put_away, 1);
        }
        // What is covered is not in the scene.
        if shown.masked {
            count(&SEEN.covered, 1);
            if !shown.items.items.is_empty() {
                return Err(format!(
                    "{} is covered and its items are in the scene",
                    shown.zone
                ));
            }
        }
        if shown.rolled_up != definition.rolled_up || (shown.expanded && !shown.rolled_up) {
            return Err(format!(
                "{} is said to be rolled up otherwise than it is",
                shown.zone
            ));
        }
        // A portal whose folder is listed shows exactly what is in it.
        if let Some(portal) = &shown.portal {
            if !crate::portal::within(&portal.root, &portal.directory) {
                return Err(format!(
                    "{} stands in {}, outside {}",
                    shown.zone, portal.directory, portal.root
                ));
            }
            match &portal.listing {
                Listing::Ready => {
                    let there: BTreeSet<ItemId> = world
                        .backend
                        .in_folder(&portal.directory)
                        .into_iter()
                        .map(|item| item.id)
                        .collect();
                    let held: BTreeSet<ItemId> = core
                        .zone_holds(&shown.zone, now)
                        .into_iter()
                        .map(|item| item.id.clone())
                        .collect();
                    if there != held {
                        return Err(format!(
                            "{} shows {held:?} of {}, which holds {there:?}",
                            shown.zone, portal.directory
                        ));
                    }
                    count(&SEEN.mirrored, 1);
                }
                Listing::Unavailable { .. } => {
                    count(&SEEN.unavailable, 1);
                    if shown.item_count != 0 {
                        return Err(format!(
                            "{} shows items of a folder that could not be listed",
                            shown.zone
                        ));
                    }
                }
                Listing::Loading => {}
            }
        }
        // A zone that fills itself from the files used last shows some of them.
        if matches!(definition.kind, ZoneKind::Smart { .. })
            && definition.source == SmartSource::Recent
        {
            let held = core.zone_holds(&shown.zone, now);
            if let Some(stray) = held
                .iter()
                .find(|item| !world.recent.iter().any(|recent| recent.id == item.id))
            {
                return Err(format!(
                    "{} shows {}, which is not among the files used last",
                    shown.zone, stray.id
                ));
            }
            let most = if definition.limit == 0 {
                world.recent.len()
            } else {
                definition.limit as usize
            };
            if shown.source == Some(Listing::Ready) && held.len() != most.min(world.recent.len()) {
                return Err(format!(
                    "{} shows {} of the files used last",
                    shown.zone,
                    held.len()
                ));
            }
            count(&SEEN.sources, 1);
        }
    }
    // What is covered is not found.
    for hit in core
        .search("f", now)
        .into_iter()
        .chain(core.search("r", now))
    {
        let covered = match &hit.zone {
            Some(zone) => core.is_masked(zone),
            None => core.loose_masked(),
        };
        if covered {
            return Err(format!("{} was found, and is covered", hit.item));
        }
    }
    if scene
        .loose
        .iter()
        .any(|loose| loose.masked && !loose.items.is_empty())
    {
        return Err("the loose items are covered and are in the scene".to_string());
    }
    // The loose items are put away with the zones (and seen again by
    // whoever peeks); and icons are grey as kept.
    let away = scene.quick_hidden && !scene.peeking;
    if scene.loose_hidden != away || scene.grey_icons != core.state().grey_icons {
        return Err(
            "the scene says otherwise than is kept whether loose items are away, or icons grey"
                .to_string(),
        );
    }
    // A zone made by type is one filled by hand, and its type's only one.
    let mut types = BTreeSet::new();
    for zone in core
        .config()
        .zones
        .iter()
        .filter(|zone| zone.kind == ZoneKind::Manual)
    {
        if zone
            .auto
            .as_ref()
            .is_some_and(|category| !types.insert(category.clone()))
        {
            return Err(format!("{} is a second zone for {:?}", zone.id, zone.auto));
        }
    }
    Ok(())
}

fn later_run(seed: u64) -> Result<(), String> {
    let mut chance = Chance(seed | 1);
    let mut backend = FakeBackend::new();
    backend.add_folder("/tree");
    backend.add_folder_in("/tree", "a");
    backend.add_folder_in("/tree/a", "b");
    backend.add_folder_in("/tree", "c");
    backend.set_source(SmartSource::Recent, Vec::new());
    let config = ZonesConfig {
        enabled: true,
        rules: the_rules(),
        rules_on_arrival: chance.one_in_a_hundred(50),
        roll_up: RollUpSettings {
            expand_on: *chance.pick(&[ExpandOn::Hover, ExpandOn::Click]),
            expand_delay_ms: chance.below(250) as u32,
            collapse_delay_ms: chance.below(500) as u32,
            ..RollUpSettings::default()
        },
        ..ZonesConfig::default()
    };
    let mut world = World {
        core: ZonesCore::new(config, ZonesState::default()),
        host: Host::new(),
        backend,
        now: START,
        at_ms: 1000,
        made: 0,
        recent: Vec::new(),
        exported: Vec::new(),
        told_reserved: Vec::new(),
        second: false,
        shared: false,
        slow: false,
        begun: BTreeSet::new(),
    };
    world.backend.slow_operations = chance.one_in_a_hundred(60);
    world.backend.add_folder("/outside");
    for name in ["o1.txt", "o2.txt", "o3.txt"] {
        world.backend.add_file_in("/outside", name);
    }
    for folder in TREE {
        for _ in 0..3 {
            let item = world.a_file(folder, &mut chance);
            world.backend.add(folder, item);
        }
    }
    for _ in 0..10 {
        let item = world.a_file(DESKTOP, &mut chance);
        world.backend.add(DESKTOP, item);
    }
    world
        .host
        .start(&mut world.core, &mut world.backend, world.now);
    for kind in [
        ZoneKind::Manual,
        ZoneKind::Manual,
        ZoneKind::Portal {
            directory: "/tree".to_string(),
        },
        ZoneKind::Smart { query: Vec::new() },
    ] {
        let smart = matches!(kind, ZoneKind::Smart { .. });
        world.ask(Command::CreateZone {
            title: "Zone".to_string(),
            kind,
            rect: None,
        });
        if smart {
            let zone = world.zones().pop().unwrap_or_default();
            world.ask(Command::ModifyZone {
                zone,
                changes: json!({"source": {"from": "recent"}}),
            });
        }
    }

    for number in 0..240 {
        let zones_before: BTreeSet<ZoneId> = world.zones().into_iter().collect();
        let layout_before = world.core.current_layout("", world.now);
        let (kind, what, effects) =
            step(&mut world, &mut chance).map_err(|problem| format!("step {number}: {problem}"))?;
        let wrong = |problem: String| Err(format!("step {number} ({what}): {problem}"));
        let core = &world.core;
        let now = world.now;

        if let Some(broken) = core.broken() {
            return wrong(broken);
        }
        // No zone is lost but by being deleted; whatever replaced the
        // whole layout kept the one there was.
        let zones: BTreeSet<ZoneId> = world.zones().into_iter().collect();
        let carried_out = !refused(&effects);
        match kind {
            Kind::Deletes => {}
            Kind::Replaces if carried_out => {
                count(&SEEN.layouts_replaced, 1);
                if !core
                    .state()
                    .snapshots
                    .iter()
                    .any(|kept| kept.same_layout(&layout_before))
                {
                    return wrong(
                        "the whole layout was replaced, and the one there was is in no snapshot"
                            .to_string(),
                    );
                }
            }
            _ => {
                if let Some(lost) = zones_before.difference(&zones).next() {
                    return wrong(format!("{lost} is lost"));
                }
            }
        }
        if !carried_out && kind != Kind::Change && zones != zones_before {
            return wrong("a command was refused, and the zones are others".to_string());
        }
        // What is so only for the moment asks for nothing to be saved.
        if kind == Kind::View {
            count(&SEEN.not_saved, 1);
            if effects
                .iter()
                .any(|effect| matches!(effect, Effect::SaveConfig | Effect::SaveState))
            {
                return wrong(
                    "something that is so for the moment only asked to be saved".to_string(),
                );
            }
        }
        // The room kept free of windows is what was last told.
        world.hear(&effects);
        let core = &world.core;
        if core.reserved_regions() != world.told_reserved {
            return wrong(format!(
                "the room kept is {:?}, and {:?} was told",
                core.reserved_regions(),
                world.told_reserved
            ));
        }
        count(&SEEN.room_kept, world.told_reserved.len());

        let scene = core.scene(now);
        if let Err(problem) = look_over(&world, &scene) {
            return wrong(problem);
        }

        // Nothing a zone shows by itself is written: no placement in a
        // zone that is not filled by hand, and no file used last anywhere.
        let written = store::encode(core.state());
        for zone in &core.config().zones {
            if zone.kind != ZoneKind::Manual
                && !core.state().placements.items_of(&zone.id).is_empty()
            {
                return wrong(format!("{} fills itself and has placements", zone.id));
            }
        }
        if written.contains("/recent/") {
            return wrong("a file used last is in the state".to_string());
        }
        // Everything survives its text.
        match store::decode(&written) {
            Ok((state, None)) if &state == core.state() => {}
            other => {
                return wrong(format!(
                    "the state does not come back from its text: {other:?}"
                ))
            }
        }
        for effect in &effects {
            let text = serde_json::to_string(effect).map_err(|error| error.to_string())?;
            if serde_json::from_str::<Effect>(&text).ok().as_ref() != Some(effect) {
                return wrong(format!(
                    "an effect does not come back from its JSON: {text}"
                ));
            }
        }
        let text = serde_json::to_string(&scene).map_err(|error| error.to_string())?;
        if serde_json::from_str::<Scene>(&text).ok().as_ref() != Some(&scene) {
            return wrong("the scene does not come back from its JSON".to_string());
        }
        let told = ZonesSnapshot::of(core, now);
        let text = serde_json::to_string(&told).map_err(|error| error.to_string())?;
        if ZonesSnapshot::from_json(&text).ok().as_ref() != Some(&told) {
            return wrong("what is told to others does not come back from its JSON".to_string());
        }
        let config: ZonesConfig = serde_json::to_string(core.config())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or("the configuration does not come back from its JSON")?;
        if &config != core.config() || !config.problems().is_empty() {
            return wrong(format!(
                "the configuration came to be one that cannot be right: {:?}",
                config.problems()
            ));
        }
        // A program started anew from what was written has the zones
        // where they are, put away if they were, and nothing of the moment.
        if number % 8 == 0 {
            let (state, _) = store::decode(&written).map_err(|error| format!("{error:?}"))?;
            let mut again = ZonesCore::new(config, state);
            again.event(
                BackendEvent::Workspace {
                    name: core.workspace().map(str::to_string),
                },
                now,
            );
            again.event(
                BackendEvent::Displays {
                    displays: core.displays().to_vec(),
                    reserved: Vec::new(),
                },
                now,
            );
            let anew = again.scene(now);
            let kept =
                anew.quick_hidden == scene.quick_hidden && anew.grey_icons == scene.grey_icons;
            if !kept || anew.focus.is_some() || anew.masked {
                return wrong(
                    "started anew, what is kept is not kept or what is not is".to_string(),
                );
            }
            for (was, is) in scene.zones.iter().zip(&anew.zones) {
                let whole = |shown: &crate::core::ZoneScene| {
                    (shown.zone.clone(), shown.display.clone(), shown.rolled_up)
                };
                if whole(was) != whole(is) || is.expanded || is.filter.is_some() {
                    return wrong(format!("started anew, {} is shown otherwise", was.zone));
                }
                if !was.expanded && was.rect != is.rect {
                    return wrong(format!(
                        "started anew, {} is at {:?}, not {:?}",
                        was.zone, is.rect, was.rect
                    ));
                }
                if is
                    .portal
                    .as_ref()
                    .is_some_and(|portal| portal.directory != portal.root)
                {
                    return wrong(format!(
                        "started anew, {} does not stand in its own folder",
                        is.zone
                    ));
                }
            }
        }
    }
    // At the end everything that is under way ends, and nothing is left
    // of it.
    world.backend.finish_operations();
    let effects = world.pump();
    world.hear(&effects);
    if !world.begun.is_empty() || !world.core.scene(world.now).operations.is_empty() {
        return Err(format!("at the end, {:?} never ended", world.begun));
    }
    Ok(())
}

#[test]
fn long_runs_of_everything_that_came_later_hold_what_must_hold() {
    many(36, later_run);
    if asked().1.is_none() {
        let seen = [
            ("navigated", &SEEN.navigated),
            ("mirrored", &SEEN.mirrored),
            ("opened for now", &SEEN.opened_for_now),
            ("keyboard moved", &SEEN.keyboard_moved),
            ("layouts replaced", &SEEN.layouts_replaced),
            ("covered", &SEEN.covered),
            ("dry runs", &SEEN.dry_runs),
            ("timed", &SEEN.timed),
            ("tabs", &SEEN.tabs),
            ("put away", &SEEN.put_away),
            ("sources", &SEEN.sources),
            ("not saved", &SEEN.not_saved),
            ("room kept", &SEEN.room_kept),
            ("unavailable", &SEEN.unavailable),
            ("on the desk", &SEEN.on_the_desk),
            ("begun", &SEEN.begun),
            ("ended", &SEEN.ended),
            ("called off", &SEEN.called_off),
            ("failed later", &SEEN.failed_later),
            ("started anew", &SEEN.started_anew),
            ("taken back", &SEEN.taken_back),
            ("made by type", &SEEN.made_by_type),
            ("unmade", &SEEN.unmade),
            ("grey", &SEEN.grey),
        ]
        .map(|(what, counter)| (what, counted(counter)));
        assert!(
            seen.iter().all(|(_, times)| *times >= 20),
            "the runs saw too little: {seen:?}"
        );
    }
}
