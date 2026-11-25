//! What came with milestones 3 to 5, at work on the backend in memory:
//! rolling up and opening for a moment, tabs, what stays when the rest
//! is put away, peeking, locks, looks, portals that walk, rules on a
//! schedule, zones that fill themselves, layouts carried about, the
//! keyboard, covering from view, and what is told to others of it all.

use crate::access::{self, Role};
use crate::backend::{BackendError, BackendEvent, BackendRequest, FileOperation};
use crate::config::{
    ExpandOn, Lock, Matcher, Rule, RuleAction, SmartSource, TitleBar, ZoneDefinition, ZoneKind,
    ZoneLook, ZonesConfig,
};
use crate::core::{Command, Effect, Scene, TabPlace, ZoneScene, ZonesEvent, DESK, FOCUSED};
use crate::core_scenarios::{events, refusal, second, sorting_desk, Desk, DAY, NOW};
use crate::export::{LayoutDocument, LAYOUT_FORMAT};
use crate::focus::{Direction, Focus, SelectMode, Step};
use crate::geometry::{Edge, ReservedArea};
use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::ipc::{Request, ZonesSnapshot};
use crate::item::{Item, ItemKind};
use crate::portal::Listing;
use crate::reserve::ReservedRegion;
use crate::rules::{Decided, Destination, Reason, Scope};
use crate::state::{ZonesState, AUTOMATIC_KEPT};
use crate::store;
use crate::testing::fake_backend::{FakeBackend, DESKTOP};
use gevrek_model::geometry::Rect;
use serde_json::json;

fn id(name: &str) -> ItemId {
    ItemId::new(format!("{DESKTOP}/{name}"))
}

fn zone_scene(desk: &Desk, zone: &ZoneId) -> ZoneScene {
    let scene = desk.core.scene(NOW);
    scene
        .zones
        .into_iter()
        .find(|shown| &shown.zone == zone)
        .expect("the zone is in the scene")
}

/// Make a zone of a kind and say what it is called.
fn make(desk: &mut Desk, title: &str, kind: ZoneKind) -> ZoneId {
    let effects = desk.does(Command::CreateZone {
        title: title.to_string(),
        kind,
        rect: None,
    });
    events(&effects)
        .into_iter()
        .find_map(|event| match event {
            ZonesEvent::ZoneCreated { zone } => Some(zone),
            _ => None,
        })
        .expect("a zone was made")
}

fn portal(desk: &mut Desk, title: &str, directory: &str) -> ZoneId {
    make(
        desk,
        title,
        ZoneKind::Portal {
            directory: directory.to_string(),
        },
    )
}

fn modify(desk: &mut Desk, zone: &ZoneId, changes: serde_json::Value) -> Vec<Effect> {
    desk.does(Command::ModifyZone {
        zone: zone.clone(),
        changes,
    })
}

fn settings(desk: &mut Desk, changes: serde_json::Value) -> Vec<Effect> {
    desk.does(Command::ModifySettings { changes })
}

fn over(desk: &mut Desk, zone: Option<&ZoneId>, at_ms: u64) -> Vec<Effect> {
    desk.does(Command::PointerOver {
        zone: zone.cloned(),
        at_ms,
        dragging: false,
    })
}

fn tick(desk: &mut Desk, at_ms: u64) -> Vec<Effect> {
    desk.does(Command::Tick { at_ms })
}

fn saves(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::SaveConfig | Effect::SaveState))
}

fn focus_of(desk: &Desk) -> Option<(String, Option<String>)> {
    desk.core.focus().map(|focus| {
        (
            focus.zone.to_string(),
            focus.item.as_ref().map(|item| item.to_string()),
        )
    })
}

// ---- milestone 3 ----------------------------------------------------------

#[test]
fn a_rolled_up_zone_opens_under_the_pointer_and_closes_when_it_has_left() {
    let mut desk = Desk::new();
    let tools = desk.zone("Tools", Rect::new(100, 100, 400, 300));
    let other = desk.zone("Other", Rect::new(700, 100, 300, 300));
    desk.does(Command::RollUp {
        zone: tools.clone(),
        rolled_up: Some(true),
    });
    let whole = Rect::new(100, 100, 400, 300);
    let title = Rect::new(100, 100, 400, 28);
    assert_eq!(desk.rect(&tools), title);
    let config = desk.core.config().clone();
    let state = desk.core.state().clone();

    // The pointer rests on it: it opens after a moment, not at once.
    assert_eq!(over(&mut desk, Some(&tools), 1000), []);
    assert_eq!(desk.core.wake_at_ms(), Some(1150));
    assert_eq!(tick(&mut desk, 1100), []);
    // (Told again that it is still there, the moment does not begin anew.)
    over(&mut desk, Some(&tools), 1120);
    assert_eq!(desk.core.wake_at_ms(), Some(1150));
    let effects = tick(&mut desk, 1150);
    assert_eq!(
        events(&effects),
        [ZonesEvent::ZoneExpanded {
            zone: tools.clone(),
            expanded: true
        }]
    );
    assert!(!saves(&effects), "opening for a moment is written nowhere");
    assert_eq!(desk.rect(&tools), whole);
    let scene = zone_scene(&desk, &tools);
    assert!(scene.rolled_up && scene.expanded && scene.expand_on == ExpandOn::Hover);
    assert_eq!((desk.core.config(), desk.core.state()), (&config, &state));
    assert_eq!(desk.core.wake_at_ms(), None);

    // It has left: closed after a moment, unless it comes back in time.
    over(&mut desk, None, 2000);
    assert_eq!(desk.core.wake_at_ms(), Some(2400));
    over(&mut desk, Some(&tools), 2200);
    assert_eq!(desk.core.wake_at_ms(), None);
    over(&mut desk, Some(&other), 3000);
    assert_eq!(tick(&mut desk, 3399), []);
    let effects = tick(&mut desk, 3400);
    assert_eq!(
        events(&effects),
        [ZonesEvent::ZoneExpanded {
            zone: tools.clone(),
            expanded: false
        }]
    );
    assert_eq!(desk.rect(&tools), title);
    assert_eq!((desk.core.config(), desk.core.state()), (&config, &state));

    // Passing over it opens nothing.
    over(&mut desk, Some(&tools), 5000);
    over(&mut desk, None, 5100);
    assert_eq!(tick(&mut desk, 6000), []);
    assert!(!desk.core.is_expanded(&tools));
    assert_eq!(desk.core.wake_at_ms(), None);
}

#[test]
fn a_zone_that_opens_on_a_press_does_not_open_under_the_pointer_but_does_for_a_drag() {
    let mut desk = Desk::new();
    let tools = desk.zone("Tools", Rect::new(100, 100, 400, 300));
    let plain = desk.zone("Plain", Rect::new(700, 100, 300, 300));
    desk.does(Command::RollUp {
        zone: tools.clone(),
        rolled_up: Some(true),
    });
    modify(&mut desk, &tools, json!({"expand_on": "click"}));
    assert_eq!(zone_scene(&desk, &tools).expand_on, ExpandOn::Click);

    over(&mut desk, Some(&tools), 1000);
    assert_eq!(desk.core.wake_at_ms(), None);
    assert_eq!(tick(&mut desk, 9000), []);
    // Something is carried over it: it must be possible to drop it inside.
    desk.does(Command::PointerOver {
        zone: Some(tools.clone()),
        at_ms: 9100,
        dragging: true,
    });
    assert_eq!(desk.core.wake_at_ms(), Some(9250));
    assert_eq!(
        events(&tick(&mut desk, 9250)),
        [ZonesEvent::ZoneExpanded {
            zone: tools.clone(),
            expanded: true
        }]
    );
    over(&mut desk, None, 9300);
    tick(&mut desk, 9700);
    assert!(!desk.core.is_expanded(&tools));

    // A press on its title opens it, another closes it.
    let effects = desk.does(Command::Expand {
        zone: tools.clone(),
        expanded: None,
    });
    assert_eq!(
        events(&effects),
        [ZonesEvent::ZoneExpanded {
            zone: tools.clone(),
            expanded: true
        }]
    );
    desk.does(Command::Expand {
        zone: tools.clone(),
        expanded: None,
    });
    assert!(!desk.core.is_expanded(&tools));
    // A zone that is not rolled up has nothing to open.
    assert_eq!(
        desk.does(Command::Expand {
            zone: plain.clone(),
            expanded: Some(true)
        }),
        []
    );
    assert!(!desk.core.is_expanded(&plain));

    // Rolled down for good, it is no longer "open for now"; and with no
    // delay set, the pointer opens at once.
    desk.does(Command::Expand {
        zone: tools.clone(),
        expanded: Some(true),
    });
    desk.does(Command::RollUp {
        zone: tools.clone(),
        rolled_up: Some(false),
    });
    let scene = zone_scene(&desk, &tools);
    assert!(!scene.rolled_up && !scene.expanded);
    settings(&mut desk, json!({"roll_up": {"expand_delay_ms": 0}}));
    modify(
        &mut desk,
        &tools,
        json!({"expand_on": null, "rolled_up": true}),
    );
    let effects = over(&mut desk, Some(&tools), 20_000);
    assert_eq!(
        events(&effects),
        [ZonesEvent::ZoneExpanded {
            zone: tools.clone(),
            expanded: true
        }]
    );

    // Tabs are one thing: they open and close together.
    desk.does(Command::Tab {
        zone: plain.clone(),
        with: tools.clone(),
    });
    desk.does(Command::Expand {
        zone: tools.clone(),
        expanded: Some(false),
    });
    let effects = desk.does(Command::Expand {
        zone: plain.clone(),
        expanded: Some(true),
    });
    assert_eq!(events(&effects).len(), 2);
    assert!(desk.core.is_expanded(&tools) && desk.core.is_expanded(&plain));
}

#[test]
fn tabs_are_put_in_order_gone_round_and_split_off() {
    let mut desk = Desk::new();
    let place = Rect::new(100, 100, 400, 300);
    let a = desk.zone("A", place);
    let b = desk.zone("B", Rect::new(600, 100, 300, 200));
    let c = desk.zone("C", Rect::new(600, 400, 300, 200));
    desk.does(Command::Tab {
        zone: b.clone(),
        with: a.clone(),
    });
    desk.does(Command::Tab {
        zone: c.clone(),
        with: a.clone(),
    });
    assert_eq!(
        zone_scene(&desk, &c).tab,
        Some(TabPlace { index: 2, count: 3 })
    );
    assert!(zone_scene(&desk, &c).front);

    // The last tab becomes the first: the place stays the group's.
    let effects = desk.does(Command::MoveTab {
        zone: c.clone(),
        index: 0,
    });
    assert!(effects.contains(&Effect::SaveConfig) && effects.contains(&Effect::SaveState));
    let order: Vec<&ZoneId> = desk
        .core
        .config()
        .zones
        .iter()
        .map(|zone| &zone.id)
        .collect();
    assert_eq!(order, [&c, &a, &b]);
    assert_eq!(
        zone_scene(&desk, &c).tab,
        Some(TabPlace { index: 0, count: 3 })
    );
    for zone in [&a, &b, &c] {
        assert_eq!(desk.rect(zone), place);
    }
    assert_eq!(
        desk.does(Command::MoveTab {
            zone: c.clone(),
            index: 0
        }),
        []
    );
    desk.does(Command::MoveTab {
        zone: c.clone(),
        index: 99,
    });
    assert_eq!(
        zone_scene(&desk, &c).tab,
        Some(TabPlace { index: 2, count: 3 })
    );
    desk.does(Command::MoveTab {
        zone: c.clone(),
        index: 0,
    });

    // Round the tabs with a key, either way, over the ends.
    assert!(zone_scene(&desk, &c).front);
    desk.does(Command::NextTab {
        zone: a.clone(),
        backwards: false,
    });
    assert!(zone_scene(&desk, &a).front);
    desk.does(Command::NextTab {
        zone: a.clone(),
        backwards: true,
    });
    assert!(zone_scene(&desk, &c).front);
    desk.does(Command::NextTab {
        zone: a.clone(),
        backwards: true,
    });
    assert!(zone_scene(&desk, &b).front);

    // A tab dragged out stands where it was let go, as large as the tabs.
    let effects = desk.does(Command::SplitTab {
        zone: b.clone(),
        x: 900,
        y: 500,
        snap: false,
    });
    assert!(events(&effects).contains(&ZonesEvent::ZonePlaced {
        zone: b.clone(),
        rect: Rect::new(900, 500, 400, 300)
    }));
    assert_eq!(zone_scene(&desk, &b).tab, None);
    assert_eq!(desk.rect(&a), place);
    assert_eq!(
        zone_scene(&desk, &a).tab,
        Some(TabPlace { index: 1, count: 2 })
    );
    assert!(refusal(&desk.ask(Command::SplitTab {
        zone: b.clone(),
        x: 0,
        y: 0,
        snap: false
    }))
    .is_some());
    assert!(refusal(&desk.ask(Command::MoveTab {
        zone: b.clone(),
        index: 0
    }))
    .is_some());
    assert!(refusal(&desk.ask(Command::NextTab {
        zone: b.clone(),
        backwards: false
    }))
    .is_some());

    // Locked where they are, the tabs stay together.
    desk.does(Command::Lock {
        zone: a.clone(),
        geometry: Some(true),
        members: None,
    });
    assert!(refusal(&desk.ask(Command::SplitTab {
        zone: c.clone(),
        x: 0,
        y: 0,
        snap: false
    }))
    .is_some());
    assert_eq!(
        zone_scene(&desk, &c).tab,
        Some(TabPlace { index: 0, count: 2 })
    );
}

#[test]
fn what_is_said_to_stay_stays_when_the_rest_is_put_away_and_that_is_remembered() {
    let mut desk = Desk::new();
    let kept = desk.zone("Kept", Rect::new(100, 100, 300, 300));
    let away = desk.zone("Away", Rect::new(500, 100, 300, 300));
    let todo = desk.file("todo.txt");
    desk.loose("loose.txt");
    desk.put(&away, &todo);

    let effects = desk.does(Command::QuickHide { hidden: Some(true) });
    assert!(
        effects.contains(&Effect::SaveState),
        "a desktop that was cleared stays clear"
    );
    assert!(desk.core.state().quick_hidden);
    desk.does(Command::AlwaysVisible {
        zone: kept.clone(),
        on: None,
    });
    assert!(zone_scene(&desk, &kept).shown && zone_scene(&desk, &kept).always_visible);
    assert!(!zone_scene(&desk, &away).shown);

    // One item of a zone that is away stays too.
    let effects = desk.does(Command::ItemAlwaysVisible {
        item: todo.clone(),
        on: Some(true),
    });
    assert!(effects.contains(&Effect::SaveState));
    assert!(zone_scene(&desk, &away).items.items[0].always_visible);
    assert_eq!(
        desk.does(Command::ItemAlwaysVisible {
            item: todo.clone(),
            on: Some(true)
        }),
        []
    );
    assert!(refusal(&desk.ask(Command::ItemAlwaysVisible {
        item: id("nothing"),
        on: None
    }))
    .is_some());
    let loose = &desk.core.scene(NOW).loose[0];
    assert!(!loose.items[0].always_visible);

    // Its mark follows it through a rename, and through a restart.
    desk.does(Command::RenameItem {
        item: todo.clone(),
        name: "done.txt".to_string(),
    });
    let renamed = id("done.txt");
    assert_eq!(
        desk.core.state().always_visible_items,
        [renamed.clone()].into()
    );
    let again = Desk::on(
        desk.core.config().clone(),
        desk.core.state().clone(),
        desk.backend,
    );
    let scene = again.core.scene(NOW);
    assert!(scene.quick_hidden);
    assert!(zone_scene(&again, &away).items.items[0].always_visible);
    let mut desk = again;
    // When the item goes, nothing of it is left behind.
    desk.backend.remove(&renamed);
    desk.pump();
    assert!(desk.core.state().always_visible_items.is_empty());
    desk.does(Command::AlwaysVisible {
        zone: kept.clone(),
        on: Some(false),
    });
    assert!(!zone_scene(&desk, &kept).shown);
}

#[test]
fn peeking_ends_when_something_is_opened_and_escape_does_the_nearest_thing() {
    let mut desk = Desk::new();
    let papers = desk.zone("Papers", Rect::new(100, 100, 400, 300));
    let report = desk.file("report.pdf");
    desk.put(&papers, &report);
    desk.does(Command::QuickHide { hidden: Some(true) });

    let effects = desk.does(Command::TogglePeek);
    assert_eq!(
        events(&effects),
        [ZonesEvent::VisibilityChanged {
            quick_hidden: true,
            peeking: true
        }]
    );
    assert!(zone_scene(&desk, &papers).shown);
    // Opening is asked of whoever runs the zones, and the peek is over.
    let effects = desk.does(Command::Open {
        items: vec![report.clone()],
        with: None,
        in_place: true,
    });
    assert_eq!(
        events(&effects),
        [
            ZonesEvent::OpenRequested {
                items: vec![report.clone()],
                with: None
            },
            ZonesEvent::VisibilityChanged {
                quick_hidden: true,
                peeking: false
            },
        ]
    );
    assert!(!zone_scene(&desk, &papers).shown);
    let effects = desk.does(Command::Open {
        items: vec![report.clone(), id("gone"), report.clone()],
        with: Some("reader".to_string()),
        in_place: true,
    });
    assert_eq!(
        events(&effects),
        [ZonesEvent::OpenRequested {
            items: vec![report.clone()],
            with: Some("reader".to_string())
        }]
    );
    assert!(refusal(&desk.ask(Command::Open {
        items: vec![id("gone")],
        with: None,
        in_place: true
    }))
    .is_some());
    desk.does(Command::TogglePeek);
    desk.does(Command::TogglePeek);
    assert!(!desk.core.scene(NOW).peeking);

    // Escape: the peek, then the filter, then the choice, then the keyboard.
    desk.does(Command::QuickHide {
        hidden: Some(false),
    });
    desk.does(Command::FocusItem {
        item: Some(report.clone()),
    });
    desk.does(Command::Filter {
        zone: None,
        typed: "rep".to_string(),
    });
    desk.does(Command::Peek { on: true });
    assert_eq!(
        events(&desk.does(Command::Dismiss)),
        [ZonesEvent::VisibilityChanged {
            quick_hidden: false,
            peeking: false
        }]
    );
    assert!(
        events(&desk.does(Command::Dismiss)).contains(&ZonesEvent::FilterChanged {
            zone: papers.clone(),
            typed: String::new()
        })
    );
    assert_eq!(
        events(&desk.does(Command::Dismiss)),
        [ZonesEvent::SelectionChanged { items: vec![] }]
    );
    assert_eq!(
        events(&desk.does(Command::Dismiss)),
        [ZonesEvent::FocusChanged {
            zone: None,
            item: None
        }]
    );
    assert_eq!(desk.does(Command::Dismiss), []);
}

#[test]
fn everything_is_locked_at_once_whatever_each_zone_says() {
    let mut desk = sorting_desk(false);
    let pictures = ZoneId::new("pictures");
    let effects = desk.does(Command::LockAll {
        geometry: None,
        members: None,
    });
    assert!(
        events(&effects).contains(&ZonesEvent::SettingsChanged)
            && effects.contains(&Effect::SaveConfig)
    );
    let all = Lock {
        geometry: true,
        members: true,
    };
    assert_eq!(desk.core.config().lock, all);
    assert_eq!(
        desk.core.config().zone(&pictures).unwrap().lock,
        Lock::default(),
        "its own lock is as it was"
    );
    assert_eq!(
        (desk.core.scene(NOW).lock, zone_scene(&desk, &pictures).lock),
        (all, all)
    );

    assert!(refusal(&desk.ask(Command::MoveZone {
        zone: pictures.clone(),
        x: 5,
        y: 5,
        snap: false
    }))
    .is_some());
    let cat = id("cat.png");
    assert!(refusal(&desk.ask(Command::AddItem {
        zone: pictures.clone(),
        item: cat.clone(),
        index: None,
        cell: None
    }))
    .is_some());
    // The rules hold back what would go into a zone.
    let plan = desk.core.plan(Scope::All, NOW);
    assert!(plan
        .steps
        .iter()
        .all(|step| matches!(step.action, RuleAction::MoveToFolder { .. })));
    assert!(plan
        .held
        .iter()
        .any(|held| held.item == cat && held.reason == Reason::Locked));

    desk.does(Command::LockAll {
        geometry: Some(false),
        members: None,
    });
    assert_eq!(
        desk.core.config().lock,
        Lock {
            geometry: false,
            members: true
        }
    );
    desk.does(Command::MoveZone {
        zone: pictures.clone(),
        x: 5,
        y: 5,
        snap: false,
    });
    // With neither named: both on unless both are, then both off.
    desk.does(Command::LockAll {
        geometry: None,
        members: None,
    });
    assert_eq!(desk.core.config().lock, all);
    desk.does(Command::LockAll {
        geometry: None,
        members: None,
    });
    assert_eq!(desk.core.config().lock, Lock::default());
    desk.put(&pictures, &cat);
    assert_eq!(
        desk.does(Command::LockAll {
            geometry: Some(false),
            members: Some(false)
        }),
        []
    );
}

#[test]
fn a_zones_look_is_its_own_over_the_general_one_and_both_are_changed_by_a_patch() {
    let config = ZonesConfig {
        look: ZoneLook {
            opacity: Some(60),
            title_bar: Some(TitleBar::Plain),
            ..ZoneLook::default()
        },
        zones: vec![ZoneDefinition {
            look: ZoneLook {
                opacity: Some(90),
                icon_saturation: Some(40),
                ..ZoneLook::default()
            },
            ..ZoneDefinition::manual("media", "Media")
        }],
        ..ZonesConfig::default()
    };
    let mut desk = Desk::with(config, ZonesState::default());
    let media = ZoneId::new("media");
    let look = zone_scene(&desk, &media).look;
    assert_eq!(
        (look.opacity, look.title_bar, look.icon_saturation),
        (Some(90), Some(TitleBar::Plain), Some(40))
    );
    // What neither says is left to the theme.
    assert_eq!(
        (look.labels, look.background.clone(), look.blur),
        (None, None, None)
    );

    let effects = modify(
        &mut desk,
        &media,
        json!({"title": "Films", "look": {"opacity": null, "labels": false, "icon_tint": "#ff000040"}}),
    );
    assert!(events(&effects).contains(&ZonesEvent::ZoneChanged {
        zone: media.clone()
    }));
    assert!(effects.contains(&Effect::SaveConfig));
    let scene = zone_scene(&desk, &media);
    assert_eq!(scene.title, "Films");
    assert_eq!(
        (scene.look.opacity, scene.look.labels),
        (Some(60), Some(false))
    );
    assert_eq!(scene.look.icon_tint.as_deref(), Some("#ff000040"));
    assert_eq!(modify(&mut desk, &media, json!({"title": "Films"})), []);

    // A patch that cannot be right says where, and changes nothing.
    let before = desk.core.config().clone();
    for (changes, says) in [
        (json!({"look": {"icon_tint": "red"}}), "look.icon_tint"),
        (json!({"icon_size": 4}), "icon_size"),
        (json!({"id": "other"}), "keeps its id"),
        (json!({"view": "pile"}), "no zone"),
        (json!("locked"), "object"),
    ] {
        let reason = refusal(&desk.ask(Command::ModifyZone {
            zone: media.clone(),
            changes,
        }))
        .unwrap();
        assert!(reason.contains(says), "{reason}");
    }
    assert_eq!(desk.core.config(), &before);

    let effects = settings(
        &mut desk,
        json!({"look": {"blur": true}, "snap": {"grid": 16}}),
    );
    assert!(
        events(&effects).contains(&ZonesEvent::SettingsChanged)
            && effects.contains(&Effect::SaveConfig)
    );
    assert_eq!(desk.core.config().snap.grid, 16);
    assert_eq!(zone_scene(&desk, &media).look.blur, Some(true));
    assert_eq!(settings(&mut desk, json!({"snap": {"grid": 16}})), []);
    let before = desk.core.config().clone();
    for (changes, says) in [
        (json!({"zones": []}), "one by one"),
        (json!({"snap": {"grid": -5}}), "snap.grid"),
        (json!({"roll_up": {"expand_on": "wave"}}), "no settings"),
        (json!(7), "object"),
    ] {
        let reason = refusal(&desk.ask(Command::ModifySettings { changes })).unwrap();
        assert!(reason.contains(says), "{reason}");
    }
    assert_eq!(desk.core.config(), &before);
    // The grid that was set is the one a moved zone comes to rest on.
    desk.does(Command::MoveZone {
        zone: media.clone(),
        x: 403,
        y: 409,
        snap: true,
    });
    assert_eq!((desk.rect(&media).x, desk.rect(&media).y), (400, 416));
}

// ---- milestone 4 ----------------------------------------------------------

/// Downloads with a file, and a folder for the year with a file and a
/// folder for the month in it.
fn downloads() -> FakeBackend {
    let mut backend = FakeBackend::new();
    backend.add_folder("/downloads");
    backend.add_file_in("/downloads", "setup.exe");
    backend.add_folder_in("/downloads", "2026");
    backend.add_file_in("/downloads/2026", "invoice.pdf");
    backend.add_folder_in("/downloads/2026", "nov");
    backend.add_file_in("/downloads/2026/nov", "a.txt");
    backend
}

#[test]
fn a_portal_walks_into_its_folders_and_back_and_never_above_its_own() {
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), downloads());
    let zone = portal(&mut desk, "Downloads", "/downloads");
    assert_eq!(desk.names_in(&zone), ["2026", "setup.exe"]);
    let shown = zone_scene(&desk, &zone).portal.unwrap();
    assert_eq!(
        (shown.root.as_str(), shown.directory.as_str()),
        ("/downloads", "/downloads")
    );
    assert!(!shown.can_go_up && !shown.can_go_back && !shown.can_go_forward);
    assert_eq!((shown.crumbs.len(), shown.listing), (1, Listing::Ready));

    let year = ItemId::new("/downloads/2026");
    let effects = desk.does(Command::EnterFolder {
        zone: zone.clone(),
        item: year.clone(),
    });
    assert!(events(&effects).contains(&ZonesEvent::Navigated {
        zone: zone.clone(),
        directory: "/downloads/2026".to_string()
    }));
    assert!(!saves(&effects), "where a portal stands is written nowhere");
    assert_eq!(desk.names_in(&zone), ["invoice.pdf", "nov"]);
    // The folder it stands in is the one that is watched.
    assert!(desk.backend.is_watched("/downloads/2026") && !desk.backend.is_watched("/downloads"));
    let shown = zone_scene(&desk, &zone).portal.unwrap();
    let way: Vec<&str> = shown
        .crumbs
        .iter()
        .map(|crumb| crumb.name.as_str())
        .collect();
    assert_eq!(way, ["downloads", "2026"]);
    assert!(shown.can_go_up && shown.can_go_back && shown.listing.is_ready());
    desk.backend.add_file_in("/downloads/2026", "new.txt");
    assert_eq!(
        events(&desk.pump()),
        [ZonesEvent::ItemsChanged {
            zone: Some(zone.clone())
        }]
    );
    desk.backend.add_file_in("/downloads", "elsewhere.txt");
    assert_eq!(desk.pump(), []);

    // Opening a folder of a portal goes into it; a file is opened.
    let month = ItemId::new("/downloads/2026/nov");
    let effects = desk.does(Command::Open {
        items: vec![month.clone()],
        with: None,
        in_place: true,
    });
    assert!(events(&effects)
        .iter()
        .any(|event| matches!(event, ZonesEvent::Navigated { .. })));
    assert_eq!(desk.names_in(&zone), ["a.txt"]);
    let file = ItemId::new("/downloads/2026/nov/a.txt");
    let effects = desk.does(Command::Open {
        items: vec![file.clone()],
        with: None,
        in_place: true,
    });
    assert_eq!(
        events(&effects),
        [ZonesEvent::OpenRequested {
            items: vec![file],
            with: None
        }]
    );

    let stands = |desk: &Desk| zone_scene(desk, &zone).portal.unwrap().directory;
    let go = |desk: &mut Desk, command: fn(ZoneId) -> Command| desk.ask(command(zone.clone()));
    go(&mut desk, |zone| Command::NavigateUp { zone });
    assert_eq!(stands(&desk), "/downloads/2026");
    go(&mut desk, |zone| Command::NavigateBack { zone });
    assert_eq!(stands(&desk), "/downloads/2026/nov");
    go(&mut desk, |zone| Command::NavigateBack { zone });
    go(&mut desk, |zone| Command::NavigateBack { zone });
    assert_eq!(stands(&desk), "/downloads");
    assert!(refusal(&go(&mut desk, |zone| Command::NavigateBack { zone })).is_some());
    assert!(refusal(&go(&mut desk, |zone| Command::NavigateUp { zone })).is_some());
    go(&mut desk, |zone| Command::NavigateForward { zone });
    assert_eq!(stands(&desk), "/downloads/2026");
    desk.does(Command::NavigateTo {
        zone: zone.clone(),
        directory: "/downloads/2026/nov".to_string(),
    });
    assert!(refusal(&go(&mut desk, |zone| Command::NavigateForward { zone })).is_some());
    desk.does(Command::NavigateTo {
        zone: zone.clone(),
        directory: "/downloads".to_string(),
    });
    assert_eq!(desk.names_in(&zone), ["2026", "elsewhere.txt", "setup.exe"]);
    assert_eq!(
        desk.backend.watches(),
        2,
        "the desktop and the one folder shown"
    );

    // Not out of its own folder, not into a file, and only a portal walks.
    for command in [
        Command::NavigateTo {
            zone: zone.clone(),
            directory: "/etc".to_string(),
        },
        Command::NavigateTo {
            zone: zone.clone(),
            directory: "/downloads-of-another".to_string(),
        },
        Command::EnterFolder {
            zone: zone.clone(),
            item: ItemId::new("/downloads/setup.exe"),
        },
        Command::EnterFolder {
            zone: zone.clone(),
            item: month,
        },
    ] {
        assert!(refusal(&desk.ask(command.clone())).is_some(), "{command:?}");
    }
    let plain = desk.zone("Plain", Rect::new(0, 0, 200, 200));
    assert!(refusal(&desk.ask(Command::NavigateUp {
        zone: plain.clone()
    }))
    .is_some());
    assert_eq!(zone_scene(&desk, &plain).portal, None);

    // Another run of the program: the portal stands in its own folder.
    let again = Desk::on(
        desk.core.config().clone(),
        desk.core.state().clone(),
        desk.backend,
    );
    assert_eq!(
        zone_scene(&again, &zone).portal.unwrap().directory,
        "/downloads"
    );
}

#[test]
fn a_zone_is_filtered_by_what_is_typed_and_files_are_made_and_moved_where_a_portal_stands() {
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), downloads());
    let zone = portal(&mut desk, "Downloads", "/downloads");
    desk.does(Command::EnterFolder {
        zone: zone.clone(),
        item: ItemId::new("/downloads/2026"),
    });

    let effects = desk.does(Command::Filter {
        zone: Some(zone.clone()),
        typed: "inv".to_string(),
    });
    assert_eq!(
        events(&effects),
        [
            ZonesEvent::FilterChanged {
                zone: zone.clone(),
                typed: "inv".to_string()
            },
            ZonesEvent::ItemsChanged {
                zone: Some(zone.clone())
            },
        ]
    );
    assert!(!saves(&effects));
    assert_eq!(desk.names_in(&zone), ["invoice.pdf"]);
    let scene = zone_scene(&desk, &zone);
    assert_eq!(
        (
            scene.filter.as_deref(),
            scene.item_count,
            scene.items.items.len()
        ),
        (Some("inv"), 2, 1)
    );
    assert_eq!(desk.core.zone_holds(&zone, NOW).len(), 2);
    assert_eq!(
        desk.does(Command::Filter {
            zone: Some(zone.clone()),
            typed: "inv".to_string()
        }),
        []
    );
    // Walking on empties what was typed of the folder that was shown.
    let effects = desk.does(Command::NavigateUp { zone: zone.clone() });
    assert!(events(&effects).contains(&ZonesEvent::FilterChanged {
        zone: zone.clone(),
        typed: String::new()
    }));
    assert_eq!(zone_scene(&desk, &zone).filter, None);
    desk.does(Command::Filter {
        zone: Some(zone.clone()),
        typed: "set".to_string(),
    });
    desk.does(Command::Filter {
        zone: Some(zone.clone()),
        typed: "  ".to_string(),
    });
    assert_eq!(desk.names_in(&zone), ["2026", "setup.exe"]);
    assert!(refusal(&desk.ask(Command::Filter {
        zone: None,
        typed: "x".to_string()
    }))
    .is_some());

    // What is dropped on the portal goes into the folder it stands in.
    desk.does(Command::NavigateBack { zone: zone.clone() });
    let report = desk.file("report.pdf");
    desk.put(&zone, &report);
    assert_eq!(desk.names_in(&zone), ["invoice.pdf", "nov", "report.pdf"]);

    desk.does(Command::NewFolder {
        zone: zone.clone(),
        name: " Invoices ".to_string(),
    });
    assert_eq!(
        desk.names_in(&zone),
        ["Invoices", "invoice.pdf", "nov", "report.pdf"]
    );
    let invoice = ItemId::new("/downloads/2026/invoice.pdf");
    desk.does(Command::MoveItemsToFolder {
        items: vec![invoice.clone()],
        folder: "/downloads/2026/Invoices".to_string(),
        copy: false,
    });
    assert_eq!(desk.names_in(&zone), ["Invoices", "nov", "report.pdf"]);
    assert_eq!(desk.backend.in_folder("/downloads/2026/Invoices").len(), 1);
    // Each can be taken back, the last first: the move, then the folder.
    desk.does(Command::UndoFileOperation);
    assert_eq!(
        desk.names_in(&zone),
        ["Invoices", "invoice.pdf", "nov", "report.pdf"]
    );
    desk.does(Command::UndoFileOperation);
    assert_eq!(desk.names_in(&zone), ["invoice.pdf", "nov", "report.pdf"]);
    // Out of the portal onto the desktop, as a copy: both are there.
    desk.does(Command::MoveItemsToFolder {
        items: vec![invoice.clone()],
        folder: DESKTOP.to_string(),
        copy: true,
    });
    assert!(desk.core.item(&id("invoice.pdf")).is_some() && desk.core.item(&invoice).is_some());

    for command in [
        Command::NewFolder {
            zone: zone.clone(),
            name: "a/b".to_string(),
        },
        Command::NewFolder {
            zone: zone.clone(),
            name: " ".to_string(),
        },
        // A folder into itself, a file to where it is, nothing at all.
        Command::MoveItemsToFolder {
            items: vec![ItemId::new("/downloads/2026/nov")],
            folder: "/downloads/2026/nov".to_string(),
            copy: false,
        },
        Command::MoveItemsToFolder {
            items: vec![invoice],
            folder: "/downloads/2026".to_string(),
            copy: false,
        },
        Command::MoveItemsToFolder {
            items: vec![],
            folder: "/downloads".to_string(),
            copy: false,
        },
    ] {
        assert!(refusal(&desk.ask(command.clone())).is_some(), "{command:?}");
    }
    let plain = desk.zone("Plain", Rect::new(0, 0, 200, 200));
    assert!(refusal(&desk.ask(Command::NewFolder {
        zone: plain,
        name: "x".to_string()
    }))
    .is_some());

    // On the desktop itself, by its folder: made, and taken back the same way.
    let on_desk = desk.backend.in_folder(DESKTOP).len();
    desk.does(Command::NewFolderIn {
        folder: DESKTOP.to_string(),
        name: "Projects".to_string(),
    });
    assert_eq!(
        desk.core.item(&id("Projects")).map(|item| item.kind),
        Some(ItemKind::Folder)
    );
    desk.does(Command::UndoFileOperation);
    assert!(desk.core.item(&id("Projects")).is_none());
    assert_eq!(desk.backend.in_folder(DESKTOP).len(), on_desk);
    for (folder, name) in [(" ", "x"), (DESKTOP, "a\\b"), (DESKTOP, "")] {
        let command = Command::NewFolderIn {
            folder: folder.to_string(),
            name: name.to_string(),
        };
        assert!(refusal(&desk.ask(command)).is_some(), "{folder:?} {name:?}");
    }
}

#[test]
fn a_folder_that_is_slow_or_gone_says_so_and_its_zone_stays() {
    let mut backend = FakeBackend::new();
    backend.add_folder("/nas");
    backend.add_file_in("/nas", "film.mkv");
    backend.set_slow("/nas", true);
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), backend);
    let zone = portal(&mut desk, "Films", "/nas");
    let listing = |desk: &Desk| zone_scene(desk, &zone).portal.unwrap().listing;

    // Asked for, and not answered yet: nobody waited for it.
    assert_eq!(listing(&desk), Listing::Loading);
    assert_eq!(desk.names_in(&zone), [] as [&str; 0]);
    desk.backend.answer();
    let heard = events(&desk.pump());
    assert!(heard.contains(&ZonesEvent::ListingChanged {
        zone: zone.clone(),
        listing: Listing::Ready
    }));
    assert_eq!(desk.names_in(&zone), ["film.mkv"]);

    // The share goes away: nothing is shown of it, and the zone is there.
    let place = desk.rect(&zone);
    desk.backend.set_reachable("/nas", false);
    let heard = events(&desk.pump());
    assert!(matches!(
        &heard[0],
        ZonesEvent::ListingChanged { listing: Listing::Unavailable { reason }, .. } if reason.contains("/nas")
    ));
    assert_eq!(desk.names_in(&zone), [] as [&str; 0]);
    assert_eq!(desk.rect(&zone), place);
    assert!(desk.core.config().zone(&zone).is_some());

    // Asked again while it is still away, and again when it is back.
    let effects = desk.does(Command::RefreshZone { zone: zone.clone() });
    assert_eq!(
        events(&effects),
        [ZonesEvent::ListingChanged {
            zone: zone.clone(),
            listing: Listing::Loading
        }]
    );
    desk.backend.answer();
    desk.pump();
    assert!(matches!(listing(&desk), Listing::Unavailable { .. }));
    desk.backend.set_reachable("/nas", true);
    desk.does(Command::RefreshZone { zone: zone.clone() });
    desk.backend.answer();
    desk.pump();
    assert_eq!(listing(&desk), Listing::Ready);
    assert_eq!(desk.names_in(&zone), ["film.mkv"]);

    // A folder that is not there at all is said as that, not as empty;
    // when it comes to be, asking again finds and watches it.
    let gone = portal(&mut desk, "Gone", "/not-there");
    assert!(matches!(
        zone_scene(&desk, &gone).portal.unwrap().listing,
        Listing::Unavailable { .. }
    ));
    desk.backend.add_folder("/not-there");
    desk.backend.add_file_in("/not-there", "now.txt");
    desk.does(Command::RefreshZone { zone: gone.clone() });
    assert_eq!(desk.names_in(&gone), ["now.txt"]);
    desk.backend.add_file_in("/not-there", "later.txt");
    desk.pump();
    assert_eq!(desk.names_in(&gone), ["later.txt", "now.txt"]);
    // Zones that show nothing from elsewhere have nothing to ask.
    let plain = desk.zone("Plain", Rect::new(0, 0, 200, 200));
    assert_eq!(desk.does(Command::RefreshZone { zone: plain }), []);
}

#[test]
fn a_dry_run_says_what_would_go_where_and_by_which_rule() {
    let mut desk = sorting_desk(false);
    let pictures = ZoneId::new("pictures");
    let papers = ZoneId::new("papers");
    let new = id("new.doc");
    desk.put(&pictures, &new);
    let before = (desk.core.config().clone(), desk.core.state().clone());
    let effects = desk.does(Command::PreviewRules { scope: Scope::All });
    let Some(ZonesEvent::RulesPreviewed { plan, lines }) = events(&effects).into_iter().next()
    else {
        panic!("no preview");
    };
    assert_eq!(
        (desk.core.config(), desk.core.state()),
        (&before.0, &before.1),
        "a dry run changes nothing"
    );
    assert_eq!(lines.len(), plan.steps.len() + plan.held.len());
    let said: Vec<(&str, Option<&ZoneId>, &Destination, &str)> = lines
        .iter()
        .map(|line| {
            (
                line.name.as_str(),
                line.from.as_ref(),
                &line.destination,
                line.rule.as_str(),
            )
        })
        .collect();
    let to_zone = |zone: &ZoneId, title: &str| Destination::Zone {
        zone: zone.clone(),
        title: title.to_string(),
    };
    assert_eq!(
        said,
        [
            ("cat.png", None, &to_zone(&pictures, "Pictures"), "pictures"),
            (
                "new.doc",
                Some(&pictures),
                &to_zone(&papers, "Papers"),
                "papers"
            ),
            (
                "old.doc",
                None,
                &Destination::Folder {
                    folder: "/archive".to_string()
                },
                "old papers"
            ),
        ]
    );
    assert!(lines.iter().all(|line| line.held.is_none()));
    assert_eq!(lines[1].decided, Decided::Rule(2));
    // Applying says the same lines, and does them.
    let effects = desk.does(Command::ApplyRules { scope: Scope::All });
    assert!(events(&effects).contains(&ZonesEvent::RulesApplied { plan, lines }));
    assert_eq!(desk.names_in(&papers), ["new.doc"]);

    // What is held back is said with the reason, and where it would have gone.
    desk.does(Command::UndoRules);
    desk.does(Command::Lock {
        zone: pictures.clone(),
        geometry: None,
        members: Some(true),
    });
    let lines = desk.core.explain(&desk.core.plan(Scope::All, NOW));
    let held: Vec<(&str, Option<&Reason>)> = lines
        .iter()
        .map(|line| (line.name.as_str(), line.held.as_ref()))
        .collect();
    assert_eq!(
        held,
        [
            ("old.doc", None),
            ("cat.png", Some(&Reason::Locked)),
            ("new.doc", Some(&Reason::Locked))
        ]
    );
    assert_eq!(lines[1].destination, to_zone(&pictures, "Pictures"));
}

#[test]
fn rules_that_ask_the_time_are_tried_on_a_schedule_the_host_keeps() {
    let stale = Rule {
        name: "Stale".to_string(),
        matchers: vec![Matcher::Untouched { days: 30 }],
        action: RuleAction::AssignToZone {
            zone: ZoneId::new("archive"),
        },
        ..Rule::default()
    };
    let texts = Rule {
        name: "Texts".to_string(),
        matchers: vec![Matcher::Extension {
            any_of: vec!["txt".to_string()],
        }],
        action: RuleAction::AssignToZone {
            zone: ZoneId::new("inbox"),
        },
        ..Rule::default()
    };
    let config = ZonesConfig {
        zones: vec![
            ZoneDefinition::manual("inbox", "Inbox"),
            ZoneDefinition::manual("archive", "Archive"),
        ],
        rules: vec![stale, texts],
        rules_on_arrival: false,
        ..ZonesConfig::default()
    };
    let mut backend = FakeBackend::new();
    for (name, age) in [("note.txt", 10), ("loose.txt", 12)] {
        let mut item = Item::file(&format!("{DESKTOP}/{name}"));
        item.facts.modified = Some(NOW - age * DAY);
        backend.add(DESKTOP, item);
    }
    let mut desk = Desk::on(config, ZonesState::default(), backend);
    let (inbox, archive) = (ZoneId::new("inbox"), ZoneId::new("archive"));
    let note = id("note.txt");
    desk.put(&inbox, &note);
    assert_eq!(desk.core.timed_rules_every(), Some(3600));

    // Nothing has come of age: nothing is said, nothing changes, and
    // what the other rules would do is not done by the way.
    assert_eq!(desk.does(Command::CheckTimedRules), []);
    assert_eq!(desk.names_in(&inbox), ["note.txt"]);
    assert!(desk.core.zone_of(&id("loose.txt")).is_none());

    // Twenty-five days on, both are a month old: also the one that was
    // sorted by hand.
    let later = NOW + 25 * DAY;
    let effects = desk.host.command(
        &mut desk.core,
        &mut desk.backend,
        Command::CheckTimedRules,
        later,
    );
    let Some(ZonesEvent::RulesApplied { plan, lines }) = events(&effects).into_iter().next() else {
        panic!("nothing was applied");
    };
    assert_eq!(plan.steps.len(), 2);
    assert!(lines.iter().all(|line| line.rule == "Stale"));
    assert_eq!(lines[1].from, Some(inbox.clone()));
    assert_eq!(desk.names_in(&archive), ["loose.txt", "note.txt"]);
    assert_eq!(
        desk.host.command(
            &mut desk.core,
            &mut desk.backend,
            Command::CheckTimedRules,
            later
        ),
        []
    );
    // It is an application like any other: it can be taken back.
    desk.does(Command::UndoRules);
    assert_eq!(desk.names_in(&inbox), ["note.txt"]);
    assert_eq!(desk.names_in(&archive), [] as [&str; 0]);

    // Never, where the configuration says so or no rule asks the time.
    settings(&mut desk, json!({"timed_rules": {"every_minutes": 0}}));
    assert_eq!(desk.core.timed_rules_every(), None);
    settings(&mut desk, json!({"timed_rules": {"every_minutes": 5}}));
    assert_eq!(desk.core.timed_rules_every(), Some(300));
    let only_texts = serde_json::to_value(&desk.core.config().rules[1..]).unwrap();
    settings(&mut desk, json!({"rules": only_texts}));
    assert_eq!(desk.core.timed_rules_every(), None);
}

#[test]
fn a_timed_rule_moves_the_file_itself_only_where_that_is_wanted() {
    let mut desk = sorting_desk(false);
    // "old papers" moves documents untouched for 90 days into /archive.
    assert_eq!(desk.does(Command::CheckTimedRules), []);
    assert_eq!(desk.backend.in_folder("/archive").len(), 0);
    settings(&mut desk, json!({"timed_rules": {"move_files": true}}));
    let effects = desk.does(Command::CheckTimedRules);
    assert!(
        matches!(&events(&effects)[0], ZonesEvent::RulesApplied { plan, .. } if plan.steps.len() == 1)
    );
    assert_eq!(desk.backend.in_folder("/archive").len(), 1);
    assert!(desk.core.item(&id("old.doc")).is_none());
    // The rules that ask nothing of the time were left alone.
    assert!(desk.core.zone_of(&id("cat.png")).is_none());
    desk.does(Command::UndoRules);
    assert!(desk.core.item(&id("old.doc")).is_some());
    assert_eq!(desk.backend.in_folder("/archive").len(), 0);
}

fn smart(
    desk: &mut Desk,
    title: &str,
    source: serde_json::Value,
    more: serde_json::Value,
) -> ZoneId {
    let zone = make(desk, title, ZoneKind::Smart { query: Vec::new() });
    let mut changes = json!({ "source": source });
    crate::core::merge(&mut changes, &more);
    modify(desk, &zone, changes);
    zone
}

fn app(name: &str) -> Item {
    Item {
        id: ItemId::new(format!("app:{}", name.to_lowercase())),
        kind: ItemKind::Launcher,
        name: name.to_string(),
        ..Item::default()
    }
}

#[test]
fn zones_fill_themselves_from_what_the_system_knows_and_nothing_of_it_is_kept() {
    let mut backend = FakeBackend::new();
    let recent = |names: &[&str]| {
        names
            .iter()
            .map(|name| Item::file(&format!("/home/{name}")))
            .collect::<Vec<_>>()
    };
    backend.set_source(SmartSource::Recent, recent(&["b.txt", "a.png", "c.txt"]));
    let mut today = Item::file("/dl/today.zip");
    today.facts.modified = Some(NOW - 3600);
    let mut last_week = Item::file("/dl/old.zip");
    last_week.facts.modified = Some(NOW - 6 * DAY);
    backend.set_source(SmartSource::Downloads, vec![last_week, today]);
    backend.add_folder("/shots");
    backend.add_file_in("/shots", "one.png");
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), backend);
    let watched = desk.backend.watches();

    // The files used last: in the source's order, the first two.
    let last = smart(
        &mut desk,
        "Recent",
        json!({"from": "recent"}),
        json!({"limit": 2}),
    );
    assert_eq!(desk.backend.watches(), watched + 1);
    assert_eq!(desk.names_in(&last), ["b.txt", "a.png"]);
    assert_eq!(zone_scene(&desk, &last).source, Some(Listing::Ready));
    assert_eq!(zone_scene(&desk, &last).item_count, 2);
    desk.backend.set_source(
        SmartSource::Recent,
        recent(&["new.txt", "b.txt", "a.png", "c.txt"]),
    );
    assert!(events(&desk.pump()).contains(&ZonesEvent::ItemsChanged {
        zone: Some(last.clone())
    }));
    assert_eq!(desk.names_in(&last), ["new.txt", "b.txt"]);
    // A query narrows what the source has; a sort orders it otherwise.
    modify(
        &mut desk,
        &last,
        json!({"limit": 0, "sort": "name", "kind": {"type": "smart", "query": [{"match": "extension", "any_of": ["txt"]}]}}),
    );
    assert_eq!(desk.names_in(&last), ["b.txt", "c.txt", "new.txt"]);
    assert!(desk.core.item(&ItemId::new("/home/c.txt")).is_some());

    // Today's downloads: tomorrow they are yesterday's, by themselves.
    let fresh = smart(
        &mut desk,
        "Today",
        json!({"from": "downloads"}),
        json!({"kind": {"type": "smart", "query": [{"match": "date", "field": "modified", "newer_than_days": 1}]}}),
    );
    assert_eq!(desk.names_in(&fresh), ["today.zip"]);
    assert!(desk.core.zone_items(&fresh, NOW + 2 * DAY).is_empty());

    // The programs that run: the backend does not know them, whoever
    // hosts the core does and says so, and the backend does not overrule.
    let running = smart(
        &mut desk,
        "Running",
        json!({"from": "running_apps"}),
        json!({}),
    );
    assert!(matches!(
        zone_scene(&desk, &running).source,
        Some(Listing::Unavailable { .. })
    ));
    let told = BackendEvent::SourceListed {
        source: SmartSource::RunningApps,
        items: vec![app("Firefox"), app("Terminal")],
    };
    let heard = events(&desk.core.event(told, NOW));
    assert!(heard.contains(&ZonesEvent::ListingChanged {
        zone: running.clone(),
        listing: Listing::Ready
    }));
    assert_eq!(desk.names_in(&running), ["Firefox", "Terminal"]);
    desk.does(Command::RefreshZone {
        zone: running.clone(),
    });
    assert_eq!(desk.names_in(&running), ["Firefox", "Terminal"]);
    assert_eq!(zone_scene(&desk, &running).source, Some(Listing::Ready));

    // Any folder, with nothing asked of it: all it has, kept up to date.
    let shots = smart(
        &mut desk,
        "Shots",
        json!({"from": "folder", "directory": "/shots"}),
        json!({}),
    );
    desk.backend.add_file_in("/shots", "two.png");
    desk.pump();
    assert_eq!(desk.names_in(&shots), ["one.png", "two.png"]);
    // The desktop, with nothing asked of it, shows nothing (it would be
    // the whole desktop again); with a query, what fits, where it lies.
    let pictures = make(&mut desk, "Pictures", ZoneKind::Smart { query: Vec::new() });
    let cat = desk.file("cat.png");
    assert_eq!(desk.names_in(&pictures), [] as [&str; 0]);
    assert_eq!(zone_scene(&desk, &pictures).source, None);
    modify(
        &mut desk,
        &pictures,
        json!({"kind": {"type": "smart", "query": [{"match": "extension", "any_of": ["png"]}]}}),
    );
    assert_eq!(desk.names_in(&pictures), ["cat.png"]);
    assert!(
        desk.core.zone_of(&cat).is_none(),
        "it is shown there and is still loose"
    );

    // All of it is found, each thing with the zone that shows it.
    let hits = desk.core.search("today", NOW);
    assert_eq!((hits.len(), hits[0].zone.as_ref()), (1, Some(&fresh)));
    assert_eq!(desk.core.search("fire", NOW)[0].zone, Some(running.clone()));
    let hit = &desk.core.search("cat", NOW)[0];
    assert_eq!(
        (hit.zone.as_ref(), hit.also_in.as_slice()),
        (None, [pictures.clone()].as_slice())
    );
    assert_eq!(desk.core.search("two", NOW)[0].zone, Some(shots.clone()));

    // Nothing of what these zones show is ever written: no placement,
    // and none of their items anywhere in the state.
    assert!(refusal(&desk.ask(Command::AddItem {
        zone: last.clone(),
        item: cat.clone(),
        index: None,
        cell: None
    }))
    .is_some());
    let written = store::encode(desk.core.state());
    for name in ["b.txt", "today.zip", "app:firefox", "one.png", "cat.png"] {
        assert!(!written.contains(name), "{name} is in the state file");
    }
    assert!(desk.core.state().placements.zones.is_empty());

    // A source no zone looks through is let go, and a late word of it is
    // passed over.
    let watched = desk.backend.watches();
    desk.does(Command::DeleteZone { zone: last });
    assert_eq!(desk.backend.watches(), watched - 1);
    let late = BackendEvent::SourceListed {
        source: SmartSource::Recent,
        items: recent(&["late.txt"]),
    };
    assert_eq!(desk.core.event(late, NOW), []);
    assert!(desk.core.item(&ItemId::new("/home/late.txt")).is_none());
    // A source that fails says so, and shows nothing it cannot vouch for.
    let error = BackendError::Refused {
        what: "the folder of downloads".to_string(),
    };
    let failed = BackendEvent::SourceFailed {
        source: SmartSource::Downloads,
        error,
    };
    desk.core.event(failed, NOW);
    assert!(matches!(
        zone_scene(&desk, &fresh).source,
        Some(Listing::Unavailable { .. })
    ));
    assert_eq!(desk.names_in(&fresh), [] as [&str; 0]);
}

// ---- milestone 5 ----------------------------------------------------------

#[test]
fn a_zone_is_shown_on_the_workspaces_it_names() {
    let mut desk = Desk::new();
    let code = desk.zone("Code", Rect::new(100, 100, 300, 300));
    let always = desk.zone("Always", Rect::new(500, 100, 300, 300));
    modify(&mut desk, &code, json!({"workspaces": ["dev", "2"]}));
    assert!(
        zone_scene(&desk, &code).shown,
        "a workspace that was never told hides nothing"
    );
    desk.does(Command::FocusZone {
        zone: Some(code.clone()),
    });

    let workspace = |desk: &mut Desk, name: &str| {
        let effects = desk.core.event(
            BackendEvent::Workspace {
                name: Some(name.to_string()),
            },
            NOW,
        );
        assert_eq!(desk.core.broken(), None);
        events(&effects)
    };
    let heard = workspace(&mut desk, "mail");
    assert!(heard.contains(&ZonesEvent::LayoutChanged));
    // Nothing that is not shown has the keyboard.
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    assert!(!zone_scene(&desk, &code).shown && zone_scene(&desk, &always).shown);
    assert_eq!(desk.core.scene(NOW).workspace.as_deref(), Some("mail"));
    assert!(refusal(&desk.ask(Command::FocusZone {
        zone: Some(code.clone())
    }))
    .is_some());
    // Peeking shows what the one key put away, not another workspace's zones.
    desk.does(Command::Peek { on: true });
    assert!(!zone_scene(&desk, &code).shown);
    desk.does(Command::Peek { on: false });
    workspace(&mut desk, "2");
    assert!(zone_scene(&desk, &code).shown);

    // Each workspace its own arrangement as well: what one hides, the
    // other shows, and each keeps that through a change of displays.
    settings(&mut desk, json!({"profile_per_workspace": true}));
    desk.does(Command::ShowZone {
        zone: always.clone(),
        shown: false,
    });
    workspace(&mut desk, "dev");
    desk.does(Command::ShowZone {
        zone: always.clone(),
        shown: true,
    });
    assert!(zone_scene(&desk, &always).shown);
    desk.backend.plug(second());
    desk.pump();
    assert!(zone_scene(&desk, &always).shown);
    workspace(&mut desk, "2");
    assert!(
        !zone_scene(&desk, &always).shown,
        "the workspace hid it before the display came"
    );
}

#[test]
fn a_whole_layout_is_carried_out_and_in_and_what_was_there_is_kept() {
    let mut desk = sorting_desk(false);
    let pictures = ZoneId::new("pictures");
    desk.put(&pictures, &id("cat.png"));
    desk.does(Command::SaveSnapshot {
        name: "Work".to_string(),
    });
    let work = desk.core.current_layout("Work", NOW);

    let effects = desk.does(Command::ExportLayout {
        name: Some("Work".to_string()),
    });
    let Some(ZonesEvent::LayoutExported { document }) = events(&effects).into_iter().next() else {
        panic!("nothing was exported");
    };
    assert!(!saves(&effects));
    assert_eq!(
        (document.format.as_str(), document.exported_at),
        (LAYOUT_FORMAT, NOW)
    );
    assert!(document.layout.same_layout(&work));
    // As text and back, which is how it is carried.
    let text = serde_json::to_string_pretty(&document).unwrap();
    let carried: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        serde_json::from_value::<LayoutDocument>(carried.clone()).unwrap(),
        *document
    );
    let effects = desk.does(Command::ExportLayout { name: None });
    assert!(
        matches!(&events(&effects)[0], ZonesEvent::LayoutExported { document } if document.layout.name == "Layout")
    );
    assert!(refusal(&desk.ask(Command::ExportLayout {
        name: Some("none".to_string())
    }))
    .is_some());

    // The desktop is another by now.
    desk.does(Command::DeleteZone {
        zone: pictures.clone(),
    });
    let extra = desk.zone("Extra", Rect::new(0, 0, 200, 200));
    let there_was = desk.core.current_layout("", NOW);

    // A document that cannot be right changes nothing at all.
    let before = (desk.core.config().clone(), desk.core.state().clone());
    let mut newer = carried.clone();
    newer["version"] = 9.into();
    let mut twice = carried.clone();
    let zone = twice["layout"]["zones"][0].clone();
    twice["layout"]["zones"].as_array_mut().unwrap().push(zone);
    for (document, says) in [
        (newer, "version 9"),
        (twice, "zones[2].id"),
        (json!({"zones": []}), "no layout"),
    ] {
        let reason = refusal(&desk.ask(Command::ImportLayout {
            document,
            apply: true,
        }))
        .unwrap();
        assert!(reason.contains(says), "{reason}");
    }
    assert_eq!(
        (desk.core.config(), desk.core.state()),
        (&before.0, &before.1)
    );

    // Taken in: in force, under a name of its own, and what was there is kept.
    let effects = desk.does(Command::ImportLayout {
        document: carried.clone(),
        apply: true,
    });
    assert!(events(&effects).contains(&ZonesEvent::LayoutImported {
        name: "Work (2)".to_string(),
        applied: true,
        kept_as: Some("Before import".to_string()),
    }));
    assert!(effects.contains(&Effect::SaveConfig) && effects.contains(&Effect::SaveState));
    assert_eq!(desk.core.config().zones, work.zones);
    assert_eq!(desk.core.state().placements, work.placements);
    assert_eq!(desk.names_in(&pictures), ["cat.png"]);
    assert_eq!(desk.core.zone_rect(&extra), None);
    let snapshots = &desk.core.state().snapshots;
    let names: Vec<(&str, bool)> = snapshots
        .iter()
        .map(|kept| (kept.name.as_str(), kept.automatic))
        .collect();
    assert_eq!(
        names,
        [
            ("Work", false),
            ("Before import", true),
            ("Work (2)", false)
        ]
    );
    assert!(
        snapshots[1].same_layout(&there_was),
        "the layout there was is whole in its snapshot"
    );
    // And can be had back, which keeps nothing more (a snapshot holds it).
    desk.does(Command::RestoreSnapshot {
        name: "Before import".to_string(),
    });
    assert!(desk.core.zone_rect(&extra).is_some());

    // Taken in without putting it in force: one more snapshot, no more.
    let config = desk.core.config().clone();
    let effects = desk.does(Command::ImportLayout {
        document: carried,
        apply: false,
    });
    assert!(events(&effects).contains(&ZonesEvent::LayoutImported {
        name: "Work (3)".to_string(),
        applied: false,
        kept_as: None,
    }));
    assert_eq!(desk.core.config(), &config);

    // A name of the person's own makes a snapshot theirs to keep.
    desk.does(Command::RenameSnapshot {
        name: "Before import".to_string(),
        to: "Old desk".to_string(),
    });
    assert!(desk
        .core
        .state()
        .snapshots
        .iter()
        .any(|kept| kept.name == "Old desk" && !kept.automatic));
    for (name, to) in [("Old desk", "Work"), ("none", "x"), ("Work", " ")] {
        assert!(refusal(&desk.ask(Command::RenameSnapshot {
            name: name.to_string(),
            to: to.to_string()
        }))
        .is_some());
    }

    // Of the copies the zones take themselves only the last few stay;
    // the person's are never dropped.
    for round in 0..AUTOMATIC_KEPT + 3 {
        desk.zone(&format!("Round {round}"), Rect::new(0, 0, 200, 200));
        desk.does(Command::RestoreSnapshot {
            name: "Work".to_string(),
        });
    }
    let snapshots = &desk.core.state().snapshots;
    assert_eq!(
        snapshots.iter().filter(|kept| kept.automatic).count(),
        AUTOMATIC_KEPT
    );
    for name in ["Work", "Work (2)", "Work (3)", "Old desk"] {
        assert!(
            snapshots.iter().any(|kept| kept.name == name),
            "{name} is gone"
        );
    }
    // What is told to others of them.
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert_eq!(told.snapshots.len(), snapshots.len());
    assert!(told
        .snapshots
        .iter()
        .any(|kept| kept.name == "Work" && kept.zones == 2 && !kept.automatic));
}

#[test]
fn an_arrangement_is_taken_over_from_another_set_of_displays() {
    let mut desk = Desk::new();
    let at_home = Rect::new(100, 100, 300, 200);
    let zone = desk.zone("Papers", at_home);
    let alone = desk.core.profile().unwrap().key.clone();
    desk.backend.plug(second());
    desk.pump();
    desk.does(Command::MoveZone {
        zone: zone.clone(),
        x: 2400,
        y: 300,
        snap: false,
    });
    assert_eq!(desk.rect(&zone), Rect::new(2400, 300, 300, 200));
    let docked = desk.core.profile().unwrap().key.clone();
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert_eq!(told.profiles.len(), 2);
    assert!(told
        .profiles
        .iter()
        .any(|kept| kept.key == docked && kept.current && kept.zones == 1));

    let effects = desk.does(Command::CopyArrangement {
        from: alone.clone(),
    });
    assert!(
        events(&effects).contains(&ZonesEvent::LayoutChanged)
            && effects.contains(&Effect::SaveState)
    );
    assert_eq!(desk.rect(&zone), at_home);
    assert_eq!(
        desk.does(Command::CopyArrangement {
            from: docked.clone()
        }),
        []
    );
    let mut unknown = docked.clone();
    unknown.displays[0].width += 1;
    assert!(refusal(&desk.ask(Command::CopyArrangement { from: unknown })).is_some());
    desk.does(Command::LockAll {
        geometry: Some(true),
        members: None,
    });
    assert!(refusal(&desk.ask(Command::CopyArrangement { from: alone })).is_some());
}

#[test]
fn a_zone_that_asks_for_it_keeps_room_free_of_tiled_windows() {
    let mut desk = Desk::new();
    let display = DisplayId::new("fake-1");
    desk.backend.set_reserved(vec![ReservedArea {
        display: display.clone(),
        edge: Edge::Top,
        thickness: 40,
    }]);
    desk.pump();
    let dock = desk.zone("Dock", Rect::new(0, 40, 1920, 120));
    let side = desk.zone("Side", Rect::new(1620, 300, 300, 600));
    assert_eq!(desk.core.reserved_regions(), []);

    let region = |rect: Rect, edge: Edge, thickness: i32, zone: &ZoneId| ReservedRegion {
        display: display.clone(),
        rect,
        edge,
        thickness,
        zones: vec![zone.clone()],
    };
    let told = |effects: &[Effect]| {
        events(effects).into_iter().find_map(|event| match event {
            ZonesEvent::ReservedChanged { regions } => Some(regions),
            _ => None,
        })
    };
    let effects = modify(&mut desk, &dock, json!({"reserves_space": true}));
    let top = region(Rect::new(0, 40, 1920, 120), Edge::Top, 120, &dock);
    assert_eq!(told(&effects), Some(vec![top.clone()]));
    let effects = modify(&mut desk, &side, json!({"reserves_space": true}));
    let right = region(Rect::new(1620, 300, 300, 600), Edge::Right, 300, &side);
    assert_eq!(told(&effects), Some(vec![top.clone(), right.clone()]));
    assert_eq!(desk.core.scene(NOW).reserved, [top.clone(), right.clone()]);
    assert_eq!(ZonesSnapshot::of(&desk.core, NOW).reserved.len(), 2);
    // What is left of the free room for the windows.
    let free = Rect::new(0, 40, 1920, 1040);
    assert_eq!(
        crate::reserve::left_over(free, &display, &desk.core.reserved_regions()),
        Rect::new(0, 160, 1620, 920)
    );

    // Rolled up, the title's room; open for a moment, still that.
    let effects = desk.does(Command::RollUp {
        zone: dock.clone(),
        rolled_up: Some(true),
    });
    let title = region(Rect::new(0, 40, 1920, 28), Edge::Top, 28, &dock);
    assert_eq!(told(&effects), Some(vec![title.clone(), right.clone()]));
    assert_eq!(
        told(&desk.does(Command::Expand {
            zone: dock.clone(),
            expanded: Some(true)
        })),
        None
    );
    // Moved, the room moves; said only when it is other than it was.
    let effects = desk.does(Command::MoveZone {
        zone: side.clone(),
        x: 0,
        y: 300,
        snap: false,
    });
    let left = region(Rect::new(0, 300, 300, 600), Edge::Left, 300, &side);
    assert_eq!(told(&effects), Some(vec![title.clone(), left.clone()]));
    assert_eq!(
        told(&desk.does(Command::RenameZone {
            zone: side.clone(),
            title: "S".to_string()
        })),
        None
    );

    // What is not shown keeps no room; peeking changes nothing.
    assert_eq!(
        told(&desk.does(Command::QuickHide { hidden: Some(true) })),
        Some(vec![])
    );
    assert_eq!(told(&desk.does(Command::Peek { on: true })), None);
    desk.does(Command::Peek { on: false });
    assert_eq!(
        told(&desk.does(Command::QuickHide {
            hidden: Some(false)
        })),
        Some(vec![title.clone(), left.clone()])
    );
    assert_eq!(
        told(&desk.does(Command::ShowZone {
            zone: side.clone(),
            shown: false
        })),
        Some(vec![title])
    );
    assert_eq!(
        told(&desk.does(Command::DeleteZone { zone: dock })),
        Some(vec![])
    );
}

/// Three zones: one with five files top left, one with one file to its
/// right, an empty one below.
fn keyboard_desk() -> (Desk, ZoneId, ZoneId, ZoneId) {
    let mut desk = Desk::new();
    let a = desk.zone("A", Rect::new(100, 100, 400, 300));
    let b = desk.zone("B", Rect::new(600, 100, 400, 300));
    let c = desk.zone("C", Rect::new(100, 500, 400, 300));
    for number in 1..=5 {
        let item = desk.file(&format!("a{number}.txt"));
        desk.put(&a, &item);
    }
    let item = desk.file("b1.txt");
    desk.put(&b, &item);
    (desk, a, b, c)
}

#[test]
fn the_keyboard_goes_from_zone_to_zone_and_through_the_items_of_one() {
    let (mut desk, a, b, c) = keyboard_desk();
    let go =
        |desk: &mut Desk, direction: Direction| desk.does(Command::FocusNeighbour { direction });
    let walk = |desk: &mut Desk, step: Step, extend: bool| {
        desk.does(Command::MoveItemFocus { step, extend })
    };
    let at = |desk: &Desk| focus_of(desk).map(|(zone, item)| (ZoneId::new(zone), item));
    let on = |zone: &ZoneId, item: Option<&str>| {
        Some((zone.clone(), item.map(|name| id(name).to_string())))
    };

    // From nowhere, to the zone nearest the top left corner.
    let effects = go(&mut desk, Direction::Right);
    assert_eq!(
        events(&effects),
        [ZonesEvent::FocusChanged {
            zone: Some(a.clone()),
            item: None
        }]
    );
    assert!(!saves(&effects), "what has the keyboard is written nowhere");
    assert!(zone_scene(&desk, &a).focused && !zone_scene(&desk, &b).focused);
    for (direction, zone) in [
        (Direction::Right, &b),
        (Direction::Right, &b),
        (Direction::Left, &a),
        (Direction::Down, &c),
        (Direction::Down, &c),
        (Direction::Up, &a),
    ] {
        go(&mut desk, direction);
        assert_eq!(at(&desk), on(zone, None), "{direction:?}");
    }

    // Through the items, four to a row: a1 a2 a3 a4 / a5.
    let effects = walk(&mut desk, Step::Right, false);
    assert_eq!(
        events(&effects),
        [
            ZonesEvent::FocusChanged {
                zone: Some(a.clone()),
                item: Some(id("a1.txt"))
            },
            ZonesEvent::SelectionChanged {
                items: vec![id("a1.txt")]
            },
        ]
    );
    for (step, item) in [
        (Step::Right, "a2.txt"),
        (Step::Down, "a5.txt"),
        (Step::Down, "a5.txt"),
        (Step::Up, "a1.txt"),
        (Step::Last, "a5.txt"),
        (Step::Left, "a4.txt"),
        (Step::Right, "a5.txt"),
        (Step::First, "a1.txt"),
    ] {
        walk(&mut desk, step, false);
        assert_eq!(at(&desk), on(&a, Some(item)), "{step:?}");
        assert_eq!(desk.core.selection(), [id(item)]);
    }
    let scene = zone_scene(&desk, &a);
    let first = &scene.items.items[0];
    assert!(first.focused && first.selected && (first.position, first.of) == (1, 5));
    assert_eq!(
        (first.name.as_str(), first.kind),
        ("a1.txt", ItemKind::File)
    );
    assert!(!scene.items.items[1].focused && !scene.items.items[1].selected);
    assert_eq!(
        desk.core.scene(NOW).focus,
        Some(Focus::on_zone(&a, Some(id("a1.txt"))))
    );

    // Walking on while holding on to what was chosen.
    walk(&mut desk, Step::Right, true);
    assert_eq!(desk.core.selection(), [id("a1.txt"), id("a2.txt")]);
    desk.does(Command::Select {
        items: vec![id("a2.txt"), id("a4.txt"), id("gone")],
        mode: SelectMode::Toggle,
    });
    assert_eq!(desk.core.selection(), [id("a1.txt"), id("a4.txt")]);
    desk.does(Command::Select {
        items: vec![id("a2.txt")],
        mode: SelectMode::Add,
    });
    desk.does(Command::Select {
        items: vec![id("a4.txt")],
        mode: SelectMode::Remove,
    });
    assert_eq!(desk.core.selection(), [id("a1.txt"), id("a2.txt")]);

    // The chosen ones go to the zone on the right; the keyboard stays,
    // on what now stands where it stood.
    desk.does(Command::MoveFocusedItems {
        direction: Some(Direction::Right),
        zone: None,
    });
    assert_eq!(desk.names_in(&b), ["b1.txt", "a1.txt", "a2.txt"]);
    assert_eq!(desk.names_in(&a), ["a3.txt", "a4.txt", "a5.txt"]);
    assert_eq!(at(&desk), on(&a, Some("a4.txt")));
    let effects = desk.does(Command::OpenFocused);
    assert_eq!(
        events(&effects),
        [ZonesEvent::OpenRequested {
            items: vec![id("a4.txt")],
            with: None
        }]
    );
    desk.does(Command::MoveFocusedItems {
        direction: None,
        zone: Some(c.clone()),
    });
    assert_eq!(desk.names_in(&c), ["a4.txt"]);
    for (direction, zone) in [
        (Some(Direction::Left), None),
        (None, None),
        (None, Some(ZoneId::new("none"))),
    ] {
        assert!(refusal(&desk.ask(Command::MoveFocusedItems { direction, zone })).is_some());
    }

    // An item is gone to wherever it is: this is how a hit is gone to.
    desk.does(Command::FocusItem {
        item: Some(id("b1.txt")),
    });
    assert_eq!(at(&desk), on(&b, Some("b1.txt")));
    assert_eq!(desk.core.selection(), [id("b1.txt")]);
    desk.does(Command::FocusItem { item: None });
    assert_eq!(at(&desk), on(&b, None));
    assert!(refusal(&desk.ask(Command::FocusItem {
        item: Some(id("gone"))
    }))
    .is_some());
    desk.does(Command::FocusZone { zone: None });
    assert_eq!(at(&desk), None);
    assert!(refusal(&desk.ask(Command::OpenFocused)).is_some());
    // With nothing focused, a step begins at the first zone's first item.
    walk(&mut desk, Step::Down, false);
    assert_eq!(at(&desk), on(&a, Some("a3.txt")));
}

#[test]
fn the_focused_zone_can_be_named_as_such_and_the_keyboard_is_never_on_what_is_gone() {
    let (mut desk, a, b, _) = keyboard_desk();
    let focused = || ZoneId::new(FOCUSED);
    let reason = refusal(&desk.ask(Command::RollUp {
        zone: focused(),
        rolled_up: None,
    }))
    .unwrap();
    assert_eq!(reason, "no zone has the keyboard");

    desk.does(Command::FocusZone {
        zone: Some(a.clone()),
    });
    desk.does(Command::RollUp {
        zone: focused(),
        rolled_up: None,
    });
    assert!(desk.core.config().zone(&a).unwrap().rolled_up);
    desk.does(Command::Lock {
        zone: focused(),
        geometry: Some(true),
        members: None,
    });
    assert!(desk.core.config().zone(&a).unwrap().lock.geometry);
    desk.does(Command::Filter {
        zone: None,
        typed: "a3".to_string(),
    });
    assert_eq!(desk.names_in(&a), ["a3.txt"]);

    // Walking into the items of a rolled-up zone opens it for as long as
    // the keyboard is in it.
    let effects = desk.does(Command::MoveItemFocus {
        step: Step::Right,
        extend: false,
    });
    assert!(events(&effects).contains(&ZonesEvent::ZoneExpanded {
        zone: a.clone(),
        expanded: true
    }));
    assert_eq!(
        focus_of(&desk),
        Some((a.to_string(), Some(id("a3.txt").to_string())))
    );
    // An item the filter hides is gone to all the same: the filter gives way.
    let effects = desk.does(Command::FocusItem {
        item: Some(id("a5.txt")),
    });
    assert!(events(&effects).contains(&ZonesEvent::FilterChanged {
        zone: a.clone(),
        typed: String::new()
    }));
    // (The pointer comes and goes meanwhile: the keyboard holds it open.)
    desk.does(Command::PointerOver {
        zone: Some(a.clone()),
        at_ms: 100,
        dragging: false,
    });
    desk.does(Command::PointerOver {
        zone: None,
        at_ms: 200,
        dragging: false,
    });
    desk.does(Command::Tick { at_ms: 5000 });
    assert!(desk.core.is_expanded(&a));
    let effects = desk.does(Command::FocusNeighbour {
        direction: Direction::Right,
    });
    assert!(events(&effects).contains(&ZonesEvent::ZoneExpanded {
        zone: a.clone(),
        expanded: false
    }));
    assert_eq!(focus_of(&desk), Some((b.to_string(), None)));

    // The item goes: the zone keeps the keyboard. The zone goes: nothing has it.
    desk.does(Command::FocusItem {
        item: Some(id("b1.txt")),
    });
    desk.backend.remove(&id("b1.txt"));
    let heard = events(&desk.pump());
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: Some(b.clone()),
        item: None
    }));
    assert!(heard.contains(&ZonesEvent::SelectionChanged { items: vec![] }));
    let heard = events(&desk.does(Command::DeleteZone { zone: b.clone() }));
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    // Hidden zones cannot have it, and lose it when they are hidden.
    desk.does(Command::FocusZone {
        zone: Some(a.clone()),
    });
    let heard = events(&desk.does(Command::ShowZone {
        zone: a.clone(),
        shown: false,
    }));
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    assert!(refusal(&desk.ask(Command::FocusZone {
        zone: Some(a.clone())
    }))
    .is_some());
    assert!(refusal(&desk.ask(Command::FocusZone {
        zone: Some(ZoneId::new("none"))
    }))
    .is_some());
}

#[test]
fn what_is_private_is_covered_until_asked_for_and_again_when_the_screen_is_shared() {
    let mut desk = Desk::new();
    let bank = desk.zone("Bank", Rect::new(100, 100, 300, 300));
    let open = desk.zone("Open", Rect::new(500, 100, 300, 300));
    let secret = desk.file("secret.txt");
    let plain = desk.file("plain.txt");
    desk.loose("loose.txt");
    desk.put(&bank, &secret);
    desk.put(&open, &plain);
    modify(&mut desk, &bank, json!({"private": true}));
    let covered = |desk: &Desk, zone: &ZoneId| zone_scene(desk, zone).masked;
    let loose_covered = |desk: &Desk| desk.core.scene(NOW).loose[0].masked;

    // Covered: what it holds does not leave the core in the scene.
    let scene = zone_scene(&desk, &bank);
    assert!(scene.masked && scene.private && scene.items.items.is_empty());
    assert_eq!(scene.item_count, 1);
    assert!(!covered(&desk, &open) && !loose_covered(&desk));
    assert_eq!(desk.core.search("secret", NOW), []);
    assert_eq!(desk.core.search("plain", NOW).len(), 1);
    assert!(refusal(&desk.ask(Command::FocusItem {
        item: Some(secret.clone())
    }))
    .is_some());
    let tree = access::tree(&desk.core.scene(NOW));
    let node = tree
        .children
        .iter()
        .find(|node| node.name == "Bank")
        .unwrap();
    assert!(
        node.masked && node.children.is_empty() && node.description.contains("covered, 1 item")
    );
    assert!(!tree.walk().iter().any(|node| node.name == "secret.txt"));
    // A listing the person asks for names them, and says they are covered.
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert!(
        told.zone(bank.as_str()).unwrap().masked
            && told.zone(bank.as_str()).unwrap().items.len() == 1
    );

    // Asked for, it is shown.
    let effects = desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: None,
    });
    assert_eq!(
        events(&effects),
        [ZonesEvent::ZoneChanged { zone: bank.clone() }]
    );
    assert!(!saves(&effects));
    assert!(!covered(&desk, &bank) && zone_scene(&desk, &bank).private);
    assert_eq!(desk.core.search("secret", NOW).len(), 1);

    // The screen is shared: covered again, though it can be asked for.
    desk.backend.share_screen(true);
    let heard = events(&desk.pump());
    assert!(heard.contains(&ZonesEvent::PrivacyChanged {
        masked: false,
        screen_shared: true
    }));
    assert!(covered(&desk, &bank) && !covered(&desk, &open));
    assert!(desk.core.scene(NOW).screen_shared);
    desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: Some(true),
    });
    desk.backend.share_screen(false);
    desk.pump();
    assert!(
        !covered(&desk, &bank),
        "what was asked for while sharing stays when sharing ends"
    );

    // Everything, where that is wanted while sharing.
    settings(&mut desk, json!({"privacy": {"when_shared": "everything"}}));
    desk.backend.share_screen(true);
    desk.pump();
    assert!(covered(&desk, &bank) && covered(&desk, &open) && loose_covered(&desk));
    let loose = &desk.core.scene(NOW).loose[0];
    assert!(loose.items.is_empty() && loose.item_count == 1);
    assert_eq!(desk.core.search("loose", NOW), []);
    let effects = desk.does(Command::Reveal {
        zone: None,
        revealed: None,
    });
    assert!(events(&effects).contains(&ZonesEvent::PrivacyChanged {
        masked: false,
        screen_shared: true
    }));
    assert!(!covered(&desk, &bank) && !covered(&desk, &open) && !loose_covered(&desk));
    desk.does(Command::Reveal {
        zone: None,
        revealed: Some(false),
    });
    assert!(covered(&desk, &open) && loose_covered(&desk));
    desk.backend.share_screen(false);
    desk.pump();
    assert!(covered(&desk, &bank) && !covered(&desk, &open));

    // By hand, with a key: everything, and what had been asked for too.
    desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: Some(true),
    });
    let effects = desk.does(Command::Mask { masked: None });
    assert!(events(&effects).contains(&ZonesEvent::PrivacyChanged {
        masked: true,
        screen_shared: false
    }));
    assert!(covered(&desk, &bank) && covered(&desk, &open) && loose_covered(&desk));
    assert!(desk.core.scene(NOW).masked);
    desk.does(Command::Mask { masked: None });
    assert!(covered(&desk, &bank) && !covered(&desk, &open) && !loose_covered(&desk));

    // The keyboard is not on an item that is covered.
    desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: Some(true),
    });
    desk.does(Command::FocusItem {
        item: Some(secret.clone()),
    });
    let heard = events(&desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: Some(false),
    }));
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: Some(bank.clone()),
        item: None
    }));
    assert!(refusal(&desk.ask(Command::MoveItemFocus {
        step: Step::Right,
        extend: false
    }))
    .is_some());

    // Where sharing is to change nothing, it changes nothing; and all
    // zones are private where the configuration says everything is.
    settings(&mut desk, json!({"privacy": {"when_shared": "nothing"}}));
    desk.does(Command::Reveal {
        zone: Some(bank.clone()),
        revealed: Some(true),
    });
    desk.backend.share_screen(true);
    desk.pump();
    assert!(!covered(&desk, &bank));
    settings(&mut desk, json!({"privacy": {"everything": true}}));
    assert!(covered(&desk, &open) && loose_covered(&desk) && zone_scene(&desk, &open).private);
}

#[test]
fn the_system_is_asked_once_at_the_start_whether_the_screen_is_shared() {
    let mut backend = FakeBackend::new();
    backend.shared = Some(true);
    let config = ZonesConfig {
        zones: vec![ZoneDefinition {
            private: true,
            ..ZoneDefinition::manual("bank", "Bank")
        }],
        ..ZonesConfig::default()
    };
    let desk = Desk::on(config, ZonesState::default(), backend);
    assert!(desk.core.scene(NOW).screen_shared);
    assert!(ZonesSnapshot::of(&desk.core, NOW).screen_shared);
    // A backend that cannot say (the default) leaves it unsaid.
    assert!(!Desk::new().core.scene(NOW).screen_shared);
}

#[test]
fn a_screen_reader_is_told_what_is_drawn() {
    let (mut desk, a, b, c) = keyboard_desk();
    desk.loose("loose.txt");
    desk.does(Command::Tab {
        zone: c.clone(),
        with: b.clone(),
    });
    desk.does(Command::Lock {
        zone: a.clone(),
        geometry: None,
        members: Some(true),
    });
    desk.does(Command::FocusItem {
        item: Some(id("a2.txt")),
    });
    desk.does(Command::Filter {
        zone: Some(a.clone()),
        typed: "a".to_string(),
    });
    let scene: Scene = desk.core.scene(NOW);
    let tree = access::tree(&scene);
    assert_eq!((tree.role, tree.id.as_str()), (Role::Desktop, "desktop"));
    let roles: Vec<(Role, &str)> = tree
        .children
        .iter()
        .map(|node| (node.role, node.name.as_str()))
        .collect();
    assert_eq!(
        roles,
        [
            (Role::Zone, "A"),
            (Role::Zone, "B"),
            (Role::Zone, "C"),
            (Role::Loose, "Desktop items")
        ]
    );

    let zone = &tree.children[0];
    assert_eq!(zone.id, "zone:zone-1");
    assert_eq!(zone.description, "5 of 5 items shown, locked");
    assert_eq!(zone.value.as_deref(), Some("a"));
    assert!(zone.focused && zone.locked && !zone.hidden && zone.expanded.is_none());
    // Two zones can be walked to: A, and of the tabs the one in front.
    assert_eq!((zone.position, zone.of), (1, 2));
    assert_eq!(zone.children.len(), 5);
    let item = &zone.children[1];
    assert_eq!((item.role, item.name.as_str()), (Role::Item, "a2.txt"));
    assert_eq!(item.id, format!("zone:zone-1/item:{}", id("a2.txt")));
    assert!(item.focused && item.selected && (item.position, item.of) == (2, 5));
    // On the desktop: the zone's content area and the item's own place.
    let content = scene.zones[0].content;
    let placed = scene.zones[0].items.items[1].rect;
    assert_eq!(
        item.rect,
        Some(Rect::new(
            content.x + placed.x,
            content.y + placed.y,
            placed.width,
            placed.height
        ))
    );
    assert_eq!(content, Rect::new(108, 136, 384, 256));

    // Of tabs, the ones behind are there and said to be hidden.
    let (behind, front) = (&tree.children[1], &tree.children[2]);
    assert!(behind.hidden && !front.hidden);
    assert_eq!(behind.description, "tab 1 of 2, 1 item");
    assert_eq!((front.position, front.of), (2, 2));
    assert!(behind.children[0].hidden);
    let loose = &tree.children[3];
    assert_eq!(
        (loose.description.as_str(), loose.children.len()),
        ("1 item", 1)
    );
    assert_eq!(loose.children[0].name, "loose.txt");

    // Rolled up: its items are not to be walked to until it is open.
    desk.does(Command::FocusZone { zone: None });
    desk.does(Command::RollUp {
        zone: a.clone(),
        rolled_up: Some(true),
    });
    let rolled = access::tree(&desk.core.scene(NOW));
    assert_eq!(rolled.children[0].expanded, Some(false));
    assert!(rolled.children[0].children.iter().all(|item| item.hidden));
    desk.does(Command::Expand {
        zone: a.clone(),
        expanded: Some(true),
    });
    let open = access::tree(&desk.core.scene(NOW));
    assert_eq!(open.children[0].expanded, Some(true));
    assert!(open.children[0].children.iter().all(|item| !item.hidden));
    // The same thing has the same id from one scene to the next.
    assert_eq!(open.children[0].children[1].id, item.id);
    let text = serde_json::to_string(&open).unwrap();
    assert_eq!(
        serde_json::from_str::<access::AccessNode>(&text).unwrap(),
        open
    );
}

#[test]
fn what_is_told_to_others_has_all_of_it_and_everything_travels_as_json() {
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), downloads());
    let zone = portal(&mut desk, "Downloads", "/downloads");
    let papers = desk.zone("Papers", Rect::new(600, 100, 400, 300));
    let recent = make(&mut desk, "Recent", ZoneKind::Smart { query: Vec::new() });
    let report = desk.file("report.pdf");
    desk.put(&papers, &report);
    modify(&mut desk, &recent, json!({"source": {"from": "recent"}}));
    modify(
        &mut desk,
        &papers,
        json!({"private": true, "workspaces": ["2"], "rolled_up": true}),
    );
    desk.does(Command::EnterFolder {
        zone: zone.clone(),
        item: ItemId::new("/downloads/2026"),
    });
    desk.does(Command::Filter {
        zone: Some(zone.clone()),
        typed: "inv".to_string(),
    });
    desk.does(Command::FocusItem {
        item: Some(ItemId::new("/downloads/2026/invoice.pdf")),
    });
    desk.does(Command::LockAll {
        geometry: Some(true),
        members: None,
    });
    desk.does(Command::Expand {
        zone: papers.clone(),
        expanded: Some(true),
    });
    desk.does(Command::SaveSnapshot {
        name: "s".to_string(),
    });

    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert_eq!(told.version, crate::ipc::VERSION);
    assert_eq!(
        told.locked,
        Lock {
            geometry: true,
            members: false
        }
    );
    assert_eq!(told.focus.as_ref().map(|focus| &focus.zone), Some(&zone));
    assert_eq!(told.selection.len(), 1);
    assert_eq!((told.snapshots.len(), told.profiles.len()), (1, 1));
    assert!(!told.can_undo_rules && told.timed_rules_every.is_none());
    let shown = told.zone(zone.as_str()).unwrap();
    assert_eq!(shown.portal.as_ref().unwrap().directory, "/downloads/2026");
    assert_eq!(
        (shown.filter.as_deref(), shown.item_count, shown.items.len()),
        (Some("inv"), 2, 1)
    );
    assert!(shown.focused && shown.locked.geometry);
    let shown = told.zone(papers.as_str()).unwrap();
    assert!(shown.private && shown.masked && shown.expanded && shown.rolled_up);
    assert_eq!(shown.workspaces, ["2"]);
    let shown = told.zone(recent.as_str()).unwrap();
    assert_eq!(shown.source, Some(SmartSource::Recent));
    assert!(matches!(shown.listing, Some(Listing::Unavailable { .. })));
    let text = serde_json::to_string(&told).unwrap();
    assert_eq!(ZonesSnapshot::from_json(&text).unwrap(), told);
    // One written before any of this was added still reads.
    let old = r#"{"version":1,"enabled":true,"zones":[{"id":"a","title":"A","visible":true}],"loose":[]}"#;
    assert!(!ZonesSnapshot::from_json(old).unwrap().zones[0].masked);
    let scene = desk.core.scene(NOW);
    let text = serde_json::to_string(&scene).unwrap();
    assert_eq!(serde_json::from_str::<Scene>(&text).unwrap(), scene);

    // Every command there is to ask, as a request and back.
    let key = desk.core.profile().unwrap().key.clone();
    let item = ItemId::new("/desk/a.txt");
    let commands = vec![
        Command::PointerOver {
            zone: Some(zone.clone()),
            at_ms: 5,
            dragging: true,
        },
        Command::PointerOver {
            zone: None,
            at_ms: 6,
            dragging: false,
        },
        Command::Expand {
            zone: zone.clone(),
            expanded: None,
        },
        Command::Tick { at_ms: 7 },
        Command::MoveTab {
            zone: zone.clone(),
            index: 1,
        },
        Command::SplitTab {
            zone: zone.clone(),
            x: 1,
            y: 2,
            snap: true,
        },
        Command::NextTab {
            zone: zone.clone(),
            backwards: true,
        },
        Command::AlwaysVisible {
            zone: zone.clone(),
            on: Some(true),
        },
        Command::ItemAlwaysVisible {
            item: item.clone(),
            on: None,
        },
        Command::TogglePeek,
        Command::Dismiss,
        Command::Open {
            items: vec![item.clone()],
            with: Some("p".to_string()),
            in_place: false,
        },
        Command::LockAll {
            geometry: Some(true),
            members: None,
        },
        Command::ModifyZone {
            zone: zone.clone(),
            changes: json!({"look": {"blur": null}}),
        },
        Command::ModifySettings {
            changes: json!({"snap": {"grid": 8}}),
        },
        Command::EnterFolder {
            zone: zone.clone(),
            item: item.clone(),
        },
        Command::NavigateTo {
            zone: zone.clone(),
            directory: "/d".to_string(),
        },
        Command::NavigateUp { zone: zone.clone() },
        Command::NavigateBack { zone: zone.clone() },
        Command::NavigateForward { zone: zone.clone() },
        Command::RefreshZone { zone: zone.clone() },
        Command::NewFolder {
            zone: zone.clone(),
            name: "n".to_string(),
        },
        Command::MoveItemsToFolder {
            items: vec![item.clone()],
            folder: "/d".to_string(),
            copy: true,
        },
        Command::Filter {
            zone: None,
            typed: "t".to_string(),
        },
        Command::CheckTimedRules,
        Command::RenameSnapshot {
            name: "a".to_string(),
            to: "b".to_string(),
        },
        Command::ExportLayout { name: None },
        Command::ImportLayout {
            document: json!({"format": LAYOUT_FORMAT}),
            apply: false,
        },
        Command::CopyArrangement { from: key },
        Command::FocusZone { zone: None },
        Command::FocusNeighbour {
            direction: Direction::Up,
        },
        Command::FocusItem {
            item: Some(item.clone()),
        },
        Command::MoveItemFocus {
            step: Step::Last,
            extend: true,
        },
        Command::Select {
            items: vec![item.clone()],
            mode: SelectMode::Remove,
        },
        Command::OpenFocused,
        Command::MoveFocusedItems {
            direction: Some(Direction::Left),
            zone: None,
        },
        Command::Mask { masked: Some(true) },
        Command::Reveal {
            zone: None,
            revealed: None,
        },
        Command::GreyIcons { on: Some(true) },
        Command::AutoCreate {
            scope: Scope::All,
            min_items: Some(3),
            display: Some(DisplayId::new("fake-1")),
            add_rules: Some(false),
            dry_run: true,
        },
        Command::UndoAutoCreate,
    ];
    for command in commands {
        let written = Request::Command(command.clone()).to_value();
        assert_eq!(
            Request::from_value(written.clone()).unwrap(),
            Request::Command(command),
            "{written}"
        );
    }
    // What is left out is what one would expect.
    let short = |text: &str| Request::from_json(text).unwrap();
    assert_eq!(
        short(r#"{"command": "open", "items": ["/desk/a.txt"]}"#),
        Request::Command(Command::Open {
            items: vec![item.clone()],
            with: None,
            in_place: true
        })
    );
    assert_eq!(
        short(r#"{"command": "pointer_over", "at_ms": 9}"#),
        Request::Command(Command::PointerOver {
            zone: None,
            at_ms: 9,
            dragging: false
        })
    );
    assert_eq!(
        short(r#"{"command": "select", "items": []}"#),
        Request::Command(Command::Select {
            items: vec![],
            mode: SelectMode::Replace
        })
    );
    assert!(matches!(
        short(r#"{"command": "import_layout", "document": {}}"#),
        Request::Command(Command::ImportLayout { apply: true, .. })
    ));

    // Everything a busy desk said comes back from its JSON as it was.
    let mut effects = desk.does(Command::ExportLayout { name: None });
    effects.extend(desk.does(Command::Search {
        typed: "inv".to_string(),
    }));
    effects.extend(desk.does(Command::PreviewRules { scope: Scope::All }));
    effects.extend(desk.does(Command::Mask { masked: Some(true) }));
    effects.extend(desk.does(Command::NavigateUp { zone: zone.clone() }));
    effects.extend(modify(
        &mut desk,
        &papers,
        json!({"reserves_space": true, "workspaces": null}),
    ));
    effects.extend(desk.does(Command::FocusZone { zone: None }));
    effects.extend(desk.core.command(
        Command::RefreshZone {
            zone: recent.clone(),
        },
        NOW,
    ));
    effects.push(Effect::Ask(BackendRequest::Relist {
        directory: "/d".to_string(),
    }));
    assert!(effects.len() > 15);
    for effect in effects {
        let text = serde_json::to_string(&effect).unwrap();
        assert_eq!(
            serde_json::from_str::<Effect>(&text).unwrap(),
            effect,
            "{text}"
        );
    }
}

#[test]
fn a_state_file_from_before_reads_and_what_is_at_its_default_is_not_written() {
    // As milestone 2 wrote it: version 1, none of the later keys.
    let old = r#"{
        "version": 1,
        "placements": {"zones": {"papers": {"items": ["/desk/a.pdf"]}}, "loose": {}},
        "profiles": [],
        "active_tabs": {},
        "snapshots": [{"name": "s", "taken_at": 5, "zones": [], "rules": [], "placements": {"zones": {}, "loose": {}},
                       "profiles": [], "active_tabs": {}}],
        "zones_made": 3,
        "choices": 1
    }"#;
    let (state, from) = store::decode(old).unwrap();
    assert_eq!(from, None, "the version is the same: nothing is brought up");
    assert!(
        !state.quick_hidden
            && state.always_visible_items.is_empty()
            && !state.snapshots[0].automatic
    );
    let written = store::encode(&state);
    for key in ["quick_hidden", "always_visible_items", "automatic"] {
        assert!(!written.contains(key), "{key} is written at its default");
    }
    assert_eq!(store::decode(&written).unwrap().0, state);

    // What an older gevrek does with a newer file is what this one does
    // with keys it does not know: keep them, and write them back.
    let mut later = ZonesState {
        quick_hidden: true,
        ..state
    };
    later.always_visible_items.insert(id("todo.txt"));
    let text = store::encode(&later).replacen(
        "\"quick_hidden\"",
        "\"from_the_future\": [1], \"quick_hidden\"",
        1,
    );
    let (read, _) = store::decode(&text).unwrap();
    assert!(read.quick_hidden && read.always_visible_items.len() == 1);
    assert!(store::encode(&read).contains("from_the_future"));
}

// ---- asked for by the others along the way ---------------------------------

fn bring_in(desk: &mut Desk, name: &str, copy: bool, zone: &ZoneId) -> Vec<Effect> {
    desk.does(Command::BringIn {
        paths: vec![format!("/elsewhere/{name}")],
        folder: DESKTOP.to_string(),
        copy,
        zone: Some(zone.clone()),
        at: None,
    })
}

fn slow_desk() -> (Desk, ZoneId) {
    let mut backend = FakeBackend::new();
    backend.add_folder("/elsewhere");
    backend.add_file_in("/elsewhere", "big.iso");
    backend.add_file_in("/elsewhere", "other.iso");
    backend.slow_operations = true;
    let mut desk = Desk::on(ZonesConfig::default(), ZonesState::default(), backend);
    let papers = desk.zone("Papers", Rect::new(100, 100, 400, 300));
    (desk, papers)
}

#[test]
fn a_file_operation_that_takes_long_ends_later_and_is_shown_until_then() {
    let (mut desk, papers) = slow_desk();
    let effects = bring_in(&mut desk, "big.iso", true, &papers);
    let operation = FileOperation::Copy {
        from: "/elsewhere/big.iso".to_string(),
        to_folder: DESKTOP.to_string(),
    };
    assert_eq!(
        events(&effects),
        [ZonesEvent::OperationBegun {
            request: 1,
            operation: operation.clone()
        }]
    );
    // Under way: said in the scene with the zone it is for, and nothing
    // is there yet that is not there.
    let scene = desk.core.scene(NOW);
    assert_eq!(scene.operations.len(), 1);
    let shown = &scene.operations[0];
    assert_eq!(
        (shown.request, &shown.operation, shown.done, shown.total),
        (1, &operation, 0, 0)
    );
    assert_eq!(shown.zones, std::slice::from_ref(&papers));
    // Told to others as well, for calling it off by its number.
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert_eq!(told.operations, scene.operations);
    assert_eq!(
        ZonesSnapshot::from_json(&serde_json::to_string(&told).unwrap()).unwrap(),
        told
    );
    assert!(zone_scene(&desk, &papers).busy);
    assert!(desk.core.item(&id("big.iso")).is_none());
    desk.backend.progress(1, 30, 100);
    assert_eq!(
        events(&desk.pump()),
        [ZonesEvent::OperationProgress {
            request: 1,
            done: 30,
            total: 100
        }]
    );
    assert_eq!(desk.core.scene(NOW).operations[0].done, 30);
    // Word of an operation nobody asked for is passed over.
    desk.backend.progress(77, 1, 2);
    assert_eq!(desk.pump(), []);

    // It ends: the file is where it was meant to go, and can be taken back.
    desk.backend.finish_operations();
    let heard = events(&desk.pump());
    assert!(heard.contains(&ZonesEvent::OperationEnded { request: 1 }));
    assert_eq!(desk.names_in(&papers), ["big.iso"]);
    assert!(desk.core.scene(NOW).operations.is_empty() && !zone_scene(&desk, &papers).busy);
    let effects = desk.does(Command::UndoFileOperation);
    assert!(matches!(
        &events(&effects)[0],
        ZonesEvent::OperationBegun {
            request: 2,
            operation: FileOperation::Trash { .. }
        }
    ));
    // The item it is done to is named while it lasts.
    assert_eq!(desk.core.scene(NOW).operations[0].item, Some(id("big.iso")));
    desk.backend.finish_operations();
    desk.pump();
    assert_eq!(desk.names_in(&papers), [] as [&str; 0]);

    // It fails later: said, nothing came, and what was to be moved is
    // where it was.
    bring_in(&mut desk, "other.iso", false, &papers);
    desk.backend.fail_next = Some(BackendError::Refused {
        what: "the disk is full".to_string(),
    });
    desk.backend.finish_operations();
    let heard = events(&desk.pump());
    assert!(matches!(
        &heard[0],
        ZonesEvent::OperationFailed {
            error: BackendError::Refused { .. },
            ..
        }
    ));
    assert_eq!(heard[1], ZonesEvent::OperationEnded { request: 3 });
    assert!(desk.core.scene(NOW).operations.is_empty());
    assert!(desk.core.item(&id("other.iso")).is_none());
    assert_eq!(desk.backend.in_folder("/elsewhere").len(), 2);

    // It is called off: the same, by the person's wish.
    bring_in(&mut desk, "other.iso", false, &papers);
    let heard = events(&desk.does(Command::CancelOperation { request: 4 }));
    assert!(matches!(
        &heard[0],
        ZonesEvent::OperationFailed {
            error: BackendError::Cancelled,
            ..
        }
    ));
    assert_eq!(heard[1], ZonesEvent::OperationEnded { request: 4 });
    assert!(desk.backend.under_way.is_empty() && desk.core.scene(NOW).operations.is_empty());
    assert!(desk.core.item(&id("other.iso")).is_none());
    assert_eq!(desk.backend.in_folder("/elsewhere").len(), 2);
    assert!(refusal(&desk.ask(Command::CancelOperation { request: 4 })).is_some());
    assert!(refusal(&desk.ask(Command::CancelOperation { request: 99 })).is_some());

    // A backend that does it at once is asked as before, and nothing is
    // ever under way.
    desk.backend.slow_operations = false;
    let effects = bring_in(&mut desk, "other.iso", false, &papers);
    assert!(!events(&effects)
        .iter()
        .any(|event| matches!(event, ZonesEvent::OperationBegun { .. })));
    assert_eq!(desk.names_in(&papers), ["other.iso"]);
}

#[test]
fn a_rule_that_moves_a_file_slowly_and_is_called_off_leaves_the_file_shown_where_it_was() {
    let mut desk = sorting_desk(false);
    let papers = ZoneId::new("papers");
    let old = id("old.doc");
    desk.put(&papers, &old);
    desk.backend.slow_operations = true;
    desk.does(Command::ApplyRules { scope: Scope::All });
    let scene = desk.core.scene(NOW);
    let moving = scene
        .operations
        .iter()
        .find(|operation| operation.item == Some(old.clone()))
        .unwrap();
    assert!(
        desk.core.zone_of(&old).is_none(),
        "it is on its way out of its zone"
    );
    desk.does(Command::CancelOperation {
        request: moving.request,
    });
    assert_eq!(desk.core.zone_of(&old), Some(&papers));
    assert!(desk.core.scene(NOW).operations.is_empty());
    assert_eq!(desk.backend.in_folder("/archive").len(), 0);
}

#[test]
fn a_program_that_ends_while_a_file_is_on_its_way_loses_nothing_and_shows_nothing_that_is_not_there(
) {
    let (mut desk, papers) = slow_desk();
    let report = desk.file("report.pdf");
    desk.put(&papers, &report);
    bring_in(&mut desk, "big.iso", true, &papers);
    // The program ends here. What it wrote is what the next one reads.
    let (config, state) = (desk.core.config().clone(), desk.core.state().clone());
    let written = store::encode(&state);
    assert!(
        !written.contains("big.iso"),
        "what is on its way is in no file"
    );
    let mut again = Desk::on(config, store::decode(&written).unwrap().0, desk.backend);
    assert!(again.core.scene(NOW).operations.is_empty());
    assert!(
        again.core.item(&id("big.iso")).is_none(),
        "no item that is not there"
    );
    assert_eq!(again.names_in(&papers), ["report.pdf"]);
    // The copy ends all the same (the system was doing it): its answer is
    // for nobody, and the file is on the desktop like any that comes:
    // in the zone that takes what comes.
    again.backend.finish_operations();
    let heard = events(&again.pump());
    assert!(!heard
        .iter()
        .any(|event| matches!(event, ZonesEvent::OperationEnded { .. })));
    assert!(again.core.item(&id("big.iso")).is_some());
    assert_eq!(again.core.zone_of(&id("big.iso")), Some(&papers));
    assert_eq!(again.names_in(&papers), ["report.pdf", "big.iso"]);
    assert_eq!(
        again.backend.in_folder("/elsewhere").len(),
        2,
        "and the one it was copied from is there"
    );
}

#[test]
fn a_refusal_by_a_lock_says_whose_and_a_zone_says_whether_it_is_here() {
    let mut desk = Desk::new();
    let papers = desk.zone("Papers", Rect::new(100, 100, 400, 300));
    let other = desk.zone("Other", Rect::new(600, 100, 300, 300));
    let (inside, outside) = (desk.file("inside.txt"), desk.file("outside.txt"));
    desk.put(&papers, &inside);
    desk.does(Command::Lock {
        zone: papers.clone(),
        geometry: Some(true),
        members: Some(true),
    });
    let by_lock = |effects: &[Effect]| {
        let heard = events(effects);
        assert!(matches!(heard[0], ZonesEvent::Refused { .. }));
        heard
            .into_iter()
            .find(|event| matches!(event, ZonesEvent::RefusedByLock { .. }))
    };
    // What a refusal was about: the zone and item named, and whether a lock said no.
    let about = |effects: &[Effect]| {
        events(effects).into_iter().find_map(|event| match event {
            ZonesEvent::RefusedAbout {
                reason,
                zone,
                item,
                locked,
            } => {
                assert_eq!(Some(reason), refusal(effects));
                Some((zone, item, locked))
            }
            _ => None,
        })
    };
    let members = Some(ZonesEvent::RefusedByLock {
        zone: papers.clone(),
        geometry: false,
        members: true,
    });
    let geometry = Some(ZonesEvent::RefusedByLock {
        zone: papers.clone(),
        geometry: true,
        members: false,
    });
    let add = |zone: &ZoneId, item: &ItemId| Command::AddItem {
        zone: zone.clone(),
        item: item.clone(),
        index: None,
        cell: None,
    };
    assert_eq!(by_lock(&desk.ask(add(&papers, &outside))), members);
    // Taken out of it, to another zone or to the desktop: the lock is its.
    assert_eq!(by_lock(&desk.ask(add(&other, &inside))), members);
    assert_eq!(
        by_lock(&desk.ask(Command::RemoveItem {
            item: inside.clone()
        })),
        members
    );
    assert_eq!(
        by_lock(&desk.ask(Command::MoveZone {
            zone: papers.clone(),
            x: 0,
            y: 0,
            snap: false
        })),
        geometry
    );
    assert_eq!(
        by_lock(&desk.ask(Command::Tab {
            zone: other.clone(),
            with: papers.clone()
        })),
        geometry
    );
    // Refused for another reason, nothing is said of a lock.
    assert_eq!(
        by_lock(&desk.ask(Command::DeleteZone {
            zone: ZoneId::new("none")
        })),
        None
    );
    assert_eq!(by_lock(&desk.ask(add(&other, &id("gone")))), None);
    assert_eq!(
        about(&desk.ask(add(&other, &id("gone")))),
        Some((Some(other.clone()), Some(id("gone")), false))
    );
    // By a lock: the zone is the one that is locked, not the one named.
    assert_eq!(
        about(&desk.ask(add(&other, &inside))),
        Some((Some(papers.clone()), Some(inside.clone()), true))
    );
    assert_eq!(
        about(&desk.ask(Command::TrashItems {
            items: vec![inside.clone()]
        })),
        Some((None, Some(inside.clone()), false))
    );
    // A command about nothing in particular says nothing more.
    assert_eq!(about(&desk.ask(Command::UndoRules)), None);
    desk.does(Command::FocusZone {
        zone: Some(other.clone()),
    });
    let named_by_focus = desk.ask(Command::NavigateUp {
        zone: ZoneId::new(FOCUSED),
    });
    assert_eq!(
        about(&named_by_focus),
        Some((Some(other.clone()), None, false))
    );
    desk.does(Command::FocusZone { zone: None });

    // Whole and here.
    let scene = zone_scene(&desk, &papers);
    assert!(scene.here && scene.shown && scene.whole == scene.rect);
    desk.does(Command::RollUp {
        zone: other.clone(),
        rolled_up: Some(true),
    });
    let scene = zone_scene(&desk, &other);
    assert_eq!(
        (scene.rect, scene.whole),
        (Rect::new(600, 100, 300, 28), Rect::new(600, 100, 300, 300))
    );
    desk.does(Command::QuickHide { hidden: Some(true) });
    let scene = zone_scene(&desk, &other);
    assert!(
        scene.here && !scene.shown,
        "put away with the one key, it still belongs here"
    );
    desk.does(Command::ShowZone {
        zone: other.clone(),
        shown: false,
    });
    desk.does(Command::QuickHide {
        hidden: Some(false),
    });
    let scene = zone_scene(&desk, &other);
    assert!(!scene.here && !scene.shown);
}

#[test]
fn tabs_put_in_another_order_keep_their_place_on_every_set_of_displays() {
    let mut desk = Desk::new();
    let place = Rect::new(100, 100, 400, 300);
    let a = desk.zone("A", place);
    let b = desk.zone("B", Rect::new(600, 100, 300, 200));
    // Tabbed while another display is there: on the first set, B still
    // has the place it had by itself written down.
    desk.backend.plug(second());
    desk.pump();
    desk.does(Command::Tab {
        zone: b.clone(),
        with: a.clone(),
    });
    desk.does(Command::MoveTab {
        zone: b.clone(),
        index: 0,
    });
    assert_eq!((desk.rect(&a), desk.rect(&b)), (place, place));
    desk.backend.unplug(&DisplayId::new("fake-2"));
    desk.pump();
    assert_eq!(
        (desk.rect(&a), desk.rect(&b)),
        (place, place),
        "the first tab's old place is not the group's"
    );
}

#[test]
fn the_keyboard_walks_the_loose_items_of_the_bare_desktop_as_it_walks_a_zone() {
    let mut desk = Desk::new();
    let zone = desk.zone("A", Rect::new(600, 100, 400, 300));
    let inside = desk.file("inside.txt");
    desk.put(&zone, &inside);
    // Three loose items, laid down the first column of the display.
    let loose: Vec<ItemId> = (1..=3)
        .map(|number| desk.loose(&format!("l{number}.txt")))
        .collect();
    let display = DisplayId::new("fake-1");
    let the_desk = ZoneId::new(DESK);
    let go =
        |desk: &mut Desk, direction: Direction| desk.does(Command::FocusNeighbour { direction });
    let walk = |desk: &mut Desk, step: Step, extend: bool| {
        desk.does(Command::MoveItemFocus { step, extend })
    };
    let on_desk = |item: Option<&ItemId>| Some(Focus::on_desk(&display, item.cloned()));

    // From nowhere to what is nearest the top left corner: the loose items.
    let effects = go(&mut desk, Direction::Right);
    assert_eq!(
        events(&effects),
        [ZonesEvent::FocusChanged {
            zone: Some(the_desk.clone()),
            item: None
        }]
    );
    assert!(!saves(&effects));
    assert_eq!(desk.core.focus().cloned(), on_desk(None));
    let scene = desk.core.scene(NOW);
    assert!(scene.loose[0].focused && scene.focus == on_desk(None));
    assert!(access::tree(&scene)
        .children
        .iter()
        .any(|node| node.role == Role::Loose && node.focused));

    // Through them, to the nearest in the direction; at the end it stays.
    for (step, item) in [
        (Step::Down, 0),
        (Step::Down, 1),
        (Step::Down, 2),
        (Step::Down, 2),
        (Step::Right, 2),
        (Step::Up, 1),
        (Step::Last, 2),
        (Step::First, 0),
    ] {
        walk(&mut desk, step, false);
        assert_eq!(
            desk.core.focus().cloned(),
            on_desk(Some(&loose[item])),
            "{step:?}"
        );
        assert_eq!(desk.core.selection(), [loose[item].clone()]);
    }
    let placed = &desk.core.scene(NOW).loose[0].items[0];
    assert!(placed.focused && placed.selected && (placed.position, placed.of) == (1, 3));

    // Between the desktop and the zones, by where they are.
    go(&mut desk, Direction::Right);
    assert_eq!(focus_of(&desk), Some((zone.to_string(), None)));
    assert!(!desk.core.scene(NOW).loose[0].focused);
    go(&mut desk, Direction::Left);
    assert_eq!(desk.core.focus().cloned(), on_desk(None));
    go(&mut desk, Direction::Left);
    assert_eq!(
        desk.core.focus().cloned(),
        on_desk(None),
        "at the side it stays"
    );

    // Loose items into a zone that lies that way; the keyboard stays on
    // the desktop, on what is left.
    walk(&mut desk, Step::First, false);
    walk(&mut desk, Step::Down, true);
    assert_eq!(desk.core.selection(), [loose[0].clone(), loose[1].clone()]);
    desk.does(Command::MoveFocusedItems {
        direction: Some(Direction::Right),
        zone: None,
    });
    assert_eq!(desk.names_in(&zone), ["inside.txt", "l1.txt", "l2.txt"]);
    assert_eq!(desk.core.focus().cloned(), on_desk(Some(&loose[2])));
    // And out of a zone onto the desktop: by its name, or that way.
    desk.does(Command::FocusItem {
        item: Some(inside.clone()),
    });
    desk.does(Command::MoveFocusedItems {
        direction: None,
        zone: Some(the_desk.clone()),
    });
    assert_eq!(desk.core.zone_of(&inside), None);
    assert_eq!(
        focus_of(&desk),
        Some((zone.to_string(), Some(loose[0].to_string()))),
        "the next one in the zone"
    );
    desk.does(Command::MoveFocusedItems {
        direction: Some(Direction::Left),
        zone: None,
    });
    assert_eq!(desk.names_in(&zone), ["l2.txt"]);
    assert!(refusal(&desk.ask(Command::MoveFocusedItems {
        direction: None,
        zone: Some(the_desk.clone())
    }))
    .is_none());
    assert_eq!(desk.names_in(&zone), [] as [&str; 0]);
    // What lies loose already has nowhere to go on the desktop.
    desk.does(Command::FocusItem {
        item: Some(inside.clone()),
    });
    assert_eq!(desk.core.focus().cloned(), on_desk(Some(&inside)));
    assert!(refusal(&desk.ask(Command::MoveFocusedItems {
        direction: None,
        zone: Some(the_desk.clone())
    }))
    .is_some());
    let effects = desk.does(Command::OpenFocused);
    assert_eq!(
        events(&effects),
        [ZonesEvent::OpenRequested {
            items: vec![inside.clone()],
            with: None
        }]
    );

    // The desktop is no zone: what is asked of "the focused zone" is
    // refused in so many words.
    for command in [
        Command::RollUp {
            zone: ZoneId::new(FOCUSED),
            rolled_up: None,
        },
        Command::Lock {
            zone: ZoneId::new(FOCUSED),
            geometry: Some(true),
            members: None,
        },
        Command::Filter {
            zone: None,
            typed: "l".to_string(),
        },
    ] {
        assert_eq!(
            refusal(&desk.ask(command)).as_deref(),
            Some("the desktop has the keyboard, not a zone")
        );
    }
    assert!(refusal(&desk.ask(Command::RollUp {
        zone: the_desk.clone(),
        rolled_up: None
    }))
    .is_some());

    // The item goes: the desktop keeps the keyboard. Put away or
    // covered, the desktop cannot have it.
    desk.backend.remove(&inside);
    let heard = events(&desk.pump());
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: Some(the_desk.clone()),
        item: None
    }));
    let heard = events(&desk.does(Command::QuickHide { hidden: Some(true) }));
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    assert!(refusal(&desk.ask(Command::FocusZone {
        zone: Some(the_desk.clone())
    }))
    .is_some());
    assert!(refusal(&desk.ask(Command::FocusItem {
        item: Some(loose[2].clone())
    }))
    .is_some());
    desk.does(Command::Peek { on: true });
    desk.does(Command::FocusZone {
        zone: Some(the_desk.clone()),
    });
    desk.does(Command::Peek { on: false });
    assert_eq!(
        desk.core.focus(),
        None,
        "the peek is over, and the desktop is put away again"
    );
    desk.does(Command::QuickHide {
        hidden: Some(false),
    });
    desk.does(Command::FocusItem {
        item: Some(loose[2].clone()),
    });
    let heard = events(&desk.does(Command::Mask { masked: Some(true) }));
    assert!(heard.contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    desk.does(Command::Mask {
        masked: Some(false),
    });

    // Each display's loose items are a place of their own.
    desk.backend.plug(second());
    desk.pump();
    desk.does(Command::PlaceLooseItem {
        item: loose[2].clone(),
        display: DisplayId::new("fake-2"),
        x: 300,
        y: 200,
    });
    desk.does(Command::FocusZone {
        zone: Some(the_desk.clone()),
    });
    assert_eq!(desk.core.focus().cloned(), on_desk(None));
    desk.does(Command::DeleteZone { zone });
    go(&mut desk, Direction::Right);
    let other = DisplayId::new("fake-2");
    assert_eq!(
        desk.core.focus().cloned(),
        Some(Focus::on_desk(&other, None))
    );
    walk(&mut desk, Step::Down, false);
    assert_eq!(
        desk.core.focus().cloned(),
        Some(Focus::on_desk(&other, Some(loose[2].clone())))
    );
    // When the last loose item of a display goes, nothing is there to have it.
    desk.backend.remove(&loose[2]);
    assert!(events(&desk.pump()).contains(&ZonesEvent::FocusChanged {
        zone: None,
        item: None
    }));
    // What is told to others says the same.
    desk.does(Command::FocusZone {
        zone: Some(the_desk.clone()),
    });
    let told = ZonesSnapshot::of(&desk.core, NOW);
    assert_eq!(told.focus, on_desk(None));
    let text = serde_json::to_string(&told.focus).unwrap();
    assert_eq!(text, r#"{"zone":"@desk","desk":"fake-1"}"#);
}
