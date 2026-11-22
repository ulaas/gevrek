//! Key bindings: which are live, by which id, and what a key that hit
//! them does.
//!
//! A system takes the keys its own way (Windows through a keyboard hook,
//! a compositor from its input devices, macOS through an event tap); what
//! it is given to take is the same everywhere: a list of hotkeys, each with
//! an id that says what it is for.
//!
//! - an `OnKeyPress` node of the graph: below zero (`-1 - index` of the
//!   node in the graph);
//! - a binding of the configuration: from [`BINDING_IDS`] on
//!   (`BINDING_IDS + index` in `Config::live_bindings`);
//! - a touchpad gesture profile: from [`GESTURE_IDS`] on.
//!
//! A key that hits bindings comes back as their ids, in the event's
//! `registration_ids` ([`Bindings::bindings_triggered`]).

use super::WmHost;
use crate::wm::commands::Commands;
use gevrek_engine::executor::{PendingFocusOp, RuntimeEventKind};
use gevrek_engine::input::{Hotkey, HotkeyEvent};
use gevrek_engine::Effect;
use gevrek_model::config::Config;
use gevrek_model::graph::{NodeGraph, NodeType};

/// Where the ids of the configuration's bindings begin.
pub const BINDING_IDS: i32 = 1_000_000;
/// Where the ids of the gesture profiles begin.
pub const GESTURE_IDS: i32 = 2_000_000;

/// The id of the graph's key node at `index` among the graph's nodes.
pub fn graph_key_id(index: usize) -> i32 {
    (-1i32).saturating_sub(index as i32)
}

/// The id of the binding at `index` in `Config::live_bindings`.
pub fn binding_id(index: usize) -> i32 {
    BINDING_IDS.saturating_add(index as i32)
}

/// The id of the gesture profile at `index`.
pub fn gesture_id(index: usize) -> i32 {
    GESTURE_IDS.saturating_add(index as i32)
}

/// Which binding of `Config::live_bindings` an id is, if it is one.
pub fn binding_index(id: i32) -> Option<usize> {
    (BINDING_IDS..GESTURE_IDS)
        .contains(&id)
        .then(|| (id - BINDING_IDS) as usize)
}

/// Which gesture profile an id is, if it is one.
pub fn gesture_index(id: i32) -> Option<usize> {
    (id >= GESTURE_IDS).then(|| (id - GESTURE_IDS) as usize)
}

/// A shortcut as it is written (`Ctrl+Alt+T`), for the log.
pub fn shortcut_name(key: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> String {
    let mut name = String::new();
    for (held, modifier) in [(win, "Win"), (ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")] {
        if held {
            name.push_str(modifier);
            name.push('+');
        }
    }
    name.push_str(key);
    name
}

/// The hotkeys that are live: every key node of the graph (they are not
/// in a submap), and the enabled bindings of `submap`.
pub fn live_bindings(config: &Config, graph: &NodeGraph, submap: &str) -> Vec<(i32, Hotkey)> {
    let mut bindings = Vec::new();
    for (index, (_, node)) in graph.nodes.iter().enumerate() {
        if let NodeType::OnKeyPress {
            key,
            ctrl,
            alt,
            shift,
            win,
        } = &node.node_type
        {
            if !key.is_empty() {
                bindings.push((
                    graph_key_id(index),
                    Hotkey::new(key.clone(), *ctrl, *alt, *shift, *win),
                ));
            }
        }
    }
    for (index, binding) in config.live_bindings().iter().enumerate() {
        if !binding.enabled
            || binding.key.trim().is_empty()
            || binding.normalized_submap() != submap
        {
            continue;
        }
        bindings.push((
            binding_id(index),
            Hotkey {
                key: binding.key.clone(),
                ctrl: binding.ctrl,
                alt: binding.alt,
                shift: binding.shift,
                win: binding.win,
                device: binding.device,
                trigger: binding.trigger,
                consume: binding.consume,
                long_press_ms: binding.long_press_ms,
            },
        ));
    }
    bindings
}

/// The host's bindings, for every backend.
pub trait Bindings: WmHost {
    /// Take the bindings that are live now: those of the active submap (or
    /// of `default`, where the configuration has no such submap any more),
    /// and the graph's key nodes.
    fn refresh_bindings(&mut self) -> anyhow::Result<()> {
        let bindings = {
            let parts = self.parts();
            let submap = &mut parts.host.active_submap;
            if submap != "default"
                && !parts
                    .config
                    .desktop
                    .bindings
                    .iter()
                    .any(|binding| binding.enabled && binding.normalized_submap() == *submap)
            {
                log::warn!("Binding submap '{submap}' no longer exists; returning to default");
                *submap = "default".to_string();
            }
            live_bindings(parts.config, parts.core.executor.get_graph(), submap)
        };
        log::info!(
            "{} binding(s) live in submap '{}'",
            bindings.len(),
            self.host().active_submap
        );
        self.install_bindings(bindings)
    }

    /// A key (or button, or wheel turn) hit the bindings with the ids of
    /// `event.registration_ids`: run the configuration's bindings among
    /// them, and the graph's key nodes where it hit one.
    fn bindings_triggered(&mut self, event: &HotkeyEvent) {
        let ids = &event.registration_ids;
        // (Only a shortcut that is registered comes here: its name, never
        // a key that was typed.)
        log::debug!(
            "the shortcut {} was used ({:?}, {:?}, held {} ms)",
            shortcut_name(&event.key, event.ctrl, event.alt, event.shift, event.win),
            event.device,
            event.trigger,
            event.held_ms,
        );
        self.host().emit(
            RuntimeEventKind::BindingTriggered,
            None,
            [
                ("device".to_string(), format!("{:?}", event.device)),
                ("trigger".to_string(), format!("{:?}", event.trigger)),
                ("key".to_string(), event.key.clone()),
                ("ctrl".to_string(), event.ctrl.to_string()),
                ("alt".to_string(), event.alt.to_string()),
                ("shift".to_string(), event.shift.to_string()),
                ("win".to_string(), event.win.to_string()),
                ("held_ms".to_string(), event.held_ms.to_string()),
            ],
        );
        let commands: Vec<_> = {
            let live = self.config().live_bindings();
            ids.iter()
                .filter_map(|id| live.get(binding_index(*id)?).map(|b| b.command.clone()))
                .collect()
        };
        for command in commands {
            if let Err(error) = self.run_command(command) {
                log::error!("Binding failed: {error}");
            }
        }
        if !ids.iter().any(|id| *id < 0) {
            return;
        }
        let parts = self.parts();
        if let Err(error) = parts.core.executor.execute_hotkey(
            parts.desktop,
            &mut parts.core.monitor_manager,
            &mut parts.core.focus_manager,
            event,
        ) {
            log::error!("Key node failed: {error}");
        }
        // Focus cycling asked for by a node.
        if let Some(operation) = parts.core.executor.take_pending_focus_op() {
            let target = match operation {
                PendingFocusOp::CycleNext => parts.core.focus_manager.cycle_next(),
                PendingFocusOp::CyclePrevious => parts.core.focus_manager.cycle_previous(),
            };
            if let Some(id) = target {
                if let Err(error) = gevrek_engine::Desktop::apply(parts.desktop, &Effect::Focus(Some(id)))
                {
                    log::error!("focus {id:?}: {error}");
                }
            }
        }
        self.settle();
    }
}

impl<T: WmHost + ?Sized> Bindings for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_says_what_it_is_for() {
        assert_eq!(graph_key_id(0), -1);
        assert_eq!(graph_key_id(4), -5);
        assert_eq!(binding_index(binding_id(0)), Some(0));
        assert_eq!(binding_index(binding_id(37)), Some(37));
        assert_eq!(gesture_index(gesture_id(3)), Some(3));
        // An id of one kind is none of the others.
        assert_eq!(binding_index(graph_key_id(2)), None);
        assert_eq!(binding_index(gesture_id(0)), None);
        assert_eq!(gesture_index(binding_id(5)), None);
        assert_eq!(gesture_index(-1), None);
    }

    #[test]
    fn the_graphs_keys_are_live_in_every_submap_and_bindings_only_in_theirs() {
        let mut config = Config::new();
        let key = |key: &str, submap: &str| gevrek_model::settings::BindingProfile {
            key: key.to_string(),
            submap: submap.to_string(),
            enabled: true,
            ..Default::default()
        };
        // (Keys the standard bindings, which are live too, do not use.)
        config.desktop.bindings =
            vec![key("F13", "default"), key("F14", "resize"), key("", "default")];
        let mut graph = NodeGraph::new();
        graph.add_node(
            NodeType::OnKeyPress {
                key: "F15".to_string(),
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
            },
            [0.0, 0.0],
        );
        let index_of = |wanted: &str| {
            config
                .live_bindings()
                .iter()
                .position(|binding| binding.key == wanted)
                .unwrap()
        };
        let mine = |submap| -> Vec<(i32, String)> {
            live_bindings(&config, &graph, submap)
                .into_iter()
                .filter(|(_, hotkey)| ["F13", "F14", "F15", ""].contains(&hotkey.key.as_str()))
                .map(|(id, hotkey)| (id, hotkey.key))
                .collect()
        };
        assert_eq!(
            mine("default"),
            vec![(-1, "F15".to_string()), (binding_id(index_of("F13")), "F13".to_string())]
        );
        assert_eq!(
            mine("resize"),
            vec![(-1, "F15".to_string()), (binding_id(index_of("F14")), "F14".to_string())]
        );
    }
}
