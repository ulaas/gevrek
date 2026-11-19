//! What came after milestone 5, at work on the backend in memory: icons
//! drawn grey, what the one key takes with it, and zones made by type:
//! the plan, the making, the making again, and the taking back.

use crate::access;
use crate::auto::{AutoPlan, AutoUndo};
use crate::config::{Matcher, RuleAction, ZoneKind, ZonesConfig};
use crate::core::{Command, Effect, Scene, ZonesEvent, DESK};
use crate::core_scenarios::{events, refusal, second, Desk, NOW};
use crate::geometry::{Edge, ReservedArea};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::ipc::{Answer, Request, ZonesSnapshot};
use crate::item::{Item, ItemKind};
use crate::placement::{LoosePosition, Whereabouts};
use crate::rules::Scope;
use crate::snap;
use crate::state::ZonesState;
use crate::store;
use crate::testing::fake_backend::DESKTOP;
use gevrek_model::geometry::Rect;
use serde_json::json;

fn id(name: &str) -> ItemId {
    ItemId::new(format!("{DESKTOP}/{name}"))
}

fn saves(effects: &[Effect]) -> (bool, bool) {
    (effects.contains(&Effect::SaveConfig), effects.contains(&Effect::SaveState))
}

fn auto(dry_run: bool) -> Command {
    Command::AutoCreate { scope: Scope::Loose, min_items: None, display: None, add_rules: None, dry_run }
}

/// A desktop nobody sorted: three pictures, two documents, two folders,
/// one song, and This PC.
fn unsorted() -> Desk {
    let mut desk = Desk::new();
    for name in ["cat.png", "dog.jpg", "sea.png", "report.pdf", "notes.txt", "song.mp3"] {
        desk.backend.add_file(name);
    }
    for name in ["Projects", "Archive"] {
        desk.backend.add(DESKTOP, Item::folder(&format!("{DESKTOP}/{name}")));
    }
    let this_pc = Item { kind: ItemKind::Virtual, path: None, ..Item::file(&format!("{DESKTOP}/This PC")) };
    desk.backend.add(DESKTOP, this_pc);
    desk.pump();
    desk
}

fn planned(effects: &[Effect]) -> AutoPlan {
    events(effects)
        .into_iter()
        .find_map(|event| match event {
            ZonesEvent::AutoPlanned { plan } => Some(plan),
            _ => None,
        })
        .expect("a plan was said")
}

fn zone_for(desk: &Desk, category: &str) -> ZoneId {
    let zones = &desk.core.config().zones;
    zones
        .iter()
        .find(|zone| zone.auto.as_deref() == Some(category))
        .map(|zone| zone.id.clone())
        .unwrap_or_else(|| panic!("no zone was made for {category}"))
}

// ---- icons drawn grey -------------------------------------------------------

#[test]
fn icons_are_drawn_grey_by_one_switch_that_is_remembered_and_changes_no_look() {
    let mut desk = Desk::new();
    let zone = desk.zone("Papers", Rect::new(100, 100, 300, 300));
    desk.does(Command::ModifyZone { zone: zone.clone(), changes: json!({"look": {"icon_saturation": 60}}) });
    desk.does(Command::ModifySettings { changes: json!({"look": {"icon_saturation": 80}}) });
    let config = desk.core.config().clone();
    assert!(!desk.core.grey_icons() && !desk.core.scene(NOW).grey_icons);

    // Without a value: the other of what it is.
    let effects = desk.does(Command::GreyIcons { on: None });
    assert_eq!(events(&effects), [ZonesEvent::GreyIconsChanged { on: true }]);
    assert_eq!(saves(&effects), (false, true), "it is remembered, and is no setting");
    assert!(desk.core.grey_icons() && desk.core.scene(NOW).grey_icons && desk.core.state().grey_icons);
    assert!(ZonesSnapshot::of(&desk.core, NOW).grey_icons);
    // The looks are as the person set them: the grey is laid over them.
    assert_eq!(desk.core.config(), &config);
    let scene = desk.core.scene(NOW);
    assert_eq!(scene.zones[0].look.icon_saturation, Some(60));

    // What it is already: nothing is said and nothing is saved.
    assert_eq!(desk.does(Command::GreyIcons { on: Some(true) }), []);
    // It is still so after a restart.
    let mut again = Desk::on(desk.core.config().clone(), desk.core.state().clone(), desk.backend);
    assert!(again.core.scene(NOW).grey_icons);
    let effects = again.does(Command::GreyIcons { on: None });
    assert_eq!(events(&effects), [ZonesEvent::GreyIconsChanged { on: false }]);
    assert!(!again.core.scene(NOW).grey_icons);
    assert_eq!(again.does(Command::GreyIcons { on: Some(false) }), []);
    // At its default it is not in the state file, nor in what is told.
    assert!(!store::encode(again.core.state()).contains("grey_icons"));
    assert!(!serde_json::to_string(&again.core.scene(NOW)).unwrap().contains("grey_icons"));
    assert!(!serde_json::to_string(&ZonesSnapshot::of(&again.core, NOW)).unwrap().contains("grey_icons"));

    // Switched off, the zones still say how the desktop is looked at.
    let state = ZonesState { grey_icons: true, ..ZonesState::default() };
    let switched_off = ZonesConfig { enabled: false, ..ZonesConfig::default() };
    let off = Desk::as_configured(switched_off, state, crate::testing::fake_backend::FakeBackend::new());
    assert!(off.core.scene(NOW).grey_icons);
}

// ---- what the one key takes with it ------------------------------------------

#[test]
fn the_one_key_takes_the_loose_items_with_the_zones_and_the_scene_says_so() {
    let mut desk = Desk::with(ZonesConfig::default(), ZonesState::default());
    let zone = desk.zone("Papers", Rect::new(600, 100, 300, 300));
    let inside = desk.file("inside.txt");
    desk.put(&zone, &inside);
    let loose = desk.file("loose.txt");
    assert!(!desk.core.scene(NOW).loose_hidden && !desk.core.loose_hidden());
    desk.does(Command::FocusZone { zone: Some(ZoneId::new(DESK)) });
    assert!(desk.core.focus().is_some_and(|focus| focus.desk.is_some()));

    desk.does(Command::QuickHide { hidden: Some(true) });
    let scene = desk.core.scene(NOW);
    assert!(scene.quick_hidden && !scene.zones[0].shown);
    assert!(scene.loose_hidden && desk.core.loose_hidden());
    // The keyboard does not stay on what is not seen.
    assert!(desk.core.focus().is_none());
    let asked = desk.ask(Command::FocusItem { item: Some(loose.clone()) });
    assert!(refusal(&asked).is_some());
    // A screen reader is told the same.
    let tree = access::tree(&scene);
    let node = tree.children.iter().find(|node| node.role == access::Role::Loose).expect("the loose items");
    assert!(node.children[0].hidden);

    // Peeking shows what was put away.
    desk.does(Command::Peek { on: true });
    let scene = desk.core.scene(NOW);
    assert!(scene.quick_hidden && scene.peeking && !scene.loose_hidden);
    desk.does(Command::Peek { on: false });
    assert!(desk.core.scene(NOW).loose_hidden);
    desk.does(Command::QuickHide { hidden: Some(false) });
    assert!(!desk.core.scene(NOW).loose_hidden);
}

// ---- zones made by type ------------------------------------------------------

#[test]
fn a_dry_run_says_what_would_be_made_and_changes_nothing() {
    let mut desk = unsorted();
    let before = (desk.core.config().clone(), desk.core.state().clone());
    let effects = desk.does(auto(true));
    assert_eq!(effects.len(), 1, "a plan, and nothing to save or to ask: {effects:?}");
    let plan = planned(&effects);
    assert_eq!((desk.core.config(), desk.core.state()), (&before.0, &before.1));
    assert!(!desk.core.can_undo_auto());
    assert_eq!(plan, desk.core.auto_plan(Scope::Loose, None, None, NOW), "what the core says when asked");

    let said: Vec<(&str, usize, bool)> =
        plan.zones.iter().map(|zone| (zone.title.as_str(), zone.items.len(), zone.rule)).collect();
    assert_eq!(said, [("Folders", 2, true), ("Documents", 2, true), ("Pictures", 3, true)]);
    assert!(plan.zones.iter().all(|zone| zone.zone.is_none()));
    assert_eq!(plan.left, [id("song.mp3")], "one song is too few; This PC is never sorted");
    assert_eq!(plan.display, Some(DisplayId::new("fake-1")));
    // In a row from the top of the free room, apart by the snap gap; and
    // beside what stays lying loose (the song and This PC, in the first
    // column of icons, each cell 76 wide), not over it.
    let area = desk.core.displays()[0].rect;
    let gap = desk.core.config().snap.gap;
    assert_eq!((plan.zones[0].rect.x, plan.zones[0].rect.y), (area.x + 76 + gap, area.y + gap));
    let scene = desk.core.scene(NOW);
    let lying: Vec<Rect> = scene.loose[0]
        .items
        .iter()
        .filter(|placed| plan.left.contains(&placed.item) || placed.item.as_str().starts_with("::"))
        .map(|placed| Rect::new(scene.loose[0].area.x + placed.rect.x, scene.loose[0].area.y + placed.rect.y, placed.rect.width, placed.rect.height))
        .collect();
    assert!(!lying.is_empty());
    for zone in &plan.zones {
        for icon in &lying {
            let apart = zone.rect.x >= icon.x + icon.width || zone.rect.y >= icon.y + icon.height || icon.x >= zone.rect.x + zone.rect.width || icon.y >= zone.rect.y + zone.rect.height;
            assert!(apart, "{:?} lies over the icon at {icon:?}", zone.rect);
        }
    }
    for pair in plan.zones.windows(2) {
        assert_eq!(pair[1].rect.y, pair[0].rect.y);
        assert_eq!(pair[1].rect.x, pair[0].rect.x + pair[0].rect.width + gap);
    }

    // What is asked with the command is what is planned with.
    let all = desk.does(Command::AutoCreate {
        scope: Scope::All,
        min_items: Some(1),
        display: None,
        add_rules: Some(false),
        dry_run: true,
    });
    let plan = planned(&all);
    assert_eq!(plan.zones.len(), 4, "the song has a zone now");
    assert!(plan.zones.iter().all(|zone| !zone.rule) && plan.left.is_empty());

    // The plan reaches whoever asked from outside: as an event of the answer.
    let request = Request::from_json(r#"{"command": "auto_create", "dry_run": true}"#).unwrap();
    let Request::Command(command) = request else { panic!("a command of the core") };
    assert_eq!(command, auto(true));
    let answer = Answer::of(&desk.does(command));
    assert!(answer.ok);
    let written = serde_json::to_value(&answer).unwrap();
    let event = &written["events"][0];
    assert_eq!(event["event"], "auto_planned");
    assert_eq!(event["plan"]["zones"][2]["title"], "Pictures");
    assert_eq!(event["plan"]["zones"][2]["items"], json!(["/desk/cat.png", "/desk/dog.jpg", "/desk/sea.png"]));
    assert_eq!(event["plan"]["zones"][2]["rule"], true);
    assert!(event["plan"]["zones"][2].get("zone").is_none(), "it is not there yet");
    assert_eq!(event["plan"]["left"], json!(["/desk/song.mp3"]));
    assert_eq!(serde_json::from_value::<Answer>(written).unwrap(), answer);
}

#[test]
fn zones_are_made_by_type_filled_and_given_their_rules() {
    let mut desk = unsorted();
    let plan = planned(&desk.does(auto(true)));
    let effects = desk.does(auto(false));
    assert_eq!(saves(&effects), (true, true));
    assert_eq!(effects.iter().filter(|effect| matches!(effect, Effect::SaveConfig | Effect::SaveState)).count(), 2);
    let heard = events(&effects);
    let made: Vec<ZoneId> = heard
        .iter()
        .filter_map(|event| match event {
            ZonesEvent::ZoneCreated { zone } => Some(zone.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(made.len(), 3);
    assert!(heard.contains(&ZonesEvent::AutoCreated { zones: made.clone(), items: 7, rules: 3 }));
    for zone in &made {
        assert!(heard.contains(&ZonesEvent::ItemsChanged { zone: Some(zone.clone()) }));
    }
    assert!(heard.contains(&ZonesEvent::ItemsChanged { zone: None }), "the loose items are fewer");
    assert!(heard.contains(&ZonesEvent::LayoutChanged));

    // Exactly the plan: each zone where it was said, with what was said.
    for (planned, zone) in plan.zones.iter().zip(&made) {
        let definition = desk.core.config().zone(zone).unwrap();
        assert_eq!(definition.title, planned.title);
        assert_eq!(definition.kind, ZoneKind::Manual);
        assert_eq!(definition.auto.as_deref(), Some(planned.category.as_str()));
        assert_eq!(desk.rect(zone), planned.rect);
        let held: Vec<ItemId> = desk.core.zone_items(zone, NOW).into_iter().map(|item| item.id.clone()).collect();
        assert_eq!(held, planned.items);
    }
    let loose: Vec<&str> = desk.core.loose_items().into_iter().map(|item| item.name.as_str()).collect();
    assert_eq!(loose, ["This PC", "song.mp3"]);
    let rules: Vec<(&str, &RuleAction)> =
        desk.core.config().rules.iter().map(|rule| (rule.name.as_str(), &rule.action)).collect();
    assert_eq!(
        rules,
        [
            ("Auto: Folders", &RuleAction::AssignToZone { zone: made[0].clone() }),
            ("Auto: Documents", &RuleAction::AssignToZone { zone: made[1].clone() }),
            ("Auto: Pictures", &RuleAction::AssignToZone { zone: made[2].clone() }),
        ]
    );
    assert_eq!(desk.core.config().problems(), []);
    assert_eq!(desk.core.config().remarks(), []);
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert!(told.can_undo_auto && desk.core.can_undo_auto());
    assert_eq!(told.zone(made[2].as_str()).unwrap().auto.as_deref(), Some("pictures"));

    // Asked again there is nothing to do: an empty plan, not a refusal,
    // and what can be taken back is still the making.
    let again = planned(&desk.does(auto(true)));
    assert!(again.zones.is_empty());
    assert_eq!(again.left, [id("song.mp3")]);
    let record = desk.core.state().auto_undo.clone();
    let effects = desk.does(auto(false));
    assert_eq!(events(&effects), [ZonesEvent::AutoCreated { zones: Vec::new(), items: 0, rules: 0 }]);
    assert_eq!(saves(&effects), (false, false));
    assert_eq!(desk.core.state().auto_undo, record);

    // What comes later and is of a type goes into its zone by its rule.
    let late = desk.file("late.png");
    assert_eq!(desk.core.zone_of(&late), Some(&made[2]));
    let folder = Item::folder(&format!("{DESKTOP}/More"));
    desk.backend.add(DESKTOP, folder.clone());
    desk.pump();
    assert_eq!(desk.core.zone_of(&folder.id), Some(&made[0]));
    // A second song makes two: now they have a zone, and the zones that
    // are there are not made again.
    let second_song = desk.file("tune.flac");
    assert_eq!(desk.core.zone_of(&second_song), None);
    let effects = desk.does(auto(false));
    let audio = zone_for(&desk, "audio");
    assert!(events(&effects).contains(&ZonesEvent::AutoCreated { zones: vec![audio.clone()], items: 2, rules: 1 }));
    assert_eq!(desk.core.config().zones.len(), 4);
    assert_eq!(desk.names_in(&audio), ["song.mp3", "tune.flac"]);
    // It stands clear of the three that were there.
    let rects: Vec<Rect> = desk.core.config().zones.iter().map(|zone| desk.rect(&zone.id)).collect();
    for (index, rect) in rects.iter().enumerate() {
        for other in &rects[..index] {
            assert!(rect.x >= other.x + other.width || rect.y >= other.y + other.height, "{rect:?} on {other:?}");
        }
    }
}

#[test]
fn a_type_that_has_its_zone_takes_what_is_new_whatever_the_number() {
    let mut desk = unsorted();
    desk.does(Command::ModifySettings { changes: json!({"rules_on_arrival": false}) });
    desk.does(auto(false));
    let pictures = zone_for(&desk, "pictures");
    let rules = desk.core.config().rules.len();
    // One picture more, loose (nothing sorts on arrival here).
    let one = desk.file("one.gif");
    assert_eq!(desk.core.zone_of(&one), None);
    let plan = planned(&desk.does(auto(true)));
    assert_eq!(plan.zones.len(), 1);
    assert_eq!(plan.zones[0].zone, Some(pictures.clone()));
    assert_eq!(plan.zones[0].rect, desk.rect(&pictures), "where it is");
    assert_eq!((plan.zones[0].items.as_slice(), plan.zones[0].rule), ([one.clone()].as_slice(), false));
    let effects = desk.does(auto(false));
    assert!(events(&effects).contains(&ZonesEvent::AutoCreated { zones: Vec::new(), items: 1, rules: 0 }));
    assert_eq!(saves(&effects), (false, true), "no zone and no rule is new: only where things are");
    assert_eq!(desk.core.zone_of(&one), Some(&pictures));
    assert_eq!(desk.core.config().rules.len(), rules);
    // Taking that back takes the one picture out, and no zone away.
    let effects = desk.does(Command::UndoAutoCreate);
    assert!(events(&effects).contains(&ZonesEvent::AutoUndone { zones: 0, items: 1, rules: 0 }));
    assert_eq!(desk.core.zone_of(&one), None);
    assert_eq!(desk.core.config().zones.len(), 3);

    // The zone renamed and moved is still its type's zone.
    desk.does(Command::RenameZone { zone: pictures.clone(), title: "Photos".to_string() });
    desk.does(Command::MoveZone { zone: pictures.clone(), x: 900, y: 600, snap: false });
    let plan = planned(&desk.does(auto(true)));
    assert_eq!((plan.zones[0].title.as_str(), plan.zones[0].rect), ("Photos", desk.rect(&pictures)));
    // Given to another purpose, it is not: a zone would be made (with
    // two pictures loose, enough for one).
    desk.file("two.gif");
    desk.does(Command::ModifyZone { zone: pictures.clone(), changes: json!({"auto": null}) });
    let plan = planned(&desk.does(auto(true)));
    assert_eq!((plan.zones[0].zone.clone(), plan.zones[0].items.len()), (None, 2));
}

#[test]
fn making_zones_by_type_is_taken_back_whole() {
    let mut desk = unsorted();
    let by_hand = desk.zone("Inbox", Rect::new(1200, 600, 400, 300));
    for name in ["sea.png", "report.pdf", "cat.png"] {
        desk.put(&by_hand, &id(name));
    }
    let at = LoosePosition { display: DisplayId::new("fake-1"), x: 700, y: 500 };
    desk.does(Command::PlaceLooseItem { item: id("dog.jpg"), display: at.display.clone(), x: at.x, y: at.y });
    let before = (desk.core.config().clone(), desk.core.state().clone());
    assert!(refusal(&desk.ask(Command::UndoAutoCreate)).is_some(), "nothing to take back yet");

    // Everything, also what was sorted by hand.
    let command = Command::AutoCreate { scope: Scope::All, min_items: None, display: None, add_rules: None, dry_run: false };
    let effects = desk.does(command);
    let heard = events(&effects);
    assert!(heard.contains(&ZonesEvent::ItemsChanged { zone: Some(by_hand.clone()) }), "the inbox is emptier");
    assert!(desk.core.zone_holds(&by_hand, NOW).is_empty());
    assert_eq!(desk.core.config().zones.len(), 4);
    let record: AutoUndo = desk.core.state().auto_undo.clone().unwrap();
    assert_eq!((record.made_at, record.zones.len(), record.entries.len(), record.rules.len()), (NOW, 3, 7, 3));
    let was = |name: &str| record.entries.iter().find(|entry| entry.item == id(name)).map(|entry| entry.before.clone());
    assert_eq!(was("dog.jpg"), Some(Whereabouts::Loose { position: Some(at.clone()) }));
    assert_eq!(was("report.pdf"), Some(Whereabouts::Zone { zone: by_hand.clone(), index: 1, cell: None }));
    // The record survives the state file.
    let (read, _) = store::decode(&store::encode(desk.core.state())).unwrap();
    assert_eq!(&read, desk.core.state());

    let effects = desk.does(Command::UndoAutoCreate);
    let heard = events(&effects);
    assert!(heard.contains(&ZonesEvent::AutoUndone { zones: 3, items: 7, rules: 3 }));
    assert_eq!(heard.iter().filter(|event| matches!(event, ZonesEvent::ZoneDeleted { .. })).count(), 3);
    assert!(heard.contains(&ZonesEvent::ItemsChanged { zone: Some(by_hand.clone()) }));
    assert!(heard.contains(&ZonesEvent::ItemsChanged { zone: None }));
    assert_eq!(saves(&effects), (true, true));
    // The zones, the rules, and every item where it was: in the inbox in
    // its order, loose at its own place.
    assert_eq!(desk.core.config(), &before.0);
    assert_eq!(desk.core.state().placements, before.1.placements);
    assert_eq!(desk.names_in(&by_hand), ["sea.png", "report.pdf", "cat.png"]);
    assert_eq!(desk.core.state().placements.loose.get(&id("dog.jpg")), Some(&at));
    assert!(!desk.core.can_undo_auto() && !ZonesSnapshot::of(&desk.core, NOW).can_undo_auto);
    assert!(!store::encode(desk.core.state()).contains("auto_undo"));
    assert_eq!(
        refusal(&desk.ask(Command::UndoAutoCreate)).as_deref(),
        Some("no zones were made by type, or it was taken back already")
    );
}

#[test]
fn taking_back_leaves_what_the_person_changed_since() {
    let mut desk = unsorted();
    desk.does(auto(false));
    let (folders, documents, pictures) =
        (zone_for(&desk, "folders"), zone_for(&desk, "documents"), zone_for(&desk, "pictures"));
    // Since then: a picture moved by hand into the documents, a rule
    // given another name, the folders' zone made the person's own, and a
    // file put into the pictures by hand.
    desk.put(&documents, &id("cat.png"));
    desk.does(Command::ModifySettings {
        changes: json!({"rules": [
            {"name": "Auto: Folders", "matchers": [{"match": "kind", "any_of": ["folder"]}],
             "action": {"action": "assign_to_zone", "zone": folders.as_str()}},
            {"name": "My papers", "matchers": [{"match": "extension", "any_of": ["pdf"]}],
             "action": {"action": "assign_to_zone", "zone": documents.as_str()}},
            {"name": "Auto: Pictures", "matchers": [{"match": "extension", "any_of": ["png"]}],
             "action": {"action": "assign_to_zone", "zone": pictures.as_str()}}]}),
    });
    desk.does(Command::ModifyZone { zone: folders.clone(), changes: json!({"auto": null}) });
    desk.put(&pictures, &id("song.mp3"));

    let effects = desk.does(Command::UndoAutoCreate);
    // Two zones go (the folders' is the person's now, and keeps what it
    // holds); of the seven items four go back: not the two folders, and
    // not the picture that was moved by hand.
    assert!(events(&effects).contains(&ZonesEvent::AutoUndone { zones: 2, items: 4, rules: 2 }));
    let zones: Vec<&ZoneId> = desk.core.config().zones.iter().map(|zone| &zone.id).collect();
    assert_eq!(zones, [&folders]);
    assert_eq!(desk.names_in(&folders), ["Archive", "Projects"]);
    let rules: Vec<&str> = desk.core.config().rules.iter().map(|rule| rule.name.as_str()).collect();
    assert_eq!(rules, ["My papers"], "the one that was given another name is the person's");
    // What a zone that went held lies loose, also what was put in by hand.
    assert_eq!(desk.core.zone_of(&id("song.mp3")), None);
    assert_eq!(desk.core.zone_of(&id("cat.png")), None);
    assert_eq!(desk.core.loose_items().len(), 7);
    assert_eq!(desk.core.broken(), None);
}

#[test]
fn locks_and_the_switch_hold_for_zones_made_by_type() {
    let mut desk = unsorted();
    // The lock for all zones refuses the whole of it, the dry run too.
    desk.does(Command::LockAll { geometry: None, members: Some(true) });
    for dry_run in [true, false] {
        assert_eq!(
            refusal(&desk.ask(auto(dry_run))).as_deref(),
            Some("every zone is locked: nothing goes in or out")
        );
    }
    assert!(desk.core.auto_plan(Scope::Loose, None, None, NOW).zones.is_empty());
    assert!(desk.core.config().zones.is_empty());
    desk.does(Command::LockAll { geometry: None, members: Some(false) });
    desk.does(auto(false));
    let pictures = zone_for(&desk, "pictures");
    let state = desk.core.state().clone();
    // Nor is it taken back under that lock; and it can be once it is off.
    desk.does(Command::LockAll { geometry: None, members: Some(true) });
    assert!(refusal(&desk.ask(Command::UndoAutoCreate)).is_some());
    assert_eq!(desk.core.state(), &state);
    desk.does(Command::LockAll { geometry: None, members: Some(false) });

    // A zone locked to what it holds takes nothing new.
    desk.does(Command::ModifySettings { changes: json!({"rules_on_arrival": false}) });
    desk.does(Command::Lock { zone: pictures.clone(), geometry: None, members: Some(true) });
    let (one, two) = (desk.file("one.gif"), desk.file("two.gif"));
    let plan = planned(&desk.does(auto(true)));
    assert!(plan.zones.is_empty(), "no second zone for pictures either");
    assert_eq!(plan.left, [one.clone(), two.clone(), id("song.mp3")]);
    // And is left as it is when the rest is taken back.
    let effects = desk.does(Command::UndoAutoCreate);
    assert!(events(&effects).contains(&ZonesEvent::AutoUndone { zones: 2, items: 4, rules: 3 }));
    assert_eq!(desk.names_in(&pictures), ["cat.png", "dog.jpg", "sea.png"]);

    // Switched off, nothing is made and nothing is planned.
    let mut off = Desk::as_configured(
        ZonesConfig { enabled: false, ..ZonesConfig::default() },
        ZonesState::default(),
        crate::testing::fake_backend::FakeBackend::new(),
    );
    off.backend.add_file("a.png");
    off.backend.add_file("b.png");
    off.pump();
    for command in [auto(true), auto(false), Command::UndoAutoCreate, Command::GreyIcons { on: None }] {
        assert_eq!(
            refusal(&off.ask(command)).as_deref(),
            Some("zones are switched off (desktop.zones.enabled)")
        );
    }
    assert!(off.core.config().zones.is_empty() && !off.core.grey_icons());
}

#[test]
fn new_zones_go_to_the_display_that_is_named_and_keep_off_what_is_there() {
    let mut desk = unsorted();
    desk.backend.plug(second());
    desk.backend.set_reserved(vec![ReservedArea { display: DisplayId::new("fake-2"), edge: Edge::Top, thickness: 40 }]);
    desk.pump();
    let other = DisplayId::new("fake-2");
    let there = desk.zone("Mine", Rect::new(1920 + 8, 48, 600, 300));
    let rolled = desk.zone("Rolled", Rect::new(1920 + 700, 48, 400, 300));
    desk.does(Command::RollUp { zone: rolled.clone(), rolled_up: Some(true) });
    let command = |display: &str, dry_run: bool| Command::AutoCreate {
        scope: Scope::Loose,
        min_items: None,
        display: Some(DisplayId::new(display)),
        add_rules: None,
        dry_run,
    };
    assert_eq!(refusal(&desk.ask(command("nowhere", true))).as_deref(), Some("no display is called nowhere"));
    assert!(desk.core.auto_plan(Scope::Loose, None, Some(&DisplayId::new("nowhere")), NOW).zones.is_empty());

    let plan = planned(&desk.does(command("fake-2", true)));
    assert_eq!(plan.display, Some(other.clone()));
    assert_eq!(plan, desk.core.auto_plan(Scope::Loose, None, Some(&other), NOW));
    let area = Rect::new(1920, 40, 2560, 1400);
    let taken = [Rect::new(1928, 48, 600, 300), Rect::new(2620, 48, 400, 300)];
    for zone in &plan.zones {
        assert!(snap::within(zone.rect, area), "{:?} is in the free room", zone.rect);
        for other in taken {
            // (A rolled-up zone keeps the room it opens to.)
            let apart = zone.rect.x >= other.x + other.width + 8 || zone.rect.y >= other.y + other.height + 8;
            assert!(apart, "{:?} keeps off {other:?}", zone.rect);
        }
    }
    desk.does(command("fake-2", false));
    for category in ["folders", "documents", "pictures"] {
        let zone = zone_for(&desk, category);
        let scene = desk.core.scene(NOW);
        let shown = scene.zones.iter().find(|shown| shown.zone == zone).unwrap();
        assert_eq!(shown.display, other);
    }
    // Without a display: the primary one, and clear of nothing there but
    // the column of icons that stay lying loose.
    desk.does(Command::UndoAutoCreate);
    let plan = planned(&desk.does(auto(true)));
    assert_eq!(plan.display, Some(DisplayId::new("fake-1")));
    assert_eq!((plan.zones[0].rect.x, plan.zones[0].rect.y), (84, 8));
    let _ = there;
}

#[test]
fn the_new_commands_events_and_keys_are_written_as_they_are_read() {
    for (text, command) in [
        (r#"{"command": "grey_icons"}"#, Command::GreyIcons { on: None }),
        (r#"{"command": "grey_icons", "on": false}"#, Command::GreyIcons { on: Some(false) }),
        (r#"{"command": "auto_create"}"#, auto(false)),
        (
            r#"{"command": "auto_create", "scope": "all", "min_items": 3, "display": "fake-2",
                "add_rules": false, "dry_run": true}"#,
            Command::AutoCreate {
                scope: Scope::All,
                min_items: Some(3),
                display: Some(DisplayId::new("fake-2")),
                add_rules: Some(false),
                dry_run: true,
            },
        ),
        (r#"{"command": "undo_auto_create"}"#, Command::UndoAutoCreate),
    ] {
        let read = Request::from_json(text).unwrap_or_else(|why| panic!("{text}: {why}"));
        assert_eq!(read, Request::Command(command), "{text}");
        assert_eq!(Request::from_value(read.to_value()).unwrap(), read);
    }
    assert_eq!(
        Request::Command(auto(false)).to_value(),
        json!({"command": "auto_create", "scope": "loose", "dry_run": false})
    );

    let mut desk = unsorted();
    let mut effects = desk.does(auto(true));
    effects.extend(desk.does(auto(false)));
    effects.extend(desk.does(Command::GreyIcons { on: None }));
    let scene = desk.core.scene(NOW);
    let told = ZonesSnapshot::of(&desk.core, NOW);
    effects.extend(desk.does(Command::UndoAutoCreate));
    for effect in effects {
        let text = serde_json::to_string(&effect).unwrap();
        assert_eq!(serde_json::from_str::<Effect>(&text).unwrap(), effect, "{text}");
    }
    let names: Vec<String> = [
        ZonesEvent::GreyIconsChanged { on: true },
        ZonesEvent::AutoPlanned { plan: AutoPlan::default() },
        ZonesEvent::AutoCreated { zones: Vec::new(), items: 0, rules: 0 },
        ZonesEvent::AutoUndone { zones: 0, items: 0, rules: 0 },
    ]
    .iter()
    .map(crate::ipc::event_name)
    .collect();
    assert_eq!(names, ["grey_icons_changed", "auto_planned", "auto_created", "auto_undone"]);
    let text = serde_json::to_string(&scene).unwrap();
    assert!(text.contains("\"grey_icons\":true"));
    assert_eq!(serde_json::from_str::<Scene>(&text).unwrap(), scene);
    let text = serde_json::to_string(&told).unwrap();
    assert!(text.contains("\"grey_icons\":true") && text.contains("\"can_undo_auto\":true"));
    assert!(text.contains("\"auto\":\"pictures\""));
    assert_eq!(ZonesSnapshot::from_json(&text).unwrap(), told);

    // A scene, a snapshot and a state from before read without them.
    let scene: Scene = serde_json::from_str(r#"{"zones": [], "loose": [], "quick_hidden": true, "peeking": false}"#).unwrap();
    assert!(!scene.grey_icons && !scene.loose_hidden);
    let old = r#"{"version":1,"enabled":true,"zones":[{"id":"a","title":"A","visible":true}],"loose":[]}"#;
    let told = ZonesSnapshot::from_json(old).unwrap();
    assert!(!told.grey_icons && !told.can_undo_auto && told.zones[0].auto.is_none());
    let (state, _) = store::decode(r#"{"version": 1, "quick_hidden": true}"#).unwrap();
    assert!(!state.grey_icons && state.auto_undo.is_none());
    let written = store::encode(&state);
    assert!(!written.contains("grey_icons") && !written.contains("auto_undo"));
    // The rule a type is given is one the configuration reads.
    let matcher = Matcher::Extension { any_of: vec!["png".to_string()] };
    assert_eq!(serde_json::to_value(&matcher).unwrap(), json!({"match": "extension", "any_of": ["png"]}));
}
