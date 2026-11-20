//! Bringing a configuration written by an older version to the current
//! layout, on the JSON before it is read into the model. Each step moves
//! from one schema version to the next; a file is never rewritten by
//! loading it, only when the user saves or applies.

use serde_json::Value;

/// Bring `config` (a whole configuration document) to the current schema
/// version. A document that is already current, or that is not shaped like
/// a configuration, is left as it is.
pub fn migrate(config: &mut Value) {
    if config.get("desktop").and_then(Value::as_object).is_none() {
        return;
    }
    let version = config["desktop"]
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if let Some((last, _)) = STEPS.last() {
        if version < u64::from(*last) {
            log::debug!("a configuration of schema {version} is brought to schema {last}");
        }
    }
    for (to, step) in STEPS {
        if version < u64::from(*to) {
            step(config);
            if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
                desktop.insert("schema_version".to_string(), Value::from(*to));
            }
        }
    }
}

/// A step takes the whole document (it may move things between `desktop`,
/// `node_graph` and the top level) from the version before to its own.
type Step = fn(&mut Value);

/// Every step, in order, with the schema version it brings a document to.
/// A new schema version adds one line here and its function below; the last
/// one is `DESKTOP_CONFIG_SCHEMA_VERSION` (a test holds them together).
/// Versions before 21 only ever added fields, which read with their
/// defaults, so they need no step.
const STEPS: &[(u32, Step)] = &[
    (21, |config| {
        if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
            to_21(desktop);
        }
    }),
    // 22: `desktop.input` is new; a file without it reads with its defaults.
    (22, |_| {}),
    (23, |config| {
        if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
            to_23(desktop);
        }
    }),
    // 24: floating windows have a border and glow of their own
    // (`appearance.floating`). A file from before gets the look that goes
    // with its own tiled look, not the one that goes with the defaults.
    (24, |config| {
        if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
            to_24(desktop);
        }
    }),
    // 25: a workspace has a name (what names it) and a title (what is
    // shown of it). A display's own entry "in the place of" a workspace
    // that every display has (`also_called`) is an entry for that
    // workspace, by its name, with what it was called as its title.
    (25, |config| {
        if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
            to_25(desktop);
        }
    }),
    // 26: the places windows snap to are `desktop.snap_zones`. They were
    // `desktop.zones`, a name that is something else from here on.
    (26, |config| {
        if let Some(desktop) = config.get_mut("desktop").and_then(Value::as_object_mut) {
            to_26(desktop);
        }
    }),
];

/// 25 -> 26: `desktop.zones` became `desktop.snap_zones`. Whatever a file
/// from before has under `zones` is of the snap zones, so all of it moves.
fn to_26(desktop: &mut serde_json::Map<String, Value>) {
    let Some(zones) = desktop.remove("zones") else {
        return;
    };
    // Snap zones already written in the new place win.
    desktop.entry("snap_zones").or_insert(zones);
}

/// What only snap zones have. A file of schema 26 or later that has one of
/// these under `desktop.zones` was written by hand in the old layout, and
/// is refused (see `Config::from_json`) rather than read as something else.
const SNAP_ZONE_KEYS: [&str; 3] = ["behavior", "default_layout", "monitor_layouts"];

/// Whether `config`, already brought to the current schema version, still
/// has snap zones under `desktop.zones`.
pub fn has_snap_zones_under_zones(config: &Value) -> bool {
    config
        .pointer("/desktop/zones")
        .and_then(Value::as_object)
        .is_some_and(|zones| SNAP_ZONE_KEYS.iter().any(|key| zones.contains_key(*key)))
}

/// 24 -> 25: `also_called` is gone. An entry that had it took the place,
/// on its display, of the workspace it named there: it is that
/// workspace's entry for the display now (the same `name`), and the name
/// it had is its `title`.
fn to_25(desktop: &mut serde_json::Map<String, Value>) {
    let Some(workspaces) = desktop.get_mut("workspaces").and_then(Value::as_array_mut) else {
        return;
    };
    for workspace in workspaces.iter_mut().filter_map(Value::as_object_mut) {
        let Some(also) = workspace.remove("also_called") else {
            continue;
        };
        let stands_for = also
            .as_array()
            .and_then(|names| names.iter().filter_map(Value::as_str).next())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let Some(stands_for) = stands_for else {
            continue;
        };
        let shown = workspace
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        workspace.insert("name".to_string(), Value::from(stands_for.clone()));
        if !shown.is_empty() && shown != stands_for && !workspace.contains_key("title") {
            workspace.insert("title".to_string(), Value::from(shown));
        }
    }
}

fn to_24(desktop: &mut serde_json::Map<String, Value>) {
    let Some(appearance) = desktop.get_mut("appearance") else {
        return;
    };
    if appearance.get("floating").is_some() {
        return;
    }
    // Read as the model reads it (what is missing has its default), so
    // that the look is derived from what the file means.
    let Ok(tiled) =
        serde_json::from_value::<crate::settings::AppearanceProfile>(appearance.clone())
    else {
        return;
    };
    let floating = crate::settings::FloatingAppearance::softer_than_tiled(&tiled);
    if let (Some(appearance), Ok(floating)) =
        (appearance.as_object_mut(), serde_json::to_value(floating))
    {
        appearance.insert("floating".to_string(), floating);
    }
}

/// 22 -> 23: `desktop.bar` became `desktop.shell.panel`, and what it
/// shows is a list for each of its sides instead of a switch for each
/// thing.
fn to_23(desktop: &mut serde_json::Map<String, Value>) {
    let Some(Value::Object(mut bar)) = desktop.remove("bar") else {
        return;
    };
    // A switch that was not written was on, but for the numbers.
    let mut shown = |key: &str, otherwise: bool| {
        bar.remove(key)
            .and_then(|value| value.as_bool())
            .unwrap_or(otherwise)
    };
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (key, module) in [
        ("show_workspaces", "workspaces"),
        ("show_layout", "layout"),
        ("show_title", "title"),
    ] {
        if shown(key, true) {
            left.push(Value::from(module));
        }
    }
    if shown("show_stats", false) {
        right.push(Value::from("stats"));
    }
    if shown("show_clock", true) {
        right.push(Value::from("clock"));
    }
    bar.insert("left".to_string(), Value::Array(left));
    bar.insert("centre".to_string(), Value::Array(Vec::new()));
    bar.insert("right".to_string(), Value::Array(right));
    if let Some(edge) = bar.get_mut("edge") {
        if let Some(word) = edge.as_str().map(str::to_lowercase) {
            *edge = Value::from(word);
        }
    }
    let shell = desktop
        .entry("shell")
        .or_insert_with(|| Value::Object(Default::default()));
    if let Some(shell) = shell.as_object_mut() {
        // A panel already written in the new place wins.
        shell.entry("panel").or_insert(Value::Object(bar));
    }
}

/// 20 -> 21: what only the Windows compositor applies moved from
/// `desktop.appearance` to `desktop.platform.windows`.
fn to_21(desktop: &mut serde_json::Map<String, Value>) {
    let mut moved = serde_json::Map::new();
    if let Some(appearance) = desktop.get_mut("appearance").and_then(Value::as_object_mut) {
        for key in ["backdrop", "corners", "shadow"] {
            if let Some(value) = appearance.remove(key) {
                moved.insert(key.to_string(), value);
            }
        }
    }
    if moved.is_empty() {
        return;
    }
    let platform = desktop
        .entry("platform")
        .or_insert_with(|| Value::Object(Default::default()));
    let Some(platform) = platform.as_object_mut() else {
        return;
    };
    let windows = platform
        .entry("windows")
        .or_insert_with(|| Value::Object(Default::default()));
    if let Some(windows) = windows.as_object_mut() {
        for (key, value) in moved {
            // A value already written in the new place wins.
            windows.entry(key).or_insert(value);
        }
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    #[test]
    fn the_bar_becomes_the_panel_with_what_it_showed_as_its_modules() {
        let mut document = serde_json::json!({ "desktop": { "schema_version": 22, "bar": {
            "enabled": true, "edge": "Bottom", "height": 30, "show_layout": false, "show_stats": true,
            "clock_format": "%H:%M", "from_later": 1 } } });
        migrate(&mut document);
        let desktop = &document["desktop"];
        assert!(desktop.get("bar").is_none(), "the old place is empty");
        let panel = &desktop["shell"]["panel"];
        assert_eq!(panel["left"], serde_json::json!(["workspaces", "title"]));
        assert_eq!(panel["right"], serde_json::json!(["stats", "clock"]));
        assert_eq!(panel["edge"], "bottom");
        assert_eq!(panel["height"], 30);
        assert_eq!(panel["from_later"], 1, "a key that is not known is kept");
        assert!(panel.get("show_layout").is_none());
        let read: crate::settings::DesktopConfig = serde_json::from_value(desktop.clone()).unwrap();
        assert_eq!(read.shell.panel.edge, crate::settings::PanelEdge::Bottom);
        assert!(read.shell.panel.shows(crate::settings::PanelModule::Stats));
        assert!(!read.shell.panel.shows(crate::settings::PanelModule::Layout));
        // A file without a bar reads with the panel as it is by default.
        let mut bare = serde_json::json!({ "desktop": { "schema_version": 22 } });
        migrate(&mut bare);
        assert!(bare["desktop"].get("shell").is_none());
    }

    #[test]
    fn a_module_from_a_later_version_does_not_stop_the_file_from_being_read() {
        let panel: crate::settings::PanelProfile =
            serde_json::from_value(serde_json::json!({ "right": ["moon_phase", "clock"] }))
                .unwrap();
        assert_eq!(
            panel.right,
            vec![
                crate::settings::PanelModule::Other("moon_phase".to_string()),
                crate::settings::PanelModule::Clock
            ]
        );
        // And is written back as it was.
        assert_eq!(
            serde_json::to_value(&panel).unwrap()["right"],
            serde_json::json!(["moon_phase", "clock"])
        );
    }

    #[test]
    fn the_last_step_is_the_current_schema_version_and_steps_are_in_order() {
        let versions: Vec<u32> = STEPS.iter().map(|(to, _)| *to).collect();
        assert_eq!(
            versions.last().copied(),
            Some(crate::settings::DESKTOP_CONFIG_SCHEMA_VERSION),
            "a new schema version needs a step in STEPS (even an empty one)"
        );
        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_document_from_any_older_version_ends_at_the_current_one() {
        for old in [0u64, 7, 12, 20, 25] {
            let mut document = serde_json::json!({ "desktop": { "schema_version": old } });
            migrate(&mut document);
            assert_eq!(
                document["desktop"]["schema_version"],
                serde_json::json!(crate::settings::DESKTOP_CONFIG_SCHEMA_VERSION)
            );
        }
        // Not a configuration: untouched.
        let mut other = serde_json::json!({ "nodes": [] });
        migrate(&mut other);
        assert_eq!(other, serde_json::json!({ "nodes": [] }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::platform::windows::{SystemBackdrop, WindowCorner};
    use crate::settings::{ShadowPolicy, DESKTOP_CONFIG_SCHEMA_VERSION};

    fn old_document() -> Value {
        let mut value = serde_json::to_value(Config::new()).unwrap();
        let desktop = value["desktop"].as_object_mut().unwrap();
        desktop.insert("schema_version".to_string(), Value::from(20));
        desktop.remove("platform");
        let appearance = desktop["appearance"].as_object_mut().unwrap();
        appearance.insert("backdrop".to_string(), Value::from("Mica"));
        appearance.insert("corners".to_string(), Value::from("DoNotRound"));
        appearance.insert("shadow".to_string(), Value::from("enabled"));
        value
    }

    #[test]
    fn a_version_20_file_keeps_its_windows_settings_in_their_new_place() {
        let config = Config::from_json(&old_document().to_string()).expect("loads");
        assert_eq!(config.desktop.schema_version, DESKTOP_CONFIG_SCHEMA_VERSION);
        assert_eq!(
            config.desktop.platform.windows.backdrop,
            SystemBackdrop::Mica
        );
        assert_eq!(
            config.desktop.platform.windows.corners,
            WindowCorner::DoNotRound
        );
        assert_eq!(
            config.desktop.platform.windows.shadow,
            ShadowPolicy::Enabled
        );
        // The rest of the appearance is untouched.
        assert_eq!(
            serde_json::to_value(&config.desktop.appearance).unwrap(),
            serde_json::to_value(Config::new().desktop.appearance).unwrap()
        );
    }

    #[test]
    fn what_is_saved_now_has_the_new_layout_and_loads_unchanged() {
        let mut config = Config::new();
        config.desktop.platform.windows.backdrop = SystemBackdrop::Acrylic;
        config.desktop.platform.wayland.xwayland = false;
        let saved = serde_json::to_value(&config).unwrap();
        assert!(saved["desktop"]["appearance"].get("backdrop").is_none());
        assert_eq!(
            saved["desktop"]["platform"]["windows"]["backdrop"],
            "Acrylic"
        );
        assert_eq!(saved["desktop"]["platform"]["wayland"]["xwayland"], false);
        let again = Config::from_json(&saved.to_string()).unwrap();
        assert_eq!(again.desktop.platform, config.desktop.platform);
    }

    #[test]
    fn a_current_document_and_a_foreign_one_are_left_alone() {
        let current = serde_json::to_value(Config::new()).unwrap();
        let mut copy = current.clone();
        migrate(&mut copy);
        assert_eq!(copy, current);
        let mut other = serde_json::json!({"hello": [1, 2, 3]});
        migrate(&mut other);
        assert_eq!(other, serde_json::json!({"hello": [1, 2, 3]}));
    }

    #[test]
    fn a_file_from_before_24_gets_the_floating_look_that_goes_with_its_own() {
        let mut document = serde_json::json!({
            "desktop": {
                "schema_version": 23,
                "appearance": {
                    "border_width": 6,
                    "border_offset": 2,
                    "effects": {"glow": {"enabled": true, "radius": 30, "opacity": 210, "inactive_opacity": 60}}
                }
            }
        });
        migrate(&mut document);
        let floating = &document["desktop"]["appearance"]["floating"];
        assert_eq!(floating["separate"], true);
        assert_eq!(floating["border_width"], 5, "one thinner than its own six");
        assert_eq!(floating["border_offset"], 2);
        assert_eq!(floating["glow_radius"], 20);
        assert_eq!(floating["glow_opacity"], 140);
        assert_eq!(floating["glow_inactive_opacity"], 40);
        assert!(
            floating["active_border_color"].is_null(),
            "the colours stay the tiled ones"
        );

        // What a file says of floating windows itself is left alone.
        let mut own = serde_json::json!({
            "desktop": {"schema_version": 23, "appearance": {"border_width": 6, "floating": {"border_width": 1}}}
        });
        migrate(&mut own);
        assert_eq!(
            own["desktop"]["appearance"]["floating"],
            serde_json::json!({"border_width": 1})
        );
    }

    #[test]
    fn the_zones_of_a_file_from_before_26_are_its_snap_zones() {
        let mut document = serde_json::json!({ "desktop": { "schema_version": 25, "zones": {
            "behavior": { "enabled": false, "snap_threshold": 0.2, "activation_modifier": "Alt" },
            "default_layout": { "id": "mine", "name": "Mine", "description": "", "zones": [
                { "id": 0, "name": "All", "x_percent": 0.0, "y_percent": 0.0,
                  "width_percent": 1.0, "height_percent": 1.0, "gap": 4, "priority": 0 } ] },
            "from_later": 1 } } });
        let before = document["desktop"]["zones"].clone();
        migrate(&mut document);
        let desktop = &document["desktop"];
        assert!(desktop.get("zones").is_none(), "the old place is empty");
        assert_eq!(desktop["snap_zones"], before, "nothing of it is changed");
        let config = Config::from_json(&document.to_string()).expect("loads");
        let snap_zones = &config.desktop.snap_zones;
        assert!(!snap_zones.behavior.enabled);
        assert_eq!(snap_zones.behavior.snap_threshold, 0.2);
        assert_eq!(
            snap_zones.behavior.activation_modifier,
            Some(crate::snap_zones::SnapZoneModifier::Alt)
        );
        assert_eq!(snap_zones.default_layout.id, "mine");
        assert_eq!(snap_zones.default_layout.zones[0].gap, 4);
        assert_eq!(
            snap_zones.extra.0["from_later"], 1,
            "a key that is not known is kept"
        );
        // And is written under its new name only: what is under `zones`
        // now is the zones of the desktop, as they are by default.
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(
            saved["desktop"]["zones"],
            serde_json::to_value(crate::zones::ZonesConfig::default()).unwrap()
        );
        assert!(!has_snap_zones_under_zones(&saved));
        assert_eq!(saved["desktop"]["snap_zones"]["default_layout"]["id"], "mine");

        // Snap zones already written in the new place win, and the old
        // place is empty all the same.
        let mut both = serde_json::json!({ "desktop": { "schema_version": 25,
            "zones": { "behavior": { "enabled": false } },
            "snap_zones": { "behavior": { "enabled": true } } } });
        migrate(&mut both);
        assert!(both["desktop"].get("zones").is_none());
        assert_eq!(both["desktop"]["snap_zones"]["behavior"]["enabled"], true);

        // A file without zones reads with the snap zones as they are by default.
        let mut bare = serde_json::json!({ "desktop": { "schema_version": 25 } });
        migrate(&mut bare);
        assert!(bare["desktop"].get("snap_zones").is_none());
    }

    #[test]
    fn snap_zones_under_the_old_name_in_a_file_of_26_are_refused() {
        // Written by hand in the old layout, under the new version: it is
        // not moved (the name is something else now) and not read wrongly.
        for zones in [
            serde_json::json!({ "behavior": { "enabled": false } }),
            serde_json::json!({ "default_layout": { "id": "mine" } }),
            serde_json::json!({ "monitor_layouts": [] }),
        ] {
            let document = serde_json::json!({ "desktop": { "schema_version": 26, "zones": zones } });
            let mut migrated = document.clone();
            migrate(&mut migrated);
            assert_eq!(migrated, document, "a current file is left as it is");
            let error = Config::from_json(&document.to_string()).expect_err("refused");
            assert!(error.to_string().contains("desktop.snap_zones"), "{error}");
        }
        // What is under `zones` and is not of the snap zones stops nothing.
        for zones in [
            serde_json::json!({}),
            serde_json::json!({ "enabled": false }),
        ] {
            let document = serde_json::json!({ "desktop": { "schema_version": 26, "zones": zones } });
            let config = Config::from_json(&document.to_string()).expect("loads");
            assert_eq!(
                serde_json::to_value(&config.desktop.snap_zones).unwrap(),
                serde_json::to_value(Config::new().desktop.snap_zones).unwrap()
            );
        }
    }

    #[test]
    fn the_zones_of_the_desktop_are_read_and_written_under_zones() {
        use crate::zones::{Matcher, Rule, RuleAction, ZoneDefinition, ZoneId, ZoneKind, ZonesConfig};
        // Nothing the zones of the desktop are written with is a key that
        // only snap zones have: a file gevrek wrote is never refused.
        let mut filled = ZonesConfig {
            enabled: true,
            zones: vec![
                ZoneDefinition::manual("papers", "Papers"),
                ZoneDefinition {
                    kind: ZoneKind::Portal { directory: "/home/ada/Downloads".to_string() },
                    tab_group: Some("side".to_string()),
                    ..ZoneDefinition::manual("downloads", "Downloads")
                },
                ZoneDefinition {
                    kind: ZoneKind::Smart { query: vec![Matcher::Untouched { days: 30 }] },
                    ..ZoneDefinition::manual("stale", "Stale")
                },
            ],
            rules: vec![Rule {
                name: "papers".to_string(),
                matchers: vec![
                    Matcher::Extension { any_of: vec!["pdf".to_string()] },
                    Matcher::Size { at_least: Some(1), at_most: None },
                ],
                action: RuleAction::AssignToZone { zone: ZoneId::new("papers") },
                ..Rule::default()
            }],
            catch_all: Some(RuleAction::MoveToFolder { folder: "/home/ada/Sorted".to_string() }),
            ..ZonesConfig::default()
        };
        filled.look.opacity = Some(70);
        filled.snap.grid = 16;
        for zones in [ZonesConfig::default(), filled] {
            let mut config = Config::new();
            config.desktop.zones = zones.clone();
            let text = serde_json::to_string_pretty(&config).unwrap();
            let written: Value = serde_json::from_str(&text).unwrap();
            for key in SNAP_ZONE_KEYS {
                assert!(written["desktop"]["zones"].get(key).is_none(), "{key}");
            }
            assert!(!has_snap_zones_under_zones(&written));
            let read = Config::from_json(&text).expect("what gevrek wrote is read");
            assert_eq!(read.desktop.zones, zones);
            assert_eq!(serde_json::to_string_pretty(&read).unwrap(), text, "and written as it was");
            let errors: Vec<_> = read
                .desktop
                .validate()
                .into_iter()
                .filter(|issue| issue.path.starts_with("desktop.zones"))
                .collect();
            assert!(errors.is_empty(), "{errors:?}");
        }
        // On, unless a file says otherwise; and a file that says they are
        // off keeps them off, whatever else it leaves unsaid.
        assert!(Config::new().desktop.zones.enabled);
        let unsaid = serde_json::json!({ "desktop": { "schema_version": 26 } });
        assert!(Config::from_json(&unsaid.to_string()).expect("read").desktop.zones.enabled);
        // (Also a file from before there were zones: it never said.)
        let old = serde_json::json!({ "desktop": { "schema_version": 25 } });
        assert!(Config::from_json(&old.to_string()).expect("read").desktop.zones.enabled);
        let off = serde_json::json!({ "desktop": { "schema_version": 26, "zones": { "enabled": false } } });
        let read = Config::from_json(&off.to_string()).expect("read");
        assert!(!read.desktop.zones.enabled);
        let written: Value = serde_json::from_str(&serde_json::to_string(&read).unwrap()).unwrap();
        assert_eq!(written["desktop"]["zones"]["enabled"], false, "and it is written back as off");
        // The old shape under the new version is refused all the same, also
        // beside keys that are of the new zones.
        let mixed = serde_json::json!({ "desktop": { "schema_version": 26, "zones": {
            "enabled": true, "zones": [], "behavior": { "enabled": true } } } });
        let error = Config::from_json(&mixed.to_string()).expect_err("refused");
        assert!(error.to_string().contains("desktop.snap_zones"), "{error}");
        // And a file from before 26 that has both: its `zones` were snap
        // zones and become that; the new section starts as it is by default.
        let old = serde_json::json!({ "desktop": { "schema_version": 25, "zones": {
            "behavior": { "enabled": false } } } });
        let config = Config::from_json(&old.to_string()).expect("migrated");
        assert!(!config.desktop.snap_zones.behavior.enabled);
        assert_eq!(config.desktop.zones, ZonesConfig::default());
    }

    #[test]
    fn the_floating_look_is_a_little_less_than_the_tiled_one() {
        use crate::settings::{AppearanceProfile, FloatingAppearance, GlowEffect};
        let tiled = AppearanceProfile::default();
        let floating = &tiled.floating;
        assert!(floating.separate && floating.border_enabled);
        assert_eq!(floating.border_width, tiled.border_width - 1);
        assert!(floating.glow_radius < tiled.effects.glow.radius);
        assert!(floating.glow_opacity < tiled.effects.glow.opacity);
        // A border of one pixel stays: thinner would be none.
        assert_eq!(
            FloatingAppearance::softer_than(1, 0, &GlowEffect::default()).border_width,
            1
        );
    }
}
