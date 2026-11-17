//! What comes from outside does not end the program: a configuration or
//! a saved session that was cut short, or that has nonsense where a
//! value belongs, is refused or lived with, never a panic.
//!
//! Each of gevrek's own example configurations, and a session saved from
//! a desktop, is spoilt some thousand ways (always the same ways: the
//! numbers come from a seed) and taken as far as a real one is taken:
//! read, checked, tidied, written back, made into rules and zones, laid
//! over a desktop with windows on it, and the windows laid out.
//!
//! `GEVREK_SPOIL=<n>` spoils n times as often, in other ways than
//! without it: for a long look now and then.

use crate::core::WmCore;
use crate::events::WindowEvent;
use crate::executor::Executor;
use crate::focus_manager::FocusManager;
use crate::layouts::LayoutType;
use crate::outputs::{MonitorHandle, Outputs};
use crate::rules::RuleEngine;
use crate::testing::fake_desktop::FakeDesktop;
use crate::zones::ZoneManager;
use gevrek_model::config::Config;
use gevrek_model::graph::NodeGraph;
use serde_json::Value;
use std::sync::{Arc, Mutex};

const OUTPUT: MonitorHandle = MonitorHandle(1);

/// Numbers that look like chance and are the same in every run.
struct Chance(u64);

impl Chance {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, count: usize) -> usize {
        (self.next() % count.max(1) as u64) as usize
    }
}

/// How many times as often as usual, and with which seed.
fn harder() -> (usize, u64) {
    let times = std::env::var("GEVREK_SPOIL")
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    (times, (times as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// What is put where a value belongs.
fn nonsense(chance: &mut Chance) -> Value {
    match chance.below(22) {
        0 => Value::Null,
        1 => Value::Bool(true),
        2 => Value::Bool(false),
        3 => serde_json::json!(0),
        4 => serde_json::json!(-1),
        5 => serde_json::json!(1),
        6 => serde_json::json!(i64::MIN),
        7 => serde_json::json!(i64::MAX),
        8 => serde_json::json!(u64::MAX),
        9 => serde_json::json!(i32::MAX),
        10 => serde_json::json!(i32::MIN),
        11 => serde_json::json!(1e308),
        12 => serde_json::json!(-1e308),
        13 => serde_json::json!(0.000_000_1),
        14 => serde_json::json!(""),
        15 => serde_json::json!("\u{0}\u{feff}\n"),
        16 => serde_json::json!("x".repeat(5000)),
        17 => serde_json::json!("(unclosed[regex"),
        18 => serde_json::json!([]),
        19 => serde_json::json!({}),
        20 => serde_json::json!([[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]),
        _ => serde_json::json!(65536),
    }
}

/// Every place in a document, as the keys and indices that lead there.
fn places(value: &Value, here: &mut Vec<String>, all: &mut Vec<Vec<String>>) {
    all.push(here.clone());
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                here.push(key.clone());
                places(inner, here, all);
                here.pop();
            }
        }
        Value::Array(list) => {
            for (index, inner) in list.iter().enumerate() {
                here.push(index.to_string());
                places(inner, here, all);
                here.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(value: &'a mut Value, place: &[String]) -> Option<&'a mut Value> {
    let mut now = value;
    for step in place {
        now = match now {
            Value::Object(map) => map.get_mut(step)?,
            Value::Array(list) => list.get_mut(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(now)
}

/// One document spoilt: a value replaced by nonsense, a number made
/// another, a list emptied, doubled or made very long, a key taken out.
/// Returns what was done, to be told where it ends the program.
fn spoil(document: &mut Value, all: &[Vec<String>], chance: &mut Chance) -> String {
    let place = &all[chance.below(all.len())];
    let Some(value) = at(document, place) else {
        return "nothing".to_string();
    };
    let how = chance.below(8);
    let what = match (how, &mut *value) {
        (0..=3, value) => {
            *value = nonsense(chance);
            format!("= {}", short(value))
        }
        (4, Value::Number(number)) => {
            let by = [-1.0, 0.0, 1000.0, -1000.0, 0.5, 1e9][chance.below(6)];
            let now = number.as_f64().unwrap_or(0.0) * by;
            *value = serde_json::Number::from_f64(now.trunc())
                .map_or(Value::Null, Value::Number);
            format!("times {by}")
        }
        (5, Value::Array(list)) => {
            list.clear();
            "emptied".to_string()
        }
        (6, Value::Array(list)) if !list.is_empty() => {
            let twice = list.clone();
            for _ in 0..chance.below(40) {
                list.extend(twice.iter().cloned());
            }
            format!("made {} long", list.len())
        }
        (7, Value::Object(map)) if !map.is_empty() => {
            let key = map.keys().nth(chance.below(map.len())).cloned().unwrap_or_default();
            map.remove(&key);
            format!("without {key}")
        }
        (_, value) => {
            *value = nonsense(chance);
            format!("= {}", short(value))
        }
    };
    format!("/{} {what}", place.join("/"))
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    text.chars().take(40).collect()
}

fn desk() -> (WmCore, FakeDesktop) {
    let rules = Arc::new(Mutex::new(RuleEngine::new()));
    let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
    let mut outputs = Outputs::with_workspace_count(0);
    outputs.add_test_monitor(OUTPUT, "Primary");
    outputs
        .create_workspace(OUTPUT, "main".to_string(), LayoutType::BSP)
        .expect("a workspace");
    let core = WmCore::new(
        executor,
        outputs,
        FocusManager::new(),
        ZoneManager::new(Default::default()),
    );
    (core, FakeDesktop::new(OUTPUT))
}

fn open(core: &mut WmCore, desktop: &mut FakeDesktop, count: usize) {
    for index in 0..count {
        let id = desktop.open(&format!("window {index}"), "app.exe");
        let _ = core.handle_window_event(desktop, WindowEvent::Created(id));
        let _ = core.flush_pending_layouts(desktop);
    }
}

/// A text that may be a configuration, taken as far as a real one is.
fn taken_in(text: &str) {
    let Ok(mut config) = Config::from_json(text) else {
        return;
    };
    let _ = config.validate();
    if config.normalize().is_err() {
        return;
    }
    // It is written and read back as any that is saved.
    let written = serde_json::to_string(&config).expect("a configuration is written");
    let _ = Config::from_json(&written);
    let _ = crate::reload::ConfigDiff::between(&Config::new(), &config).steps();
    let _ = config.desktop.clone().set_main_key(gevrek_model::settings::MainKey::Alt);

    // It is laid over a desktop with windows on it.
    let (mut core, mut desktop) = desk();
    open(&mut core, &mut desktop, 2);
    let rules = crate::config_apply::build_rule_engine(&config.desktop);
    let _ = rules.rule_states();
    crate::config_apply::configure_monitor_profiles(&mut core.monitor_manager, &config.desktop);
    let _ = crate::config_apply::build_zone_manager(
        &config.desktop.zones,
        &core.monitor_manager,
        |_| false,
    );
    let _ = core.apply_workspace_profile_set(
        &config.desktop.workspaces,
        &config.desktop.appearance,
        |_| true,
        true,
    );
    let _ = core.flush_pending_layouts(&mut desktop);
    // And windows come to it and are laid out by it.
    open(&mut core, &mut desktop, 3);
    let _ = core.monitor_data(&desktop, &config.desktop);
}

/// Runs `run` and tells whether it came to its end. (What a panic would
/// print is not printed: thousands of them are expected to be none.)
fn lived(run: impl FnOnce() + std::panic::UnwindSafe) -> Result<(), String> {
    std::panic::catch_unwind(run).map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "a panic".to_string())
    })
}

#[test]
fn no_configuration_from_outside_ends_the_program() {
    let (times, seed) = harder();
    let mut chance = Chance(0x9e37_79b9_7f4a_7c15 ^ seed.rotate_left(17) | 1);
    let mut ended: Vec<String> = Vec::new();
    for example in gevrek_model::examples::built_in_examples() {
        let whole = serde_json::to_value(&example.config).expect("an example");
        let text = serde_json::to_string_pretty(&whole).expect("written");

        // Cut short, at a hundred places.
        for cut in 0..100 {
            let mut end = text.len() * cut / 100;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let part = text[..end].to_string();
            if let Err(panic) = lived(move || taken_in(&part)) {
                ended.push(format!("{} cut at {end}: {panic}", example.id));
            }
        }

        // Nonsense where a value belongs: one place, and three at once.
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);
        for round in 0..150 * times {
            let mut document = whole.clone();
            let mut done = Vec::new();
            for _ in 0..if round % 4 == 0 { 3 } else { 1 } {
                done.push(spoil(&mut document, &all, &mut chance));
            }
            let spoilt = document.to_string();
            if let Err(panic) = lived(move || taken_in(&spoilt)) {
                ended.push(format!("{} with {}: {panic}", example.id, done.join(" and ")));
            }
        }
    }
    ended.sort();
    ended.dedup();
    assert!(
        ended.is_empty(),
        "{} spoilt configurations ended the program:\n{}",
        ended.len(),
        ended.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn no_saved_session_from_outside_ends_the_program() {
    // A session as it is saved from a desktop with windows on two
    // workspaces.
    let (mut core, mut desktop) = desk();
    open(&mut core, &mut desktop, 4);
    let saved = core.session_state(&desktop);
    let whole = serde_json::to_value(&saved).expect("a session");
    let text = serde_json::to_string_pretty(&whole).expect("written");
    let mut all = Vec::new();
    places(&whole, &mut Vec::new(), &mut all);

    let restored = |text: &str| {
        let Ok(state) =
            serde_json::from_str::<crate::persistence::WindowManagerState>(text)
        else {
            return;
        };
        let (mut core, mut desktop) = desk();
        open(&mut core, &mut desktop, 4);
        let _ = core.restore_session(&mut desktop, &state);
        let _ = core.flush_pending_layouts(&mut desktop);
    };

    let (times, seed) = harder();
    let mut chance = Chance(0x2545_f491_4f6c_dd1d ^ seed.rotate_left(29) | 1);
    let mut ended: Vec<String> = Vec::new();
    for cut in 0..200 {
        let mut end = text.len() * cut / 200;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let part = text[..end].to_string();
        if let Err(panic) = lived(move || restored(&part)) {
            ended.push(format!("cut at {end}: {panic}"));
        }
    }
    for round in 0..500 * times {
        let mut document = whole.clone();
        let mut done = Vec::new();
        for _ in 0..if round % 4 == 0 { 3 } else { 1 } {
            done.push(spoil(&mut document, &all, &mut chance));
        }
        let spoilt = document.to_string();
        if let Err(panic) = lived(move || restored(&spoilt)) {
            ended.push(format!("with {}: {panic}", done.join(" and ")));
        }
    }
    ended.sort();
    ended.dedup();
    assert!(
        ended.is_empty(),
        "{} spoilt sessions ended the program:\n{}",
        ended.len(),
        ended.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}
