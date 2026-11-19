//! The core at work on the backend in memory: what a person does with
//! zones, one thing after another, and what must have come of it.

use crate::backend::{BackendError, BackendEvent, BackendRequest, FileOperation, Location, ZonesBackend};
use crate::config::{Matcher, Rule, RuleAction, Sort, View, ZoneDefinition, ZoneKind, ZonesConfig};
use crate::core::{Command, Effect, ZonesCore, ZonesEvent};
use crate::geometry::{Display, Edge, ReservedArea};
use crate::host::Host;
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::item::Item;
use crate::placement::Cell;
use crate::rules::Scope;
use crate::snap::Edges;
use crate::state::ZonesState;
use crate::store::{LoadReport, Store};
use crate::testing::fake_backend::{FakeBackend, DESKTOP};
use gevrek_model::geometry::Rect;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 24 * 60 * 60;

/// A core, a backend and what carries between them.
struct Desk {
    core: ZonesCore,
    backend: FakeBackend,
    host: Host,
}

impl Desk {
    fn new() -> Self {
        Self::with(ZonesConfig::default(), ZonesState::default())
    }

    fn with(config: ZonesConfig, state: ZonesState) -> Self {
        Self::on(config, state, FakeBackend::new())
    }

    /// (The zones are switched on: off is looked at by a test of its own.)
    fn on(config: ZonesConfig, state: ZonesState, backend: FakeBackend) -> Self {
        Self::as_configured(ZonesConfig { enabled: true, ..config }, state, backend)
    }

    fn as_configured(config: ZonesConfig, state: ZonesState, backend: FakeBackend) -> Self {
        let mut desk = Self { core: ZonesCore::new(config, state), backend, host: Host::new() };
        desk.host.start(&mut desk.core, &mut desk.backend, NOW);
        assert_eq!(desk.core.broken(), None);
        desk
    }

    fn ask(&mut self, command: Command) -> Vec<Effect> {
        let effects = self.host.command(&mut self.core, &mut self.backend, command, NOW);
        assert_eq!(self.core.broken(), None);
        effects
    }

    /// The command is carried out and not refused.
    fn does(&mut self, command: Command) -> Vec<Effect> {
        let effects = self.ask(command.clone());
        assert_eq!(refusal(&effects), None, "{command:?}");
        effects
    }

    fn pump(&mut self) -> Vec<Effect> {
        let effects = self.host.pump(&mut self.core, &mut self.backend, NOW);
        assert_eq!(self.core.broken(), None);
        effects
    }

    /// Make a zone at a rectangle and say what it is called.
    fn zone(&mut self, title: &str, rect: Rect) -> ZoneId {
        let effects = self.does(Command::CreateZone {
            title: title.to_string(),
            kind: ZoneKind::Manual,
            rect: Some(rect),
        });
        events(&effects)
            .into_iter()
            .find_map(|event| match event {
                ZonesEvent::ZoneCreated { zone } => Some(zone),
                _ => None,
            })
            .expect("a zone was made")
    }

    fn file(&mut self, name: &str) -> ItemId {
        let item = self.backend.add_file(name);
        self.pump();
        item.id
    }

    fn put(&mut self, zone: &ZoneId, item: &ItemId) {
        self.does(Command::AddItem { zone: zone.clone(), item: item.clone(), index: None, cell: None });
    }

    fn rect(&self, zone: &ZoneId) -> Rect {
        self.core.zone_rect(zone).expect("the zone has a place")
    }

    fn names_in(&self, zone: &ZoneId) -> Vec<String> {
        self.core.zone_items(zone, NOW).into_iter().map(|item| item.name.clone()).collect()
    }
}

fn events(effects: &[Effect]) -> Vec<ZonesEvent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn refusal(effects: &[Effect]) -> Option<String> {
    events(effects).into_iter().find_map(|event| match event {
        ZonesEvent::Refused { reason } => Some(reason),
        _ => None,
    })
}

fn second() -> Display {
    Display::new("fake-2", Rect::new(1920, 0, 2560, 1440))
}

#[test]
fn a_zone_is_made_filled_and_shown() {
    let mut desk = Desk::new();
    let a = desk.file("a.txt");
    let b = desk.file("b.txt");
    let effects = desk.ask(Command::CreateZone {
        title: "Papers".to_string(),
        kind: ZoneKind::Manual,
        rect: Some(Rect::new(100, 100, 400, 300)),
    });
    assert!(effects.contains(&Effect::SaveConfig) && effects.contains(&Effect::SaveState));
    let zone = desk.core.config().zones[0].id.clone();
    assert_eq!(desk.rect(&zone), Rect::new(100, 100, 400, 300));
    desk.put(&zone, &b);
    desk.put(&zone, &a);
    assert_eq!(desk.names_in(&zone), ["b.txt", "a.txt"], "the order they were put in");

    let scene = desk.core.scene(NOW);
    assert_eq!(scene.zones.len(), 1);
    assert_eq!(scene.zones[0].items.items.len(), 2);
    assert!(scene.zones[0].shown && scene.zones[0].front);
    assert!(scene.loose.is_empty(), "nothing is left on the bare desktop");

    // Sorted by name the order is another, and what was put by hand is kept.
    desk.does(Command::SetView {
        zone: zone.clone(),
        view: Some(View::List),
        sort: Some(Sort::Name),
        descending: None,
        icon_size: None,
    });
    assert_eq!(desk.names_in(&zone), ["a.txt", "b.txt"]);
    assert_eq!(desk.core.state().placements.items_of(&zone), [b.clone(), a.clone()]);
    assert_eq!(desk.core.scene(NOW).zones[0].items.columns, 1);

    // Taken out, an item is on the bare desktop again.
    desk.does(Command::RemoveItem { item: a.clone() });
    let scene = desk.core.scene(NOW);
    assert_eq!(scene.loose.len(), 1);
    assert_eq!(scene.loose[0].items[0].item, a);

    // Without a zone, its items are loose and nothing of it is left.
    desk.does(Command::DeleteZone { zone: zone.clone() });
    assert!(desk.core.config().zones.is_empty());
    assert!(desk.core.state().placements.zones.is_empty());
    assert_eq!(desk.core.scene(NOW).loose[0].items.len(), 2);
    assert!(refusal(&desk.ask(Command::DeleteZone { zone })).is_some());
}

#[test]
fn zones_that_are_switched_off_show_nothing_and_ask_for_nothing() {
    // A configuration with zones, a portal and rules; and off.
    let mut backend = FakeBackend::new();
    backend.add_folder("/downloads");
    backend.add("/downloads", Item::file("/downloads/setup.exe"));
    backend.add_file("cat.png");
    let mut config = sorting_config();
    config.zones.push(ZoneDefinition {
        kind: ZoneKind::Portal { directory: "/downloads".to_string() },
        ..ZoneDefinition::manual("portal", "Downloads")
    });
    assert!(!config.enabled, "off is what a configuration is by default");
    let mut desk = Desk::as_configured(config.clone(), ZonesState::default(), backend);
    assert_eq!(desk.backend.watches(), 0, "nothing is watched");
    assert_eq!(desk.backend.calls, 0, "the backend was asked nothing");
    assert_eq!(desk.core.scene(NOW), crate::core::Scene::default());
    assert_eq!(desk.core.start(), []);
    assert!(!desk.core.is_shown(&ZoneId::new("pictures")));
    // What happens on the desktop is not heard, and no command is carried out.
    desk.backend.add_file("dog.png");
    assert_eq!(desk.pump(), []);
    let state = desk.core.state().clone();
    for command in [
        Command::CreateZone { title: "Z".to_string(), kind: ZoneKind::Manual, rect: None },
        Command::ApplyRules { scope: Scope::All },
        Command::QuickHide { hidden: None },
        Command::Search { typed: "cat".to_string() },
    ] {
        let effects = desk.ask(command.clone());
        assert!(refusal(&effects).is_some_and(|reason| reason.contains("desktop.zones.enabled")), "{command:?}");
        assert_eq!(effects.len(), 1);
    }
    assert_eq!((desk.core.state(), desk.core.config()), (&state, &config));
    assert_eq!(desk.backend.calls, 0);
    // Told of an item all the same (by a host that does not know better),
    // the core sorts nothing by itself.
    let stray = BackendEvent::ItemAdded { location: Location::Desktop, item: Item::file("/desk/x.png") };
    desk.core.event(stray, NOW);
    assert_eq!(desk.core.state(), &state);

    // Switched on, everything is there as if it had been on at the start.
    let on = ZonesConfig { enabled: true, ..config.clone() };
    desk.host.set_config(&mut desk.core, &mut desk.backend, on, NOW);
    assert_eq!(desk.backend.watches(), 2, "the desktop and the portal's folder");
    let scene = desk.core.scene(NOW);
    assert_eq!(scene.zones.len(), 3);
    assert_eq!(scene.loose[0].items.len(), 2);
    assert_eq!(desk.names_in(&ZoneId::new("portal")), ["setup.exe"]);
    desk.does(Command::ApplyRules { scope: Scope::Loose });
    assert_eq!(desk.names_in(&ZoneId::new("pictures")), ["cat.png", "dog.png"]);
    // And off again: nothing is shown, nothing is watched, and what was
    // sorted is kept for the next time.
    let placements = desk.core.state().placements.clone();
    let effects = desk.host.set_config(&mut desk.core, &mut desk.backend, config, NOW);
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::Ask(_))));
    assert_eq!(desk.backend.watches(), 0);
    assert_eq!(desk.core.scene(NOW), crate::core::Scene::default());
    assert_eq!(desk.core.state().placements, placements);
    let calls = desk.backend.calls;
    desk.backend.add_file("late.png");
    assert_eq!(desk.pump(), []);
    assert_eq!(desk.backend.calls, calls);
}

#[test]
fn a_zone_made_without_a_place_stands_beside_the_others() {
    let mut desk = Desk::new();
    let make = Command::CreateZone { title: "Z".to_string(), kind: ZoneKind::Manual, rect: None };
    desk.does(make.clone());
    desk.does(make.clone());
    let zones = desk.core.config().zones.clone();
    assert_ne!(zones[0].id, zones[1].id);
    assert_eq!(desk.rect(&zones[0].id), Rect::new(40, 40, 320, 240));
    assert_eq!(desk.rect(&zones[1].id), Rect::new(72, 72, 320, 240));
    // A rectangle off every display is brought to where it can be reached.
    let far = desk.zone("Far", Rect::new(90_000, -500, 300, 200));
    assert_eq!(desk.rect(&far), Rect::new(1620, 0, 300, 200));
    // Too small to hold on to: made the least a zone is.
    let tiny = desk.zone("Tiny", Rect::new(600, 600, 2, 2));
    assert_eq!(desk.rect(&tiny), Rect::new(600, 600, 120, 28));
}

#[test]
fn a_moved_zone_snaps_and_a_locked_one_stays() {
    let mut desk = Desk::new();
    desk.backend.set_reserved(vec![ReservedArea {
        display: DisplayId::new("fake-1"),
        edge: Edge::Top,
        thickness: 40,
    }]);
    desk.pump();
    let left = desk.zone("Left", Rect::new(100, 200, 300, 200));
    let right = desk.zone("Right", Rect::new(900, 600, 300, 200));

    // Near the other zone: beside it with the gap, in a line with its top.
    let effects = desk.does(Command::MoveZone { zone: right.clone(), x: 405, y: 204, snap: true });
    assert_eq!(desk.rect(&right), Rect::new(408, 200, 300, 200));
    assert!(events(&effects).contains(&ZonesEvent::ZonePlaced {
        zone: right.clone(),
        rect: Rect::new(408, 200, 300, 200)
    }));
    // Near the panel: below it, not under it.
    desk.does(Command::MoveZone { zone: right.clone(), x: 1000, y: 35, snap: true });
    assert_eq!(desk.rect(&right), Rect::new(1000, 40, 300, 200));
    // Without snapping it goes where it is put.
    desk.does(Command::MoveZone { zone: right.clone(), x: 1003, y: 47, snap: false });
    assert_eq!(desk.rect(&right), Rect::new(1003, 47, 300, 200));
    // Sized by its right side up to the display's.
    desk.does(Command::ResizeZone {
        zone: right.clone(),
        rect: Rect::new(1003, 47, 910, 200),
        edges: Edges { right: true, ..Edges::default() },
        snap: true,
    });
    assert_eq!(desk.rect(&right), Rect::new(1003, 47, 917, 200));

    desk.does(Command::Lock { zone: left.clone(), geometry: Some(true), members: None });
    let before = desk.core.state().clone();
    assert!(refusal(&desk.ask(Command::MoveZone { zone: left.clone(), x: 0, y: 0, snap: true })).is_some());
    assert!(refusal(&desk.ask(Command::ResizeZone {
        zone: left.clone(),
        rect: Rect::new(0, 0, 500, 500),
        edges: Edges::BOTTOM_RIGHT,
        snap: true
    }))
    .is_some());
    assert_eq!(desk.core.state(), &before, "a refused command changes nothing");
    assert_eq!(desk.rect(&left), Rect::new(100, 200, 300, 200));
}

#[test]
fn a_locked_zone_lets_nothing_in_or_out() {
    let mut desk = Desk::new();
    let inside = desk.file("inside.txt");
    let outside = desk.file("outside.txt");
    let locked = desk.zone("Locked", Rect::new(100, 100, 300, 200));
    let other = desk.zone("Other", Rect::new(500, 100, 300, 200));
    desk.put(&locked, &inside);
    desk.does(Command::Lock { zone: locked.clone(), geometry: None, members: Some(true) });
    let add = |zone: &ZoneId, item: &ItemId| Command::AddItem {
        zone: zone.clone(),
        item: item.clone(),
        index: None,
        cell: None,
    };
    assert!(refusal(&desk.ask(add(&locked, &outside))).is_some());
    assert!(refusal(&desk.ask(add(&other, &inside))).is_some());
    assert!(refusal(&desk.ask(Command::RemoveItem { item: inside.clone() })).is_some());
    assert_eq!(desk.names_in(&locked), ["inside.txt"]);
    // What is not on the desktop cannot be put anywhere.
    assert!(refusal(&desk.ask(add(&other, &ItemId::new("/desk/ghost.txt")))).is_some());
    assert!(refusal(&desk.ask(add(&ZoneId::new("nowhere"), &outside))).is_some());
}

#[test]
fn tabs_share_one_place_and_move_together() {
    let mut desk = Desk::new();
    let one = desk.zone("One", Rect::new(100, 100, 300, 200));
    let two = desk.zone("Two", Rect::new(800, 500, 400, 300));
    let three = desk.zone("Three", Rect::new(1300, 100, 200, 200));
    desk.does(Command::Tab { zone: two.clone(), with: one.clone() });
    assert_eq!(desk.rect(&two), desk.rect(&one));
    assert_eq!(desk.rect(&one), Rect::new(100, 100, 300, 200));
    let front = |desk: &Desk| -> Vec<bool> {
        desk.core.scene(NOW).zones.iter().map(|zone| zone.front).collect()
    };
    assert_eq!(front(&desk), [false, true, true], "the tab that joined is in front");
    desk.does(Command::ActivateTab { zone: one.clone() });
    assert_eq!(front(&desk), [true, false, true]);

    desk.does(Command::MoveZone { zone: two.clone(), x: 600, y: 300, snap: false });
    assert_eq!(desk.rect(&one), Rect::new(600, 300, 300, 200));
    assert_eq!(desk.rect(&two), Rect::new(600, 300, 300, 200));
    // They roll up together.
    desk.does(Command::RollUp { zone: one.clone(), rolled_up: None });
    assert_eq!(desk.rect(&two), Rect::new(600, 300, 300, 28));
    desk.does(Command::RollUp { zone: two.clone(), rolled_up: Some(false) });
    assert_eq!(desk.rect(&one), Rect::new(600, 300, 300, 200));

    // A third joins, then one leaves: it stands a step beside the others.
    desk.does(Command::Tab { zone: three.clone(), with: two.clone() });
    assert_eq!(desk.core.scene(NOW).zones.iter().filter(|zone| zone.tab_group.is_some()).count(), 3);
    desk.does(Command::Untab { zone: one.clone() });
    assert_eq!(desk.rect(&one), Rect::new(624, 324, 300, 200));
    assert_eq!(desk.rect(&two), Rect::new(600, 300, 300, 200));
    assert_eq!(desk.rect(&three), desk.rect(&two));
    // Two were left; one more leaves and the last is no tab either.
    desk.does(Command::Untab { zone: three.clone() });
    assert!(desk.core.config().zones.iter().all(|zone| zone.tab_group.is_none()));
    assert!(desk.core.state().active_tabs.is_empty());
    assert!(refusal(&desk.ask(Command::Untab { zone: two.clone() })).is_some());
    assert!(refusal(&desk.ask(Command::Tab { zone: two.clone(), with: two })).is_some());
}

#[test]
fn zones_are_hidden_with_one_key_and_peeked_at_with_another() {
    let mut desk = Desk::new();
    let plain = desk.zone("Plain", Rect::new(100, 100, 300, 200));
    let pinned = desk.zone("Pinned", Rect::new(500, 100, 300, 200));
    let hidden = desk.zone("Hidden", Rect::new(900, 100, 300, 200));
    desk.core.set_config({
        let mut config = desk.core.config().clone();
        config.zone_mut(&pinned).unwrap().always_visible = true;
        config
    });
    desk.does(Command::ShowZone { zone: hidden.clone(), shown: false });
    let shown = |desk: &Desk| -> Vec<bool> {
        [&plain, &pinned, &hidden].iter().map(|zone| desk.core.is_shown(zone)).collect()
    };
    assert_eq!(shown(&desk), [true, true, false]);
    let effects = desk.does(Command::QuickHide { hidden: None });
    assert_eq!(
        events(&effects),
        [ZonesEvent::VisibilityChanged { quick_hidden: true, peeking: false }]
    );
    assert_eq!(shown(&desk), [false, true, false]);
    // A peek shows what a key hid, not what the person put away.
    desk.does(Command::Peek { on: true });
    assert_eq!(shown(&desk), [true, true, false]);
    assert!(desk.core.scene(NOW).peeking);
    desk.does(Command::Peek { on: false });
    assert_eq!(shown(&desk), [false, true, false]);
    desk.does(Command::QuickHide { hidden: None });
    desk.does(Command::ShowZone { zone: hidden.clone(), shown: true });
    assert_eq!(shown(&desk), [true, true, true]);
}

fn sorting_config() -> ZonesConfig {
    ZonesConfig {
        zones: vec![ZoneDefinition::manual("pictures", "Pictures"), ZoneDefinition::manual("papers", "Papers")],
        rules: vec![
            Rule {
                name: "pictures".to_string(),
                matchers: vec![Matcher::Extension { any_of: vec!["png".to_string()] }],
                action: RuleAction::AssignToZone { zone: ZoneId::new("pictures") },
                ..Rule::default()
            },
            Rule {
                name: "old papers".to_string(),
                matchers: vec![
                    Matcher::Extension { any_of: vec!["doc".to_string()] },
                    Matcher::Untouched { days: 90 },
                ],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
            Rule {
                name: "papers".to_string(),
                matchers: vec![Matcher::Extension { any_of: vec!["doc".to_string()] }],
                action: RuleAction::AssignToZone { zone: ZoneId::new("papers") },
                ..Rule::default()
            },
        ],
        ..ZonesConfig::default()
    }
}

/// A desktop with an old paper, a new one and a picture, before the
/// zones start.
fn sorting_desk(rules_on_arrival: bool) -> Desk {
    let mut backend = FakeBackend::new();
    backend.add_folder("/archive");
    for (name, age) in [("old.doc", 200), ("new.doc", 1), ("cat.png", 5)] {
        let mut item = Item::file(&format!("{DESKTOP}/{name}"));
        item.facts.modified = Some(NOW - age * DAY);
        backend.add(DESKTOP, item);
    }
    let config = ZonesConfig { rules_on_arrival, ..sorting_config() };
    Desk::on(config, ZonesState::default(), backend)
}

#[test]
fn rules_are_previewed_applied_and_taken_back_files_and_all() {
    let mut desk = sorting_desk(false);
    let before = desk.core.state().placements.clone();
    let on_desk = desk.backend.in_folder(DESKTOP);

    let effects = desk.does(Command::PreviewRules { scope: Scope::Loose });
    let Some(ZonesEvent::RulesPreviewed { plan }) = events(&effects).into_iter().next() else {
        panic!("{effects:?}");
    };
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(desk.core.state().placements, before, "a preview changed something");
    assert_eq!(desk.backend.performed, []);

    let effects = desk.does(Command::ApplyRules { scope: Scope::Loose });
    assert!(events(&effects).contains(&ZonesEvent::RulesApplied { plan }), "what was previewed is what is done");
    assert_eq!(desk.names_in(&ZoneId::new("pictures")), ["cat.png"]);
    assert_eq!(desk.names_in(&ZoneId::new("papers")), ["new.doc"]);
    assert_eq!(
        desk.backend.performed,
        [FileOperation::Move { from: "/desk/old.doc".to_string(), to_folder: "/archive".to_string() }]
    );
    assert_eq!(desk.backend.in_folder("/archive").len(), 1);
    assert_eq!(desk.core.scene(NOW).loose.len(), 0, "the old paper left the desktop");
    // Once more: there is nothing left to do, and what can be taken back stays.
    let again = desk.does(Command::ApplyRules { scope: Scope::Loose });
    assert!(matches!(&events(&again)[0], ZonesEvent::RulesApplied { plan } if plan.steps.is_empty()));
    assert!(desk.core.state().undo.is_some());

    desk.does(Command::UndoRules);
    assert_eq!(desk.core.state().placements, before);
    assert_eq!(desk.backend.in_folder(DESKTOP), on_desk);
    assert!(desk.backend.in_folder("/archive").is_empty());
    assert!(refusal(&desk.ask(Command::UndoRules)).is_some(), "only once");
}

#[test]
fn a_file_that_could_not_be_moved_stays_where_it_was_shown() {
    let mut desk = sorting_desk(false);
    let old = ItemId::new("/desk/old.doc");
    desk.put(&ZoneId::new("papers"), &old);
    let before = desk.core.state().placements.clone();
    desk.backend.fail_next = Some(BackendError::Refused { what: "in use".to_string() });
    let effects = desk.does(Command::ApplyRules { scope: Scope::All });
    assert!(events(&effects).iter().any(|event| matches!(event, ZonesEvent::OperationFailed { .. })));
    assert_eq!(desk.core.zone_of(&old), Some(&ZoneId::new("papers")));
    assert_eq!(desk.names_in(&ZoneId::new("papers")), ["old.doc", "new.doc"]);
    // Taking back undoes the rest, and asks for no file to be moved.
    let performed = desk.backend.performed.len();
    desk.does(Command::UndoRules);
    assert_eq!(desk.core.state().placements, before);
    assert_eq!(desk.backend.performed.len(), performed);
}

#[test]
fn the_way_back_is_the_one_the_backend_told() {
    let mut desk = sorting_desk(false);
    // One of that name is in the archive already: the moved file gets another.
    desk.backend.add("/archive", Item::file("/archive/old.doc"));
    desk.does(Command::ApplyRules { scope: Scope::Loose });
    let record = desk.core.state().undo.clone().unwrap();
    assert_eq!(
        record.moved[0].undo,
        FileOperation::Move { from: "/archive/old (2).doc".to_string(), to_folder: DESKTOP.to_string() }
    );
    desk.does(Command::UndoRules);
    let names: Vec<String> = desk.backend.in_folder("/archive").into_iter().map(|item| item.name).collect();
    assert_eq!(names, ["old.doc"], "the one that was there stays");
    assert_eq!(desk.backend.in_folder(DESKTOP).len(), 3);
}

#[test]
fn an_item_that_comes_goes_to_its_zone_but_no_file_is_moved_unasked() {
    let mut desk = sorting_desk(true);
    // What was there at the start is left as the person left it.
    assert!(desk.core.state().placements.zones.is_empty());
    let dog = desk.file("dog.png");
    assert_eq!(desk.core.zone_of(&dog), Some(&ZoneId::new("pictures")));
    let mut ancient = Item::file("/desk/ancient.doc");
    ancient.facts.modified = Some(NOW - 400 * DAY);
    desk.backend.add(DESKTOP, ancient.clone());
    desk.pump();
    assert_eq!(desk.core.zone_of(&ancient.id), None);
    assert_eq!(desk.backend.performed, []);
    // Switched off, nothing is sorted by itself.
    let mut desk = sorting_desk(false);
    let dog = desk.file("dog.png");
    assert_eq!(desk.core.zone_of(&dog), None);
}

#[test]
fn an_item_that_is_renamed_keeps_its_place_and_one_that_goes_leaves_none() {
    let mut desk = Desk::new();
    let zone = desk.zone("Z", Rect::new(100, 100, 300, 200));
    for name in ["a.txt", "b.txt", "c.txt"] {
        let item = desk.file(name);
        desk.put(&zone, &item);
    }
    let path = "/desk/b.txt".to_string();
    desk.backend.perform(&FileOperation::Rename { path, new_name: "B.md".to_string() }).unwrap();
    desk.pump();
    assert_eq!(desk.names_in(&zone), ["a.txt", "B.md", "c.txt"]);
    assert!(desk.backend.remove(&ItemId::new("/desk/a.txt")));
    let effects = desk.pump();
    assert!(events(&effects).contains(&ZonesEvent::ItemsChanged { zone: Some(zone.clone()) }));
    assert_eq!(desk.names_in(&zone), ["B.md", "c.txt"]);
    assert_eq!(desk.core.state().placements.items_of(&zone).len(), 2);
}

#[test]
fn an_item_put_on_a_cell_is_drawn_there() {
    let mut desk = Desk::new();
    let zone = desk.zone("Z", Rect::new(100, 100, 400, 300));
    let a = desk.file("a.txt");
    let b = desk.file("b.txt");
    desk.put(&zone, &a);
    desk.does(Command::AddItem {
        zone: zone.clone(),
        item: b.clone(),
        index: None,
        cell: Some(Cell { column: 2, row: 1 }),
    });
    let scene = desk.core.scene(NOW);
    let cells: Vec<Cell> = scene.zones[0].items.items.iter().map(|placed| placed.cell).collect();
    assert_eq!(cells, [Cell { column: 0, row: 0 }, Cell { column: 2, row: 1 }]);
    // A loose item put somewhere by hand is there.
    desk.does(Command::PlaceLooseItem { item: a.clone(), display: DisplayId::new("fake-1"), x: 700, y: 500 });
    let scene = desk.core.scene(NOW);
    assert_eq!((scene.loose[0].items[0].rect.x, scene.loose[0].items[0].rect.y), (700, 500));
    assert!(refusal(&desk.ask(Command::PlaceLooseItem {
        item: a,
        display: DisplayId::new("nowhere"),
        x: 0,
        y: 0
    }))
    .is_some());
}

#[test]
fn a_portal_shows_a_folder_and_what_is_dropped_on_it_is_moved_there() {
    let mut backend = FakeBackend::new();
    backend.add_folder("/downloads");
    backend.add("/downloads", Item::file("/downloads/setup.exe"));
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), backend);
    let report = desk.file("report.pdf");
    let effects = desk.does(Command::CreateZone {
        title: "Downloads".to_string(),
        kind: ZoneKind::Portal { directory: "/downloads".to_string() },
        rect: None,
    });
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::Ask(_))), "the host carried the asking out");
    let portal = desk.core.config().zones[0].id.clone();
    assert_eq!(desk.names_in(&portal), ["setup.exe"]);

    // Something comes into the folder from outside.
    desk.backend.add("/downloads", Item::file("/downloads/movie.mkv"));
    let effects = desk.pump();
    assert_eq!(events(&effects), [ZonesEvent::ItemsChanged { zone: Some(portal.clone()) }]);
    assert_eq!(desk.names_in(&portal), ["movie.mkv", "setup.exe"]);

    // Dropped on the portal, the file itself moves, and can be brought back.
    desk.put(&portal, &report);
    assert_eq!(desk.names_in(&portal), ["movie.mkv", "report.pdf", "setup.exe"]);
    assert!(desk.backend.in_folder(DESKTOP).is_empty());
    assert!(desk.core.scene(NOW).loose.is_empty());
    assert!(refusal(&desk.ask(Command::AddItem {
        zone: portal.clone(),
        item: ItemId::new("/downloads/report.pdf"),
        index: None,
        cell: None
    }))
    .is_some());
    desk.does(Command::UndoFileOperation);
    assert_eq!(desk.names_in(&portal), ["movie.mkv", "setup.exe"]);
    assert_eq!(desk.backend.in_folder(DESKTOP).len(), 1);
    assert!(refusal(&desk.ask(Command::UndoFileOperation)).is_some());

    // Found by name, wherever it is shown.
    let hits = desk.core.search("set");
    assert_eq!((hits[0].name.as_str(), hits[0].zone.as_ref()), ("setup.exe", Some(&portal)));
    assert_eq!(desk.core.search("report")[0].zone, None);

    // Without the portal the folder is no longer watched.
    desk.does(Command::DeleteZone { zone: portal });
    desk.backend.add("/downloads", Item::file("/downloads/late.txt"));
    assert_eq!(desk.pump(), []);
}

#[test]
fn files_go_to_the_bin_and_come_from_elsewhere_and_both_can_be_taken_back() {
    let mut backend = FakeBackend::new();
    backend.add_folder("/elsewhere");
    backend.add("/elsewhere", Item::file("/elsewhere/new.txt"));
    backend.add("/elsewhere", Item::file("/elsewhere/other.txt"));
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), backend);
    let zone = desk.zone("Z", Rect::new(100, 100, 300, 200));
    let old = desk.file("old.txt");
    desk.put(&zone, &old);

    // Into the bin: gone from its zone; taken back, it is on the desktop again.
    desk.does(Command::TrashItems { items: vec![old.clone()] });
    assert!(desk.backend.in_folder(DESKTOP).is_empty());
    assert_eq!(desk.names_in(&zone), Vec::<String>::new());
    desk.does(Command::UndoFileOperation);
    assert_eq!(desk.backend.in_folder(DESKTOP).len(), 1);
    assert!(refusal(&desk.ask(Command::TrashItems { items: vec![ItemId::new("/desk/ghost")] })).is_some());

    // Another name, and back; it keeps its place in the zone.
    desk.put(&zone, &old);
    desk.does(Command::RenameItem { item: old.clone(), name: " older.txt ".to_string() });
    assert_eq!(desk.names_in(&zone), ["older.txt"]);
    assert!(refusal(&desk.ask(Command::RenameItem { item: ItemId::new("/desk/older.txt"), name: "a/b".to_string() })).is_some());
    desk.does(Command::UndoFileOperation);
    assert_eq!(desk.names_in(&zone), ["old.txt"]);
    desk.does(Command::RemoveItem { item: old.clone() });

    // A file from elsewhere, copied to the desktop and into the zone.
    let folder = desk.backend.desktop_folder().unwrap();
    desk.does(Command::BringIn {
        paths: vec!["/elsewhere/new.txt".to_string()],
        folder: folder.clone(),
        copy: true,
        zone: Some(zone.clone()),
        at: None,
    });
    assert_eq!(desk.names_in(&zone), ["new.txt"]);
    assert_eq!(desk.backend.in_folder("/elsewhere").len(), 2, "copied: the original stays");
    // Another, moved to a place of its own on the bare desktop.
    let at = crate::placement::LoosePosition { display: DisplayId::new("fake-1"), x: 700, y: 500 };
    desk.does(Command::BringIn {
        paths: vec!["/elsewhere/other.txt".to_string()],
        folder,
        copy: false,
        zone: None,
        at: Some(at.clone()),
    });
    assert_eq!(desk.backend.in_folder("/elsewhere").len(), 1);
    assert_eq!(desk.core.state().placements.loose.get(&ItemId::new("/desk/other.txt")), Some(&at));
    // Taken back: it is where it came from.
    desk.does(Command::UndoFileOperation);
    assert_eq!(desk.backend.in_folder("/elsewhere").len(), 2);
    assert!(desk.core.item(&ItemId::new("/desk/other.txt")).is_none());
    assert!(refusal(&desk.ask(Command::BringIn {
        paths: vec!["/elsewhere/new.txt".to_string()],
        folder: "/desk".to_string(),
        copy: true,
        zone: Some(ZoneId::new("nowhere")),
        at: None
    }))
    .is_some());
}

#[test]
fn a_portal_onto_a_folder_that_is_not_there_is_an_empty_zone() {
    let mut desk = Desk::new();
    desk.does(Command::CreateZone {
        title: "Stick".to_string(),
        kind: ZoneKind::Portal { directory: "/media/stick".to_string() },
        rect: None,
    });
    let portal = desk.core.config().zones[0].id.clone();
    assert_eq!(desk.names_in(&portal), Vec::<String>::new());
    assert_eq!(desk.core.scene(NOW).zones.len(), 1);
}

#[test]
fn a_smart_zone_shows_what_fits_without_taking_it() {
    let mut desk = Desk::new();
    let manual = desk.zone("Manual", Rect::new(100, 100, 300, 200));
    desk.does(Command::CreateZone {
        title: "Pictures".to_string(),
        kind: ZoneKind::Smart { query: vec![Matcher::Extension { any_of: vec!["png".to_string()] }] },
        rect: None,
    });
    let smart = desk.core.config().zones[1].id.clone();
    let cat = desk.file("cat.png");
    desk.file("notes.txt");
    desk.put(&manual, &cat);
    assert_eq!(desk.names_in(&smart), ["cat.png"]);
    assert_eq!(desk.core.zone_of(&cat), Some(&manual), "it stays where it was put");
    assert!(refusal(&desk.ask(Command::AddItem { zone: smart, item: cat, index: None, cell: None })).is_some());
}

#[test]
fn a_snapshot_brings_back_zones_rules_and_places() {
    let mut desk = sorting_desk(false);
    let pictures = ZoneId::new("pictures");
    let cat = ItemId::new("/desk/cat.png");
    desk.put(&pictures, &cat);
    desk.does(Command::MoveZone { zone: pictures.clone(), x: 500, y: 400, snap: false });
    desk.does(Command::SaveSnapshot { name: "tidy".to_string() });
    let config = desk.core.config().clone();
    let placements = desk.core.state().placements.clone();

    desk.does(Command::DeleteZone { zone: pictures.clone() });
    let extra = desk.zone("Extra", Rect::new(0, 0, 200, 200));
    desk.does(Command::MoveZone { zone: ZoneId::new("papers"), x: 900, y: 700, snap: false });
    assert_ne!(desk.core.config(), &config);

    let effects = desk.does(Command::RestoreSnapshot { name: "tidy".to_string() });
    assert!(effects.contains(&Effect::SaveConfig) && effects.contains(&Effect::SaveState));
    assert_eq!(desk.core.config(), &config);
    assert_eq!(desk.core.state().placements, placements);
    assert_eq!(desk.rect(&pictures), Rect::new(500, 400, 320, 240));
    assert_eq!(desk.core.zone_rect(&extra), None);
    // The snapshot is still there, and a zone made now gets a name of its own.
    let next = desk.zone("Next", Rect::new(0, 0, 200, 200));
    assert_ne!(next, extra);
    assert!(refusal(&desk.ask(Command::RestoreSnapshot { name: "none".to_string() })).is_some());
    assert!(refusal(&desk.ask(Command::SaveSnapshot { name: " ".to_string() })).is_some());
    desk.does(Command::DeleteSnapshot { name: "tidy".to_string() });
    assert!(desk.core.state().snapshots.is_empty());
}

#[test]
fn zones_of_a_display_that_goes_come_over_and_go_back_when_it_returns() {
    let mut desk = Desk::new();
    desk.backend.plug(second());
    let effects = desk.pump();
    assert!(events(&effects)
        .iter()
        .any(|event| matches!(event, ZonesEvent::ProfileChanged { made: true, .. })));
    let here = desk.zone("Here", Rect::new(100, 100, 300, 200));
    // In the bottom right corner of the second display, 20 from its sides.
    let there = desk.zone("There", Rect::new(1920 + 2560 - 420, 1440 - 320, 400, 300));
    let item = desk.file("a.txt");
    desk.put(&there, &item);
    let docked = (desk.rect(&here), desk.rect(&there));

    desk.backend.unplug(&DisplayId::new("fake-2"));
    let effects = desk.pump();
    // (An arrangement for the first display alone is there from the
    // start; the zones made since come over from the one that was in use.)
    assert!(events(&effects).contains(&ZonesEvent::ProfileChanged { made: false, moved: vec![there.clone()] }));
    assert_eq!(desk.rect(&here), docked.0);
    // The same corner of the display that is left.
    assert_eq!(desk.rect(&there), Rect::new(1920 - 420, 1080 - 320, 400, 300));
    assert_eq!(desk.names_in(&there), ["a.txt"], "what it holds came with it");
    // Moved while the display is away: that is for this set of displays only.
    desk.does(Command::MoveZone { zone: there.clone(), x: 700, y: 300, snap: false });

    desk.backend.plug(second());
    let effects = desk.pump();
    assert!(events(&effects).contains(&ZonesEvent::ProfileChanged { made: false, moved: vec![] }));
    assert_eq!((desk.rect(&here), desk.rect(&there)), docked);
    // And away again: where it was put the last time it was away.
    desk.backend.unplug(&DisplayId::new("fake-2"));
    desk.pump();
    assert_eq!(desk.rect(&there), Rect::new(700, 300, 400, 300));
}

#[test]
fn a_zone_keeps_its_corner_through_resolution_scale_and_order() {
    let mut desk = Desk::new();
    desk.backend.plug(second());
    desk.pump();
    let corner = desk.zone("Corner", Rect::new(1920 + 2560 - 420, 1440 - 320, 400, 300));
    let stored = desk.core.profile().unwrap().geometry[&corner].clone();

    // The second display at another resolution and scale.
    desk.backend.plug(Display { scale: 2.0, ..Display::new("fake-2", Rect::new(1920, 0, 1920, 1080)) });
    desk.pump();
    assert_eq!(desk.rect(&corner), Rect::new(1920 + 1920 - 420, 1080 - 320, 400, 300));
    // Too small for the zone as it is: shown as large as fits, kept as it was.
    desk.backend.plug(Display::new("fake-2", Rect::new(1920, 0, 320, 200)));
    desk.pump();
    assert_eq!(desk.rect(&corner), Rect::new(1920, 0, 320, 200));
    assert_eq!(desk.core.profile().unwrap().geometry[&corner], stored);
    // As it was at first, but on the other side of the first display
    // and told in the other order: the zone is in its corner there.
    desk.backend.set_displays(vec![
        Display::new("fake-2", Rect::new(-2560, 0, 2560, 1440)),
        Display { primary: true, ..Display::new("fake-1", Rect::new(0, 0, 1920, 1080)) },
    ]);
    let effects = desk.pump();
    assert!(events(&effects).contains(&ZonesEvent::ProfileChanged { made: false, moved: vec![] }));
    assert_eq!(desk.rect(&corner), Rect::new(-420, 1440 - 320, 400, 300));
    // No display at all (the machine sleeps): nothing changes.
    let before = desk.core.state().clone();
    desk.backend.set_displays(Vec::new());
    assert_eq!(desk.pump(), []);
    assert_eq!(desk.core.state(), &before);
    assert_eq!(desk.rect(&corner), Rect::new(-420, 1440 - 320, 400, 300));
}

#[test]
fn each_workspace_has_its_own_arrangement_where_that_is_wanted() {
    let config = ZonesConfig { profile_per_workspace: true, ..ZonesConfig::default() };
    let mut desk = Desk::with(config, ZonesState::default());
    let workspace = |desk: &mut Desk, name: &str| {
        let event = BackendEvent::Workspace { name: Some(name.to_string()) };
        desk.core.event(event, NOW)
    };
    workspace(&mut desk, "1");
    let zone = desk.zone("Z", Rect::new(100, 100, 300, 200));
    workspace(&mut desk, "2");
    assert_eq!(desk.rect(&zone), Rect::new(100, 100, 300, 200), "it starts as it was on the other");
    desk.does(Command::MoveZone { zone: zone.clone(), x: 900, y: 500, snap: false });
    desk.does(Command::ShowZone { zone: zone.clone(), shown: false });
    workspace(&mut desk, "1");
    assert_eq!(desk.rect(&zone), Rect::new(100, 100, 300, 200));
    assert!(desk.core.is_shown(&zone));
    workspace(&mut desk, "2");
    assert_eq!(desk.rect(&zone), Rect::new(900, 500, 300, 200));
    assert!(!desk.core.is_shown(&zone));
    // Not wanted: another workspace changes nothing.
    let mut plain = Desk::new();
    let zone = plain.zone("Z", Rect::new(100, 100, 300, 200));
    assert_eq!(workspace(&mut plain, "2"), []);
    assert_eq!(plain.core.state().profiles.len(), 1);
    assert_eq!(plain.rect(&zone), Rect::new(100, 100, 300, 200));
}

#[test]
fn what_was_saved_is_what_the_next_start_shows() {
    let folder = std::env::temp_dir().join(format!("gevrek-zones-scenario-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let store = Store::new(&folder);
    let mut desk = sorting_desk(false);
    desk.backend.plug(second());
    desk.pump();
    let extra = desk.zone("Extra", Rect::new(2500, 300, 500, 400));
    desk.does(Command::ApplyRules { scope: Scope::Loose });
    desk.does(Command::Tab { zone: ZoneId::new("papers"), with: ZoneId::new("pictures") });
    desk.does(Command::SaveSnapshot { name: "s".to_string() });
    desk.does(Command::RollUp { zone: extra, rolled_up: Some(true) });
    store.save(desk.core.state()).unwrap();
    let scene = desk.core.scene(NOW);

    let loaded = store.load().unwrap();
    assert_eq!(loaded.report, LoadReport::Read);
    assert_eq!(&loaded.state, desk.core.state());
    // The same desktop, another run of the program.
    let config = desk.core.config().clone();
    let again = Desk::on(config, loaded.state, desk.backend);
    assert_eq!(again.core.scene(NOW), scene);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn commands_events_and_effects_travel_as_json() {
    let command: Command = serde_json::from_str(r#"{"command": "move_zone", "zone": "zone-1", "x": 10, "y": 20}"#).unwrap();
    assert_eq!(command, Command::MoveZone { zone: ZoneId::new("zone-1"), x: 10, y: 20, snap: true });
    let command: Command = serde_json::from_str(r#"{"command": "create_zone", "title": "T"}"#).unwrap();
    assert_eq!(command, Command::CreateZone { title: "T".to_string(), kind: ZoneKind::Manual, rect: None });
    let command: Command = serde_json::from_str(
        r#"{"command": "create_zone", "title": "D", "kind": {"type": "portal", "directory": "/d"}}"#,
    )
    .unwrap();
    assert!(matches!(command, Command::CreateZone { kind: ZoneKind::Portal { .. }, .. }));
    assert_eq!(
        serde_json::from_str::<Command>(r#"{"command": "apply_rules"}"#).unwrap(),
        Command::ApplyRules { scope: Scope::Loose }
    );
    assert_eq!(serde_json::from_str::<Command>(r#"{"command": "undo_rules"}"#).unwrap(), Command::UndoRules);

    // Everything a busy desk gave comes back from its JSON as it was.
    let mut desk = sorting_desk(true);
    let mut effects = desk.does(Command::ApplyRules { scope: Scope::All });
    effects.extend(desk.does(Command::Search { typed: "cat".to_string() }));
    effects.extend(desk.ask(Command::Untab { zone: ZoneId::new("papers") }));
    effects.extend(desk.core.command(
        Command::CreateZone {
            title: "P".to_string(),
            kind: ZoneKind::Portal { directory: "/archive".to_string() },
            rect: None,
        },
        NOW,
    ));
    effects.push(Effect::Ask(BackendRequest::Perform {
        request: 9,
        operation: FileOperation::Trash { path: "/a".to_string() },
    }));
    assert!(effects.len() > 8);
    for effect in effects {
        let text = serde_json::to_string(&effect).unwrap();
        assert_eq!(serde_json::from_str::<Effect>(&text).unwrap(), effect, "{text}");
    }
    let text = serde_json::to_string(&Effect::Notify(ZonesEvent::ZonePlaced {
        zone: ZoneId::new("z"),
        rect: Rect::new(1, 2, 3, 4),
    }))
    .unwrap();
    assert_eq!(
        text,
        r#"{"effect":"notify","event":"zone_placed","zone":"z","rect":{"x":1,"y":2,"width":3,"height":4}}"#
    );
    let scene = desk.core.scene(NOW);
    let text = serde_json::to_string(&scene).unwrap();
    assert_eq!(serde_json::from_str::<crate::core::Scene>(&text).unwrap(), scene);
    // A listing of a folder nobody asked about is passed over.
    let stray = BackendEvent::ItemAdded {
        location: Location::Directory { path: "/elsewhere".to_string() },
        item: Item::file("/elsewhere/a.txt"),
    };
    assert_eq!(desk.core.event(stray, NOW), []);
}
