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
use crate::spoil::{harder, lived, places, spoil, Chance};
use crate::testing::fake_desktop::FakeDesktop;
use crate::zones::ZoneManager;
use gevrek_model::config::Config;
use gevrek_model::graph::NodeGraph;
use std::sync::{Arc, Mutex};

const OUTPUT: MonitorHandle = MonitorHandle(1);

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

/// The names of the commands, as they are written in JSON: read from
/// where they are declared, so that one that is added is here too.
fn command_names() -> Vec<String> {
    let source = include_str!("../../model/src/command.rs");
    let from = source.find("pub enum DispatchCommand").expect("the commands");
    let declared = &source[from..];
    let declared = &declared[..declared.find("\n}").expect("their end")];
    declared
        .lines()
        .filter_map(|line| {
            let name = line.strip_prefix("    ")?;
            let name: String = name
                .chars()
                .take_while(|letter| letter.is_ascii_alphanumeric())
                .collect();
            name.starts_with(|letter: char| letter.is_ascii_uppercase())
                .then_some(name)
        })
        .map(|name| {
            let mut snake = String::new();
            for letter in name.chars() {
                if letter.is_ascii_uppercase() && !snake.is_empty() {
                    snake.push('_');
                }
                snake.push(letter.to_ascii_lowercase());
            }
            snake
        })
        .collect()
}

/// One of each command, with something in every field it must have:
/// found by asking the reader what is missing or of the wrong kind,
/// until it reads. (A command whose fields are not found so is left to
/// the commands of the examples.)
fn every_command() -> Vec<serde_json::Value> {
    use gevrek_model::command::DispatchCommand;
    let between = |text: &str, from: &str| -> Option<String> {
        let rest = &text[text.find(from)? + from.len()..];
        Some(rest[..rest.find('`')?].to_string())
    };
    let kinds = [
        serde_json::json!(1),
        serde_json::json!("x"),
        serde_json::json!(true),
        serde_json::json!(0.5),
        serde_json::json!([]),
        serde_json::json!({}),
    ];
    // Where in the document a field is: at the top, or in a thing that
    // is itself a field (`selector`, then `kind`).
    fn put(document: &mut serde_json::Value, place: &[String], value: serde_json::Value) {
        match place {
            [field] => document[field.as_str()] = value,
            [outer, field] => document[outer.as_str()][field.as_str()] = value,
            _ => {}
        }
    }
    let mut all = Vec::new();
    for name in command_names() {
        let mut document = serde_json::json!({ "command": name });
        // The field that was given last, and which kind it was given.
        let mut last: Option<(Vec<String>, usize)> = None;
        for _ in 0..80 {
            // (The reading that says why: the one every reader goes
            // through keeps what it cannot read as a command unknown.)
            let why = match DispatchCommand::deserialize(document.clone()) {
                Ok(DispatchCommand::Unknown(_)) => break,
                Ok(_) => {
                    all.push(document);
                    break;
                }
                Err(error) => error.to_string(),
            };
            if let Some(field) = between(&why, "missing field `") {
                // In the thing the last field is or is in, unless it has
                // that field already: then it is the command's own.
                let inside = last.as_ref().and_then(|(place, _)| {
                    let outer = place.first()?;
                    let thing = document[outer.as_str()].as_object()?;
                    (!thing.contains_key(&field)).then(|| vec![outer.clone(), field.clone()])
                });
                let place = inside.unwrap_or_else(|| vec![field]);
                put(&mut document, &place, kinds[0].clone());
                last = Some((place, 0));
            } else if let (Some(word), Some((place, _))) = (
                between(&why, "expected one of `").or_else(|| between(&why, "expected `")),
                &last,
            ) {
                // A word of a few: the first of them.
                put(&mut document, place, word.into());
            } else if let Some((place, kind)) = last.clone() {
                // Of the wrong kind: the next kind.
                let Some(next) = kinds.get(kind + 1) else {
                    break;
                };
                put(&mut document, &place, next.clone());
                last = Some((place, kind + 1));
            } else {
                break;
            }
        }
    }
    all
}

/// A command comes as JSON from whoever may speak to the window manager
/// (`gevrek dispatch`, the panel, a script): spoilt, it is refused or
/// carried out as far as it can be, with windows on the desktop.
#[test]
fn no_command_from_outside_ends_the_program() {
    use gevrek_model::command::DispatchCommand;
    let given = |text: &str| {
        let Ok(command) = serde_json::from_str::<DispatchCommand>(text) else {
            return;
        };
        // (What reaches outside the engine is not for this, whatever
        // it was spoilt into.)
        let name = serde_json::to_value(&command)
            .ok()
            .and_then(|value| value["command"].as_str().map(str::to_string));
        if matches!(command, DispatchCommand::Unknown(_))
            || name.is_none_or(|name| crate::random_runs::LEFT_OUT.contains(&name.as_str()))
        {
            return;
        }
        let (mut core, mut desktop) = desk();
        open(&mut core, &mut desktop, 3);
        let _ = core.dispatch(&mut desktop, command.clone());
        let _ = core.flush_pending_layouts(&mut desktop);
        // And once more, on what the first left.
        let _ = core.dispatch(&mut desktop, command);
        let _ = core.flush_pending_layouts(&mut desktop);
        let config = gevrek_model::examples::starting_configuration().desktop;
        let _ = core.monitor_data(&desktop, &config);
    };

    let (times, seed) = harder();
    let mut chance = Chance(0x6a09_e667_f3bc_c909 ^ seed.rotate_left(11) | 1);
    let mut ended: Vec<String> = Vec::new();
    let mut commands = crate::random_runs::commands();
    commands.extend(every_command());
    // (All but a few are found so; were it far fewer, the finding
    // would have stopped working.)
    assert!(
        every_command().len() * 10 >= command_names().len() * 9,
        "{} of {} commands have a sample",
        every_command().len(),
        command_names().len()
    );
    for whole in commands {
        let text = whole.to_string();
        for end in 0..text.len() {
            if !text.is_char_boundary(end) {
                continue;
            }
            let part = text[..end].to_string();
            if let Err(panic) = lived(move || given(&part)) {
                ended.push(format!("{text} cut at {end}: {panic}"));
            }
        }
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);
        for _ in 0..40 * times {
            let mut document = whole.clone();
            let done = spoil(&mut document, &all, &mut chance);
            let spoilt = document.to_string();
            if let Err(panic) = lived(move || given(&spoilt)) {
                ended.push(format!("{text} with {done}: {panic}"));
            }
        }
    }
    ended.sort();
    ended.dedup();
    assert!(
        ended.is_empty(),
        "{} spoilt commands ended the program:\n{}",
        ended.len(),
        ended.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}
