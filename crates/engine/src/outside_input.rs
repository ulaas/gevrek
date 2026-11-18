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
