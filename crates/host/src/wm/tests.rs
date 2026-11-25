//! The shared host on the engine's fake desktop: what every backend's
//! host does, tried without a windowing system (and so on every system,
//! where the backends' own tests run only on theirs).

use super::bindings::{binding_id, Bindings};
use super::commands::Commands;
use super::configuration::Configuration;
use super::published::{Published, SystemState};
use super::reload::Reload;
use super::{HostState, Parts, WmHost};
use crate::config_store::{config_channel, ConfigRequest, FIRST_REVISION, REFUSED};
use gevrek_engine::core::WmCore;
use gevrek_engine::executor::{Executor, NodeExecutionEvent, RuntimeEventKind};
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::input::{Hotkey, HotkeyEvent};
use gevrek_engine::outputs::{MonitorHandle, Outputs};
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::snap_zones::SnapZoneManager;
use gevrek_engine::testing::FakeDesktop;
use gevrek_model::command::{DispatchCommand, ShellAction};
use gevrek_model::config::Config;
use gevrek_model::settings::{BindingDevice, BindingProfile, BindingTrigger};
use std::sync::{Arc, Mutex};

const OUTPUT: MonitorHandle = MonitorHandle(1);

/// A backend of the plainest kind: the real core on a fake desktop.
struct TestHost {
    config: Config,
    core: WmCore,
    desktop: FakeDesktop,
    host: HostState,
    rules: Arc<Mutex<RuleEngine>>,
    /// What it was last given to take.
    installed: Vec<(i32, Hotkey)>,
    settled: usize,
    /// Everything the event stream was told.
    events: Arc<Mutex<Vec<RuntimeEventKind>>>,
    notices: Vec<gevrek_engine::overlay_data::Notice>,
}

impl TestHost {
    fn new(config: Config) -> Self {
        let rules = Arc::new(Mutex::new(RuleEngine::new()));
        let executor = Executor::new(
            config.node_graph.clone(),
            Arc::new(Mutex::new(Vec::new())),
            rules.clone(),
        );
        let mut outputs = Outputs::with_workspace_count(1);
        outputs.add_test_monitor(OUTPUT, "Primary");
        let core = WmCore::new(
            executor,
            outputs,
            FocusManager::new(),
            SnapZoneManager::new(Default::default()),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let told = events.clone();
        let host = HostState {
            events: Some(Arc::new(move |event| {
                if let NodeExecutionEvent::Runtime(event) = event {
                    told.lock().unwrap().push(event.kind);
                }
            })),
            overlay: Some(Arc::new(Mutex::new(None))),
            ..HostState::default()
        };
        Self {
            config,
            core,
            desktop: FakeDesktop::new(OUTPUT),
            host,
            rules,
            installed: Vec::new(),
            settled: 0,
            events,
            notices: Vec::new(),
        }
    }

    fn told(&self, kind: RuntimeEventKind) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|told| **told == kind)
            .count()
    }

    fn published(&self) -> gevrek_engine::overlay_data::OverlayData {
        self.host
            .overlay
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .clone()
            .map(|data| (*data).clone())
            .expect("published")
    }
}

impl WmHost for TestHost {
    type Desktop = FakeDesktop;

    fn parts(&mut self) -> Parts<'_, FakeDesktop> {
        Parts {
            config: &mut self.config,
            core: &mut self.core,
            desktop: &mut self.desktop,
            host: &mut self.host,
        }
    }
    fn host(&self) -> &HostState {
        &self.host
    }
    fn config(&self) -> &Config {
        &self.config
    }
    fn settle(&mut self) {
        self.settled += 1;
        self.core.flush_pending_layouts(&mut self.desktop).unwrap();
    }
    fn install_bindings(&mut self, bindings: Vec<(i32, Hotkey)>) -> anyhow::Result<()> {
        self.installed = bindings;
        Ok(())
    }
    fn apply_appearance(&mut self) {
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
    }
    fn set_animations(&mut self, _animations: &gevrek_model::settings::AnimationProfile) {}
    fn configure_monitors(&mut self) {
        gevrek_engine::config_apply::configure_monitor_profiles(
            &mut self.core.monitor_manager,
            &self.config.desktop,
        );
    }
    fn rules(&self) -> &Arc<Mutex<RuleEngine>> {
        &self.rules
    }
    fn system_state(&self) -> SystemState {
        SystemState {
            notices: self.notices.clone(),
            ..SystemState::default()
        }
    }
}

fn binding(key: &str, submap: &str, command: DispatchCommand) -> BindingProfile {
    BindingProfile {
        key: key.to_string(),
        submap: submap.to_string(),
        enabled: true,
        command,
        ..BindingProfile::default()
    }
}

/// A configuration with a binding in the submap `resize` (and one to get
/// there), on keys the standard bindings leave alone.
fn with_submap() -> Config {
    let mut config = Config::new();
    config.desktop.bindings = vec![
        binding(
            "F13",
            "default",
            DispatchCommand::SetBindingSubmap {
                name: "resize".to_string(),
            },
        ),
        binding("F14", "resize", DispatchCommand::ResetBindingSubmap),
    ];
    config
}

fn installed_keys(host: &TestHost) -> Vec<String> {
    host.installed
        .iter()
        .map(|(_, hotkey)| hotkey.key.clone())
        .collect()
}

#[test]
fn a_submap_is_taken_with_its_keys_and_told_at_once() {
    let mut host = TestHost::new(with_submap());
    host.refresh_bindings().unwrap();
    assert!(installed_keys(&host).contains(&"F13".to_string()));
    assert!(!installed_keys(&host).contains(&"F14".to_string()));

    host.run_command(DispatchCommand::SetBindingSubmap {
        name: "Resize ".to_string(),
    })
    .unwrap();
    assert_eq!(host.host.active_submap, "resize");
    assert!(installed_keys(&host).contains(&"F14".to_string()));
    assert!(!installed_keys(&host).contains(&"F13".to_string()));
    // Published before the command is answered: the panel shows the mode.
    assert_eq!(host.published().active_binding_submap, "resize");
    assert_eq!(host.told(RuntimeEventKind::DispatchSucceeded), 1);

    host.run_command(DispatchCommand::ResetBindingSubmap)
        .unwrap();
    assert_eq!(host.host.active_submap, "default");
}

#[test]
fn a_submap_the_configuration_has_not_is_refused_and_told() {
    let mut host = TestHost::new(with_submap());
    let error = host
        .run_command(DispatchCommand::SetBindingSubmap {
            name: "nowhere".to_string(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("nowhere"), "{error}");
    assert_eq!(host.host.active_submap, "default");
    assert_eq!(host.told(RuntimeEventKind::DispatchFailed), 1);
}

#[test]
fn a_key_runs_its_binding() {
    let mut host = TestHost::new(with_submap());
    host.refresh_bindings().unwrap();
    let (id, hotkey) = host
        .installed
        .iter()
        .find(|(_, hotkey)| hotkey.key == "F13")
        .cloned()
        .unwrap();
    let index = host
        .config
        .live_bindings()
        .iter()
        .position(|binding| binding.key == "F13")
        .unwrap();
    assert_eq!(id, binding_id(index));
    host.bindings_triggered(&HotkeyEvent {
        registration_ids: vec![id],
        key: hotkey.key,
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
        device: BindingDevice::Keyboard,
        trigger: BindingTrigger::Press,
        held_ms: 0,
    });
    assert_eq!(host.host.active_submap, "resize");
    assert_eq!(host.told(RuntimeEventKind::BindingTriggered), 1);
}

#[test]
fn a_submap_a_reload_takes_out_falls_back_to_default() {
    let mut host = TestHost::new(with_submap());
    host.run_command(DispatchCommand::SetBindingSubmap {
        name: "resize".to_string(),
    })
    .unwrap();
    let mut without = with_submap();
    without.desktop.bindings.pop();
    host.reload_configuration(without);
    assert_eq!(host.host.active_submap, "default");
    assert!(installed_keys(&host).contains(&"F13".to_string()));
}

#[test]
fn a_toggle_is_kept_in_the_configuration_and_published() {
    let mut host = TestHost::new(Config::new());
    let (publisher, watch) = config_channel(host.config.clone());
    host.host.publish_to(publisher, &host.config);
    let before = watch.borrow().revision;
    let transparency = host.config.desktop.appearance.transparency_enabled;

    host.run_command(DispatchCommand::ToggleTransparency)
        .unwrap();
    assert!(host.host.config_dirty);
    host.publish_config_if_changed();
    let snapshot = watch.borrow().clone();
    assert_eq!(snapshot.revision, before + 1);
    assert_eq!(
        snapshot.config.desktop.appearance.transparency_enabled,
        !transparency
    );
    // A reload of what runs (as the file watcher would) does not undo it.
    let running = host.config.clone();
    host.reload_configuration(running);
    assert_eq!(
        host.config.desktop.appearance.transparency_enabled,
        !transparency
    );
}

#[test]
fn an_apply_based_on_a_revision_that_no_longer_runs_is_refused() {
    let mut host = TestHost::new(Config::new());
    let (sender, receiver) = std::sync::mpsc::channel();
    host.host.config_receiver = Some(receiver);
    let (publisher, watch) = config_channel(host.config.clone());
    host.host.publish_to(publisher, &host.config);
    let published = watch.borrow().revision;
    assert!(published >= FIRST_REVISION);

    // A key toggles the stackbar: changed here, not yet published.
    let stackbar = !host.config.desktop.appearance.stackbar.enabled;
    host.config.desktop.appearance.stackbar.enabled = stackbar;
    host.host.config_dirty = true;

    // The editor's configuration was made from `published`.
    let mut edited = Config::new();
    edited.desktop.focus.history_size += 5;
    let wanted = edited.desktop.focus.history_size;
    sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited.clone()),
            base_revision: published,
        })
        .unwrap();
    host.apply_configuration_changes();
    let snapshot = watch.borrow().clone();
    assert_eq!(
        snapshot.revision,
        published + 1,
        "the toggle got its revision"
    );
    assert_eq!(
        (snapshot.requests_applied, snapshot.last_request_revision),
        (1, REFUSED)
    );
    assert_eq!(host.config.desktop.appearance.stackbar.enabled, stackbar);
    assert_ne!(host.config.desktop.focus.history_size, wanted);

    // Made again from the revision that runs now, it is applied.
    edited.desktop.appearance.stackbar.enabled = stackbar;
    sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited),
            base_revision: published + 1,
        })
        .unwrap();
    host.apply_configuration_changes();
    let snapshot = watch.borrow().clone();
    assert_eq!(
        (snapshot.requests_applied, snapshot.last_request_revision),
        (2, published + 2)
    );
    assert_eq!(host.config.desktop.focus.history_size, wanted);
    assert_eq!(host.told(RuntimeEventKind::ConfigurationReloaded), 1);
}

#[test]
fn reloading_the_running_configuration_changes_nothing_and_says_so() {
    let mut host = TestHost::new(with_submap());
    let running = host.config.clone();
    host.reload_configuration(running);
    assert_eq!(host.told(RuntimeEventKind::ConfigurationReloaded), 1);
    // Nothing was applied, nor laid out.
    assert_eq!(host.settled, 0);
    assert!(host.installed.is_empty());
}

#[test]
fn quit_stops_the_loop_through_the_server() {
    let mut host = TestHost::new(Config::new());
    let stop = Arc::new(Mutex::new(false));
    host.host.stop_flag = Some(stop.clone());
    host.run_command(DispatchCommand::Quit).unwrap();
    assert!(*stop.lock().unwrap());
    assert!(host.host.stop_requested());
}

#[test]
fn what_a_system_cannot_do_it_says() {
    let mut host = TestHost::new(Config::new());
    for command in [
        DispatchCommand::PickWindow,
        DispatchCommand::WorkspaceOverview,
        DispatchCommand::ShortcutsInhibit,
        DispatchCommand::NightLightPreview {
            temperature: 3000,
            seconds: 4,
        },
    ] {
        let error = host.run_command(command.clone()).unwrap_err();
        assert!(
            error.to_string().contains("not possible on this system"),
            "{command:?}: {error}"
        );
    }
    assert!(host
        .run_command(DispatchCommand::OutputTrial {
            answer: gevrek_model::command::TrialAnswer::Keep,
        })
        .is_err());
}

#[test]
fn the_night_light_key_switches_it_and_says_what_that_does() {
    let mut host = TestHost::new(Config::new());
    let on = host.config.desktop.night_light.enabled;
    host.run_command(DispatchCommand::Shell {
        action: ShellAction::NightLight,
    })
    .unwrap();
    assert_eq!(host.config.desktop.night_light.enabled, !on);
    assert!(host.host.config_dirty);
    // Told to the shell twice: the action, and what to show on the screen.
    assert_eq!(host.told(RuntimeEventKind::ShellCommand), 2);
}

#[test]
fn other_notices_are_told_once_as_they_are_published() {
    let mut host = TestHost::new(Config::new());
    host.publish_state(None);
    assert_eq!(host.told(RuntimeEventKind::NoticesChanged), 0);
    host.notices = vec![gevrek_engine::overlay_data::Notice {
        id: "display".to_string(),
        title: "A display".to_string(),
        text: "is not as asked".to_string(),
    }];
    host.publish_state(None);
    host.publish_state(None);
    assert_eq!(host.told(RuntimeEventKind::NoticesChanged), 1);
    assert_eq!(host.published().notices.len(), 1);
    assert!(!host.host.state_stale.get());
}

#[test]
fn every_command_settles_the_desktop() {
    let mut host = TestHost::new(Config::new());
    let before = host.settled;
    host.run_command(DispatchCommand::ToggleStackbar).unwrap();
    let _ = host.run_command(DispatchCommand::PickWindow);
    assert_eq!(host.settled, before + 2);
}

#[test]
fn a_notice_whose_text_changes_is_told_again() {
    let mut host = TestHost::new(Config::new());
    let notice = |text: &str| gevrek_engine::overlay_data::Notice {
        id: "display".to_string(),
        title: "A display".to_string(),
        text: text.to_string(),
    };
    host.notices = vec![notice("is not as asked")];
    host.publish_state(None);
    host.notices = vec![notice("was refused: its mode is not there")];
    host.publish_state(None);
    assert_eq!(host.told(RuntimeEventKind::NoticesChanged), 2);
}

#[test]
fn a_batch_of_commands_settles_once_after_the_last() {
    let mut host = TestHost::new(Config::new());
    let (sender, receiver) = std::sync::mpsc::channel();
    host.host.command_receiver = Some(receiver);
    let (response, answer) = tokio::sync::oneshot::channel();
    sender
        .send(crate::command::ManualCommand {
            command: crate::command::RuntimeCommand::DispatchBatch {
                commands: vec![
                    DispatchCommand::ToggleStackbar,
                    DispatchCommand::ToggleStackbar,
                ],
                stop_on_error: false,
            },
            response,
        })
        .unwrap();
    let before = host.settled;
    assert!(host.run_queued_commands());
    assert_eq!(host.settled, before + 1);
    assert_eq!(host.told(RuntimeEventKind::DispatchSucceeded), 2);
    drop(answer);
}
