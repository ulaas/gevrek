//! What can be done to a graph: wires made and taken away, nodes removed,
//! a selection copied and put back in.
//!
//! The model decides what a valid wire is (`NodeGraph::add_connection`);
//! [`verdict`] says the same beforehand, so that the canvas can show it
//! while a wire is still being pulled. A test holds the two together.

use super::geometry::{PinRef, WireRef};
use gevrek_model::graph::{Connection, Node, NodeGraph, NodeId, PinType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// What would happen if two pins were wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A wire is made. A data input has one wire: the one it had goes.
    Wire,
    /// The kinds differ but one is made from the other: a node that does
    /// it is put in between.
    Convert,
    /// No wire, and why.
    Refuse(&'static str),
}

impl Verdict {
    pub fn makes_a_wire(self) -> bool {
        !matches!(self, Verdict::Refuse(_))
    }
}

/// The two pins as the output and the input, whichever was picked first.
pub fn oriented(a: PinRef, b: PinRef) -> Option<(PinRef, PinRef)> {
    match (a.output, b.output) {
        (true, false) => Some((a, b)),
        (false, true) => Some((b, a)),
        _ => None,
    }
}

fn pin_type(graph: &NodeGraph, pin: PinRef) -> Option<&PinType> {
    let node = graph.nodes.get(&pin.node)?;
    let pins = if pin.output {
        &node.outputs
    } else {
        &node.inputs
    };
    pins.iter()
        .find(|other| other.id == pin.pin)
        .map(|pin| &pin.pin_type)
}

pub fn verdict(graph: &NodeGraph, a: PinRef, b: PinRef) -> Verdict {
    if a == b {
        return Verdict::Refuse("This is the pin the wire starts at.");
    }
    let Some((from, to)) = oriented(a, b) else {
        return Verdict::Refuse(if a.output {
            "An output goes to an input, not to another output."
        } else {
            "An input takes an output, not another input."
        });
    };
    let (Some(from_type), Some(to_type)) = (pin_type(graph, from), pin_type(graph, to)) else {
        return Verdict::Refuse("The pin is gone.");
    };
    let already = graph.connections.iter().any(|connection| {
        connection.from_node == from.node
            && connection.from_pin == from.pin
            && connection.to_node == to.node
            && connection.to_pin == to.pin
    });
    if already {
        return Verdict::Refuse("These two are wired already.");
    }
    let fits = graph
        .nodes
        .get(&to.node)
        .is_some_and(|node| gevrek_model::graph::pin_takes(&node.node_type, to_type, from_type));
    if fits {
        return Verdict::Wire;
    }
    // (A number or a yes-or-no into text: the node that writes it out
    // is put in between.)
    let to_text =
        matches!(from_type, PinType::Number | PinType::Bool) && *to_type == PinType::String;
    let from_text = *from_type == PinType::String && *to_type == PinType::Number;
    if to_text || from_text {
        Verdict::Convert
    } else {
        Verdict::Refuse("These two do not carry the same kind of thing.")
    }
}

/// Wire two pins, picked in either order. Returns whether a wire was made.
pub fn connect(graph: &mut NodeGraph, a: PinRef, b: PinRef) -> bool {
    let Some((from, to)) = oriented(a, b) else {
        return false;
    };
    graph.add_connection(Connection {
        from_node: from.node,
        from_pin: from.pin,
        to_node: to.node,
        to_pin: to.pin,
    })
}

/// Take away every wire of a pin. Returns how many there were.
pub fn disconnect(graph: &mut NodeGraph, pin: PinRef) -> usize {
    let before = graph.connections.len();
    if pin.output {
        graph.remove_all_connections_from_output(pin.node, pin.pin);
    } else {
        graph.remove_all_connections_to_input(pin.node, pin.pin);
    }
    before - graph.connections.len()
}

pub fn remove_wire(graph: &mut NodeGraph, wire: WireRef) -> bool {
    let before = graph.connections.len();
    graph.remove_connection(wire.from_node, wire.from_pin, wire.to_node, wire.to_pin);
    graph.connections.len() != before
}

/// Remove nodes, and with them their wires.
pub fn remove_nodes(graph: &mut NodeGraph, nodes: &BTreeSet<NodeId>) -> usize {
    let before = graph.nodes.len();
    for id in nodes {
        graph.remove_node(*id);
    }
    before - graph.nodes.len()
}

/// Nodes taken out of a graph to be put in again, with the wires that run
/// between them. It is written as JSON onto the system's clipboard, so a
/// copy can be pasted into another window of the studio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    /// The key says what the text is, should anything else be pasted.
    #[serde(rename = "gevrek_nodes")]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub wires: Vec<Connection>,
}

impl Clip {
    pub fn from_text(text: &str) -> Option<Clip> {
        let clip: Clip = serde_json::from_str(text).ok()?;
        (!clip.nodes.is_empty()).then_some(clip)
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The top left corner of what was copied.
    pub fn corner(&self) -> [f32; 2] {
        let least = |axis: usize| {
            self.nodes
                .iter()
                .map(|node| node.position[axis])
                .fold(f32::INFINITY, f32::min)
        };
        [least(0), least(1)]
    }
}

/// The selected nodes and the wires that both start and end among them.
pub fn copy(graph: &NodeGraph, selected: &BTreeSet<NodeId>) -> Clip {
    let nodes = selected
        .iter()
        .filter_map(|id| graph.nodes.get(id))
        .cloned()
        .collect();
    let wires = graph
        .connections
        .iter()
        .filter(|wire| selected.contains(&wire.from_node) && selected.contains(&wire.to_node))
        .cloned()
        .collect();
    Clip { nodes, wires }
}

/// Put a copy into a graph, moved by `by`. The nodes and their pins get
/// ids of this graph; the wires follow them. Returns the new nodes.
///
/// A pin is found again by its place among the node's pins, since a node
/// of one type always has the same pins. A node this version does not
/// know is pasted without pins (the model gives it none), so its wires
/// are left out.
pub fn paste(graph: &mut NodeGraph, clip: &Clip, by: [f32; 2]) -> Vec<NodeId> {
    let mut new_nodes: HashMap<NodeId, NodeId> = HashMap::new();
    let mut new_pins: HashMap<PinRef, PinRef> = HashMap::new();
    let mut made = Vec::new();
    for old in &clip.nodes {
        let id = graph.add_node(
            old.node_type.clone(),
            [old.position[0] + by[0], old.position[1] + by[1]],
        );
        let new = &graph.nodes[&id];
        for (output, old_pins, pins) in [
            (false, &old.inputs, &new.inputs),
            (true, &old.outputs, &new.outputs),
        ] {
            for (old_pin, pin) in old_pins.iter().zip(pins) {
                new_pins.insert(
                    PinRef {
                        node: old.id,
                        pin: old_pin.id,
                        output,
                    },
                    PinRef {
                        node: id,
                        pin: pin.id,
                        output,
                    },
                );
            }
        }
        new_nodes.insert(old.id, id);
        made.push(id);
    }
    for wire in &clip.wires {
        let from = new_pins.get(&PinRef {
            node: wire.from_node,
            pin: wire.from_pin,
            output: true,
        });
        let to = new_pins.get(&PinRef {
            node: wire.to_node,
            pin: wire.to_pin,
            output: false,
        });
        if let (Some(from), Some(to)) = (from, to) {
            graph.add_connection(Connection {
                from_node: from.node,
                from_pin: from.pin,
                to_node: to.node,
                to_pin: to.pin,
            });
        }
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::graph::NodeType;

    fn first(graph: &NodeGraph, node: NodeId, kind: PinType, output: bool) -> PinRef {
        let pins = if output {
            &graph.nodes[&node].outputs
        } else {
            &graph.nodes[&node].inputs
        };
        let found = pins
            .iter()
            .find(|pin| pin.pin_type == kind)
            .expect("a pin of that kind");
        PinRef {
            node,
            pin: found.id,
            output,
        }
    }

    /// Wire the first output of a kind of one node to the first input of
    /// that kind of another.
    fn join(graph: &mut NodeGraph, from: NodeId, to: NodeId, kind: PinType) -> bool {
        let (from, to) = (
            first(graph, from, kind.clone(), true),
            first(graph, to, kind, false),
        );
        connect(graph, from, to)
    }

    /// Startup -> Log, with a number and a text to feed it.
    fn small() -> (NodeGraph, [NodeId; 4]) {
        let mut graph = NodeGraph::new();
        let start = graph.add_node(NodeType::OnStartup, [0.0, 0.0]);
        let log = graph.add_node(
            NodeType::LogMessage {
                message: String::new(),
                level: gevrek_model::graph::LogLevel::Info,
            },
            [300.0, 0.0],
        );
        let number = graph.add_node(NodeType::NumberValue { value: 4.0 }, [0.0, 200.0]);
        let text = graph.add_node(
            NodeType::StringValue {
                value: "hello".to_string(),
            },
            [0.0, 300.0],
        );
        (graph, [start, log, number, text])
    }

    #[test]
    fn a_wire_runs_from_an_output_to_an_input_of_its_kind() {
        let (mut graph, [start, log, number, text]) = small();
        let out = first(&graph, start, PinType::Execution, true);
        let run = first(&graph, log, PinType::Execution, false);
        let message = first(&graph, log, PinType::String, false);
        let words = first(&graph, text, PinType::String, true);
        let value = first(&graph, number, PinType::Number, true);

        assert_eq!(verdict(&graph, out, run), Verdict::Wire);
        // Picked the other way round, it is the same wire.
        assert_eq!(verdict(&graph, run, out), Verdict::Wire);
        assert!(matches!(verdict(&graph, out, out), Verdict::Refuse(_)));
        assert!(
            matches!(verdict(&graph, out, value), Verdict::Refuse(_)),
            "two outputs"
        );
        assert!(
            matches!(verdict(&graph, run, message), Verdict::Refuse(_)),
            "two inputs"
        );
        assert!(
            matches!(verdict(&graph, out, message), Verdict::Refuse(_)),
            "the order of things is not a text"
        );
        assert_eq!(verdict(&graph, words, message), Verdict::Wire);
        assert_eq!(verdict(&graph, value, message), Verdict::Convert);

        assert!(connect(&mut graph, run, out));
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(
            (graph.connections[0].from_node, graph.connections[0].to_node),
            (start, log)
        );
        assert!(
            matches!(verdict(&graph, out, run), Verdict::Refuse(_)),
            "wired already"
        );
        assert!(!connect(&mut graph, out, run));
    }

    #[test]
    fn a_switch_wired_to_text_is_written_out_by_a_node_in_between() {
        let (mut graph, [_, log, _, _]) = small();
        let switch = graph.add_node(NodeType::BoolValue { value: true }, [0.0, 400.0]);
        let on = first(&graph, switch, PinType::Bool, true);
        let message = first(&graph, log, PinType::String, false);
        assert_eq!(verdict(&graph, on, message), Verdict::Convert);
        let nodes = graph.nodes.len();
        assert!(connect(&mut graph, on, message));
        assert_eq!(graph.nodes.len(), nodes + 1, "a node in between");
        let between = graph
            .nodes
            .values()
            .find(|node| matches!(node.node_type, NodeType::ToString))
            .expect("the node that writes a value out")
            .id;
        assert!(graph
            .connections
            .iter()
            .any(|wire| wire.from_node == switch && wire.to_node == between));
        assert!(graph
            .connections
            .iter()
            .any(|wire| wire.from_node == between && wire.to_node == log));
        // The window manager keeps both wires when it loads the graph.
        assert!(graph.problems().is_empty(), "{:?}", graph.problems());
        let wires = graph.connections.len();
        graph.refresh_node_pins();
        assert_eq!(graph.validate_and_clean(), (0, 0));
        assert_eq!(graph.connections.len(), wires);
        // A switch straight into that node's own input fits too; into
        // any other number it does not.
        let other = graph.add_node(NodeType::ToString, [0.0, 500.0]);
        let into = first(&graph, other, PinType::Number, false);
        assert_eq!(verdict(&graph, on, into), Verdict::Wire);
        let sum = graph.add_node(NodeType::Add, [0.0, 600.0]);
        let number = first(&graph, sum, PinType::Number, false);
        assert!(matches!(verdict(&graph, on, number), Verdict::Refuse(_)));
    }

    #[test]
    fn the_verdict_is_what_the_model_then_does() {
        // Every output against every input of a graph with many kinds of pins.
        let mut graph = NodeGraph::new();
        let mut at = 0.0;
        for (_, types) in gevrek_model::graph::node_catalog() {
            for node_type in types.into_iter().take(3) {
                graph.add_node(node_type, [at, 0.0]);
                at += 250.0;
            }
        }
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        for node in graph.nodes.values() {
            outputs.extend(node.outputs.iter().map(|pin| PinRef {
                node: node.id,
                pin: pin.id,
                output: true,
            }));
            inputs.extend(node.inputs.iter().map(|pin| PinRef {
                node: node.id,
                pin: pin.id,
                output: false,
            }));
        }
        assert!(outputs.len() > 20 && inputs.len() > 20);
        let (mut wired, mut converted, mut refused) = (0, 0, 0);
        for from in &outputs {
            for to in &inputs {
                let mut trial = graph.clone();
                let said = verdict(&trial, *from, *to);
                let nodes_before = trial.nodes.len();
                let made = connect(&mut trial, *from, *to);
                assert_eq!(made, said.makes_a_wire(), "{from:?} -> {to:?}: {said:?}");
                match said {
                    Verdict::Wire => {
                        assert_eq!(trial.nodes.len(), nodes_before);
                        wired += 1;
                    }
                    Verdict::Convert => {
                        assert_eq!(trial.nodes.len(), nodes_before + 1, "a node in between");
                        converted += 1;
                    }
                    Verdict::Refuse(_) => refused += 1,
                }
            }
        }
        assert!(
            wired > 0 && converted > 0 && refused > 0,
            "{wired} wired, {converted} converted, {refused} refused"
        );
    }

    #[test]
    fn a_data_input_keeps_one_wire_and_the_order_of_things_many() {
        let (mut graph, [start, log, _, text]) = small();
        let other = graph.add_node(
            NodeType::StringValue {
                value: "other".to_string(),
            },
            [0.0, 400.0],
        );
        let message = first(&graph, log, PinType::String, false);
        assert!(join(&mut graph, text, log, PinType::String));
        assert_eq!(
            verdict(&graph, first(&graph, other, PinType::String, true), message),
            Verdict::Wire
        );
        assert!(join(&mut graph, other, log, PinType::String));
        assert_eq!(
            graph.connections.len(),
            1,
            "the new wire took the place of the old"
        );
        assert_eq!(graph.connections[0].from_node, other);

        let again = graph.add_node(NodeType::OnStartupCompleted, [0.0, 500.0]);
        let run = first(&graph, log, PinType::Execution, false);
        assert!(join(&mut graph, start, log, PinType::Execution));
        assert!(join(&mut graph, again, log, PinType::Execution));
        assert_eq!(graph.connections.len(), 3);

        assert_eq!(disconnect(&mut graph, run), 2);
        assert_eq!(disconnect(&mut graph, run), 0);
        let last = &graph.connections[0];
        let wire = WireRef {
            from_node: last.from_node,
            from_pin: last.from_pin,
            to_node: last.to_node,
            to_pin: last.to_pin,
        };
        assert!(remove_wire(&mut graph, wire));
        assert!(!remove_wire(&mut graph, wire));
        assert!(graph.connections.is_empty());
    }

    #[test]
    fn removing_nodes_removes_their_wires() {
        let (mut graph, [start, log, _, text]) = small();
        assert!(join(&mut graph, start, log, PinType::Execution));
        assert!(join(&mut graph, text, log, PinType::String));
        assert_eq!(remove_nodes(&mut graph, &BTreeSet::from([start, text])), 2);
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.connections.is_empty());
        // Whatever is left is a graph the model accepts as it is.
        assert_eq!(graph.validate_and_clean(), (0, 0));
    }

    #[test]
    fn a_copy_is_pasted_with_new_ids_and_its_inner_wires() {
        let (mut graph, [start, log, number, text]) = small();
        assert!(join(&mut graph, start, log, PinType::Execution));
        assert!(join(&mut graph, text, log, PinType::String));
        let before = graph.clone();

        // The log and its text, not the node that starts it.
        let clip = copy(&graph, &BTreeSet::from([log, text]));
        assert_eq!(clip.nodes.len(), 2);
        assert_eq!(
            clip.wires.len(),
            1,
            "only the wire between the two copied nodes"
        );
        assert_eq!(clip.corner(), [0.0, 0.0]);

        // Through the clipboard's text and back.
        let clip = Clip::from_text(&clip.to_text()).expect("read back");
        assert!(Clip::from_text("hello").is_none());
        assert!(Clip::from_text(r#"{"gevrek_nodes": []}"#).is_none());

        let made = paste(&mut graph, &clip, [40.0, 60.0]);
        assert_eq!(made.len(), 2);
        assert_eq!(graph.nodes.len(), 6);
        assert_eq!(graph.connections.len(), 3);
        for id in &made {
            assert!(![start, log, number, text].contains(id), "a new id");
        }
        // No pin id is used twice.
        let mut pins: Vec<usize> = graph
            .nodes
            .values()
            .flat_map(|node| node.inputs.iter().chain(&node.outputs))
            .map(|pin| pin.id)
            .collect();
        let count = pins.len();
        pins.sort_unstable();
        pins.dedup();
        assert_eq!(pins.len(), count);

        // The new wire joins the two new nodes, at the pins of the same name.
        let wire = graph.connections.last().expect("the pasted wire").clone();
        assert!(made.contains(&wire.from_node) && made.contains(&wire.to_node));
        let new_text = &graph.nodes[&wire.from_node];
        let new_log = &graph.nodes[&wire.to_node];
        assert_eq!(
            new_text.node_type,
            NodeType::StringValue {
                value: "hello".to_string()
            }
        );
        assert_eq!(new_text.position, [40.0, 360.0]);
        let old_wire = &before.connections[1];
        let name = |node: &Node, pin: usize, output: bool| {
            let pins = if output { &node.outputs } else { &node.inputs };
            pins.iter()
                .find(|other| other.id == pin)
                .map(|pin| pin.name.clone())
        };
        assert_eq!(
            name(new_log, wire.to_pin, false),
            name(&before.nodes[&log], old_wire.to_pin, false)
        );
        assert_eq!(
            name(new_text, wire.from_pin, true),
            name(&before.nodes[&text], old_wire.from_pin, true)
        );

        // The originals are as they were.
        for (id, node) in &before.nodes {
            assert_eq!(graph.nodes[id].position, node.position);
        }
        assert_eq!(graph.validate_and_clean(), (0, 0));
        // And the whole is a graph the configuration reads back.
        let json = serde_json::to_value(&graph).expect("JSON");
        let read: NodeGraph = serde_json::from_value(json).expect("read back");
        assert_eq!(read.nodes.len(), 6);
    }
}
