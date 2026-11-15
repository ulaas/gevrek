//! The shortcuts of a configuration, as a list for people to read.
//!
//! A configuration assigns keys in two places: the binding profiles
//! (`desktop.bindings`) and the key nodes of the automation graph. Both are
//! listed here in one form.

use crate::config::Config;
use crate::graph::{NodeType, PinType};
use crate::settings::{BindingDevice, BindingTrigger};

/// One assignment: what to press and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// `Ctrl+Alt+J`
    pub keys: String,
    /// `Focus next window`
    pub action: String,
    /// Where it is defined, for the reader who wants to change it.
    pub source: ShortcutSource,
    /// The binding submap it belongs to; `default` for the usual one.
    pub submap: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutSource {
    /// A binding profile (Configure page, "Bindings").
    Binding,
    /// A key node of the automation graph.
    Graph,
}

/// How the Super key is called on this system.
pub fn super_key_name() -> &'static str {
    if cfg!(windows) {
        "Win"
    } else {
        "Super"
    }
}

fn chord(ctrl: bool, alt: bool, shift: bool, super_key: bool, key: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if super_key {
        parts.push(super_key_name());
    }
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    let key = key.trim();
    let key = if key.len() == 1 {
        key.to_ascii_uppercase()
    } else {
        key.to_string()
    };
    parts.push(&key);
    parts.join("+")
}

/// The keys a configuration has for one command (given as the JSON a
/// binding holds for it), as people read them: the bindings of the
/// keyboard that are switched on, in the usual mode, pressed (not held
/// or let go), in the configuration's order. For whoever has to say
/// "press this" of a command: the tutorial.
pub fn keys_of_command(config: &Config, command: &serde_json::Value) -> Vec<String> {
    config
        .desktop
        .bindings
        .iter()
        .filter(|binding| {
            binding.enabled
                && binding.device == BindingDevice::Keyboard
                && binding.trigger == BindingTrigger::Press
                && binding.normalized_submap() == "default"
                && serde_json::to_value(&binding.command).ok().as_ref() == Some(command)
        })
        .map(|binding| {
            chord(
                binding.ctrl,
                binding.alt,
                binding.shift,
                binding.win,
                &binding.key,
            )
        })
        .collect()
}

/// Every enabled key assignment of `config`: the binding profiles first, in
/// their order, then the graph's key nodes by key.
pub fn shortcuts_of(config: &Config) -> Vec<Shortcut> {
    let mut list = Vec::new();
    for binding in config
        .desktop
        .bindings
        .iter()
        .filter(|binding| binding.enabled)
    {
        let mut keys = chord(
            binding.ctrl,
            binding.alt,
            binding.shift,
            binding.win,
            &binding.key,
        );
        if binding.device == BindingDevice::Mouse {
            keys = format!("Mouse {keys}");
        }
        match binding.trigger {
            BindingTrigger::Press => {}
            other => keys = format!("{keys} ({other:?})").to_string(),
        }
        let command = binding.command.display_name();
        let name = binding.name.trim();
        let action =
            if name.is_empty() || name == "New binding" || name.eq_ignore_ascii_case(command) {
                command.to_string()
            } else {
                format!("{name} ({command})")
            };
        list.push(Shortcut {
            keys,
            action,
            source: ShortcutSource::Binding,
            submap: binding.normalized_submap(),
        });
    }

    let graph = &config.node_graph;
    let mut from_graph = Vec::new();
    for node in graph.nodes.values() {
        let NodeType::OnKeyPress {
            key,
            ctrl,
            alt,
            shift,
            win,
        } = &node.node_type
        else {
            continue;
        };
        // What the key leads to: the nodes wired to its execution output.
        let execution_pins: Vec<_> = node
            .outputs
            .iter()
            .filter(|pin| pin.pin_type == PinType::Execution)
            .map(|pin| pin.id)
            .collect();
        let mut actions: Vec<String> = graph
            .connections
            .iter()
            .filter(|connection| {
                connection.from_node == node.id && execution_pins.contains(&connection.from_pin)
            })
            .filter_map(|connection| graph.nodes.get(&connection.to_node))
            .map(|target| match &target.node_type {
                // Which program: the file name, without its folders.
                crate::graph::NodeType::LaunchProcess { program, .. }
                    if !program.trim().is_empty() =>
                {
                    let name = program.trim().rsplit(['/', '\\']).next().unwrap_or(program);
                    format!("{} ({name})", target.node_type.display_name())
                }
                _ => target.node_type.display_name().to_string(),
            })
            .collect();
        actions.sort();
        actions.dedup();
        if actions.is_empty() {
            // A key node that leads nowhere does nothing.
            continue;
        }
        from_graph.push(Shortcut {
            keys: chord(*ctrl, *alt, *shift, *win, key),
            action: actions.join(", "),
            source: ShortcutSource::Graph,
            submap: "default".to_string(),
        });
    }
    from_graph.sort_by(|a, b| a.keys.cmp(&b.keys));
    list.extend(from_graph);
    list
}

/// The names people type for the Super key, whatever this system calls it
/// ([`super_key_name`]): the documentation says Super, the keyboard says
/// Win.
const SUPER_KEY_NAMES: [&str; 4] = ["super", "win", "windows", "meta"];

/// Whether `word` is one of the Super key's names, or the start of one
/// (three letters at least, so that "me" does not find every chord).
pub fn means_the_super_key(word: &str) -> bool {
    word.chars().count() >= 3 && SUPER_KEY_NAMES.iter().any(|name| name.starts_with(word))
}

/// The rows of `list` that match what was typed, in the list's order:
/// every word has to be found, in the keys or in what they do. The same
/// search wherever the list is shown (the settings program's window, the
/// shell's surface).
pub fn filtered<'a>(list: &'a [Shortcut], filter: &str) -> Vec<&'a Shortcut> {
    let wanted = filter.trim().to_lowercase();
    let super_key = super_key_name().to_lowercase();
    list.iter()
        .filter(|shortcut| {
            let keys = shortcut.keys.to_lowercase();
            wanted.is_empty()
                || wanted.split_whitespace().all(|word| {
                    keys.contains(word)
                        || shortcut.action.to_lowercase().contains(word)
                        || (means_the_super_key(word) && keys.contains(&super_key))
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::DispatchCommand;
    use crate::settings::BindingProfile;

    #[test]
    fn the_default_configuration_lists_its_graph_keys_and_the_key_for_this_list() {
        let config = Config::with_default_graph();
        let list = shortcuts_of(&config);
        // The key that opens the list is in the list.
        let own = list
            .iter()
            .find(|shortcut| shortcut.action.contains("Show shortcuts"))
            .expect("the default configuration has a key for the list of shortcuts");
        assert_eq!(own.source, ShortcutSource::Binding);
        assert_eq!(own.keys, format!("{}+F1", super_key_name()));
        // The graph's key nodes are there with what they lead to.
        assert!(list.iter().any(|shortcut| shortcut.keys == "Ctrl+Alt+J"
            && shortcut.action == "Focus Next Window"
            && shortcut.source == ShortcutSource::Graph));
        // Nothing is listed twice.
        let mut seen = std::collections::HashSet::new();
        for shortcut in &list {
            assert!(
                seen.insert((shortcut.keys.clone(), shortcut.action.clone())),
                "{shortcut:?} is listed twice"
            );
        }
    }

    #[test]
    fn the_search_matches_keys_and_actions_without_regard_to_case() {
        let list = shortcuts_of(&crate::examples::starting_configuration());
        assert!(list.len() > 50);
        assert_eq!(filtered(&list, "").len(), list.len());
        assert!(filtered(&list, "SHORTCUTS")
            .iter()
            .any(|shortcut| shortcut.action.contains("hortcuts")));
        // Every word has to be found, in the keys or in what they do.
        // (The Super key has the name this system gives it.)
        let wanted = format!("{} close", super_key_name());
        let narrowed = filtered(&list, &wanted);
        assert!(!narrowed.is_empty() && narrowed.len() < list.len());
        assert!(filtered(&list, "no such thing").is_empty());
    }

    #[test]
    fn the_super_key_is_found_by_any_of_its_names() {
        let list = shortcuts_of(&crate::examples::starting_configuration());
        // (Counted from the keys alone: "win" is also in "window".)
        let name_here = super_key_name().to_lowercase();
        let by_its_name_here = list
            .iter()
            .filter(|shortcut| shortcut.keys.to_lowercase().contains(&name_here))
            .count();
        assert!(by_its_name_here > 0);
        // "Super" in the documentation, "Win" on the keyboard: both find
        // the chords, on either system, and so does the start of a name.
        for name in ["super", "Win", "windows", "META", "sup"] {
            assert!(filtered(&list, name).len() >= by_its_name_here, "{name}");
            assert!(
                !filtered(&list, &format!("{name} close")).is_empty(),
                "{name} close"
            );
        }
        // Two letters are not yet a name of the key.
        assert!(!means_the_super_key("me") && !means_the_super_key("su"));
        assert!(!means_the_super_key("supra"));
    }

    #[test]
    fn bindings_are_listed_with_their_keys_names_and_submaps() {
        let mut config = Config::new();
        config.desktop.bindings = vec![
            BindingProfile {
                name: "Overview".to_string(),
                key: "o".to_string(),
                ctrl: true,
                command: DispatchCommand::ToggleExpose,
                ..BindingProfile::default()
            },
            BindingProfile {
                name: "Off".to_string(),
                key: "X".to_string(),
                enabled: false,
                ..BindingProfile::default()
            },
            BindingProfile {
                key: "H".to_string(),
                win: false,
                submap: "Resize".to_string(),
                command: DispatchCommand::FocusNext,
                ..BindingProfile::default()
            },
        ];
        let list = shortcuts_of(&config);
        assert_eq!(list.len(), 2, "a disabled binding is not listed: {list:?}");
        assert_eq!(list[0].keys, format!("{}+Ctrl+O", super_key_name()));
        assert_eq!(list[0].action, "Overview (Toggle overview)");
        assert_eq!(list[0].submap, "default");
        assert_eq!(list[1].keys, "H");
        assert_eq!(list[1].action, DispatchCommand::FocusNext.display_name());
        assert_eq!(list[1].submap, "resize");
    }
}
