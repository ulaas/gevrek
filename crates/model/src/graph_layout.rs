//! Putting the nodes of a graph in order.
//!
//! The nodes are laid out from left to right the way things happen: a node
//! stands to the right of every node that leads to it. Chains that have
//! nothing to do with each other get rows of their own, one under the other.
//! Nothing but the positions of the nodes is changed.

use crate::graph::{NodeGraph, NodeId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The sizes the editor draws a node with (see the editor's canvas).
pub const NODE_WIDTH: f32 = 200.0;
const HEADER_HEIGHT: f32 = 30.0;
const PIN_HEIGHT: f32 = 20.0;
const COLUMN_GAP: f32 = 90.0;
const ROW_GAP: f32 = 28.0;
const GROUP_GAP: f32 = 64.0;
/// Groups are put side by side up to this width.
const SHELF_WIDTH: f32 = 2000.0;

fn node_height(graph: &NodeGraph, id: NodeId) -> f32 {
    let pins = graph
        .nodes
        .get(&id)
        .map(|node| node.inputs.len().max(node.outputs.len()))
        .unwrap_or(0);
    HEADER_HEIGHT + pins as f32 * PIN_HEIGHT + 10.0
}

/// Lay the graph out anew. Returns how many nodes were moved.
pub fn tidy(graph: &mut NodeGraph) -> usize {
    let mut ids: Vec<NodeId> = graph.nodes.keys().copied().collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return 0;
    }
    let mut leads_to: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut comes_from: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for connection in &graph.connections {
        if connection.from_node == connection.to_node
            || !graph.nodes.contains_key(&connection.from_node)
            || !graph.nodes.contains_key(&connection.to_node)
        {
            continue;
        }
        leads_to
            .entry(connection.from_node)
            .or_default()
            .push(connection.to_node);
        comes_from
            .entry(connection.to_node)
            .or_default()
            .push(connection.from_node);
    }

    // The column of a node: one more than the furthest column of what leads
    // to it. A loop is cut where it is met a second time.
    fn column_of(
        id: NodeId,
        comes_from: &HashMap<NodeId, Vec<NodeId>>,
        known: &mut HashMap<NodeId, usize>,
        on_the_way: &mut HashSet<NodeId>,
    ) -> usize {
        if let Some(column) = known.get(&id) {
            return *column;
        }
        if !on_the_way.insert(id) {
            return 0;
        }
        let column = comes_from
            .get(&id)
            .map(|sources| {
                sources
                    .iter()
                    .map(|source| column_of(*source, comes_from, known, on_the_way) + 1)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        on_the_way.remove(&id);
        known.insert(id, column);
        column
    }
    let mut columns: HashMap<NodeId, usize> = HashMap::new();
    for id in &ids {
        column_of(*id, &comes_from, &mut columns, &mut HashSet::new());
    }

    // The groups: nodes that are joined by wires, directly or not.
    let mut group_of: HashMap<NodeId, usize> = HashMap::new();
    let mut groups: Vec<Vec<NodeId>> = Vec::new();
    for id in &ids {
        if group_of.contains_key(id) {
            continue;
        }
        let index = groups.len();
        let mut members = Vec::new();
        let mut open = vec![*id];
        while let Some(current) = open.pop() {
            if group_of.insert(current, index).is_some() {
                continue;
            }
            members.push(current);
            for next in leads_to.get(&current).into_iter().flatten() {
                open.push(*next);
            }
            for next in comes_from.get(&current).into_iter().flatten() {
                open.push(*next);
            }
        }
        members.sort_unstable();
        groups.push(members);
    }
    // Groups in the order they had, from top to bottom and on one height
    // from left to right, so that a tidy graph stays as it is; single nodes
    // come last.
    let corner_of = |members: &Vec<NodeId>| {
        members
            .iter()
            .filter_map(|id| graph.nodes.get(id))
            .map(|node| (node.position[1], node.position[0]))
            .fold((f32::INFINITY, f32::INFINITY), |(top, left), (y, x)| {
                (top.min(y), left.min(x))
            })
    };
    groups.sort_by(|a, b| {
        let (top_a, left_a) = corner_of(a);
        let (top_b, left_b) = corner_of(b);
        (a.len() == 1)
            .cmp(&(b.len() == 1))
            .then(
                top_a
                    .partial_cmp(&top_b)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                left_a
                    .partial_cmp(&left_b)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a[0].cmp(&b[0]))
    });

    // Every group is laid out by itself first, then the groups are put on
    // shelves: side by side while they fit the width of the widest one (or
    // of a large screen), then on the next shelf.
    struct Laid {
        nodes: Vec<(NodeId, [f32; 2])>,
        width: f32,
        height: f32,
    }
    let mut laid: Vec<Laid> = Vec::new();
    for members in &groups {
        if members.len() == 1 {
            let id = members[0];
            laid.push(Laid {
                nodes: vec![(id, [0.0, 0.0])],
                width: NODE_WIDTH,
                height: node_height(graph, id),
            });
            continue;
        }
        let mut by_column: BTreeMap<usize, Vec<NodeId>> = BTreeMap::new();
        for id in members {
            by_column.entry(columns[id]).or_default().push(*id);
        }
        let mut local: HashMap<NodeId, [f32; 2]> = HashMap::new();
        let mut height = 0.0f32;
        let mut width = 0.0f32;
        for (column, nodes) in &mut by_column {
            // Within a column: in the order of the rows of what leads there,
            // so that wires cross little.
            nodes.sort_by(|a, b| {
                let row = |id: &NodeId| {
                    comes_from
                        .get(id)
                        .into_iter()
                        .flatten()
                        .filter_map(|source| local.get(source))
                        .map(|position| position[1])
                        .fold(f32::INFINITY, f32::min)
                };
                row(a)
                    .partial_cmp(&row(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.cmp(b))
            });
            let x = *column as f32 * (NODE_WIDTH + COLUMN_GAP);
            let mut y = 0.0f32;
            for id in nodes.iter() {
                local.insert(*id, [x, y]);
                y += node_height(graph, *id) + ROW_GAP;
            }
            height = height.max(y - ROW_GAP);
            width = width.max(x + NODE_WIDTH);
        }
        let mut nodes: Vec<(NodeId, [f32; 2])> = local.into_iter().collect();
        nodes.sort_by_key(|(id, _)| *id);
        laid.push(Laid {
            nodes,
            width,
            height,
        });
    }

    let shelf_width = laid
        .iter()
        .map(|group| group.width)
        .fold(SHELF_WIDTH, f32::max);
    let mut placed: HashMap<NodeId, [f32; 2]> = HashMap::new();
    let (mut x, mut y, mut shelf_height) = (0.0f32, 0.0f32, 0.0f32);
    for group in &laid {
        if x > 0.0 && x + group.width > shelf_width {
            x = 0.0;
            y += shelf_height + GROUP_GAP;
            shelf_height = 0.0;
        }
        for (id, position) in &group.nodes {
            placed.insert(*id, [x + position[0], y + position[1]]);
        }
        x += group.width + COLUMN_GAP;
        shelf_height = shelf_height.max(group.height);
    }

    let mut moved = 0;
    for (id, position) in placed {
        if let Some(node) = graph.nodes.get_mut(&id) {
            let position = [position[0] + 60.0, position[1] + 60.0];
            if node.position != position {
                node.position = position;
                moved += 1;
            }
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeType;

    fn overlaps(graph: &NodeGraph) -> Vec<(NodeId, NodeId)> {
        let mut ids: Vec<NodeId> = graph.nodes.keys().copied().collect();
        ids.sort_unstable();
        let rect = |id: NodeId| {
            let node = &graph.nodes[&id];
            (
                node.position[0],
                node.position[1],
                NODE_WIDTH,
                node_height(graph, id),
            )
        };
        let mut found = Vec::new();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                let (ax, ay, aw, ah) = rect(*a);
                let (bx, by, bw, bh) = rect(*b);
                if ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah {
                    found.push((*a, *b));
                }
            }
        }
        found
    }

    #[test]
    fn a_tidy_graph_runs_left_to_right_and_no_node_lies_on_another() {
        let mut graph = crate::config::Config::with_default_graph().node_graph;
        // Thrown on a heap first.
        for node in graph.nodes.values_mut() {
            node.position = [0.0, 0.0];
        }
        assert!(tidy(&mut graph) > 0);
        assert_eq!(overlaps(&graph), Vec::new());
        for connection in &graph.connections {
            let from = graph.nodes[&connection.from_node].position[0];
            let to = graph.nodes[&connection.to_node].position[0];
            assert!(
                from < to,
                "{} does not stand left of {}",
                connection.from_node,
                connection.to_node
            );
        }
        // Done a second time, nothing moves.
        assert_eq!(tidy(&mut graph), 0);
    }

    #[test]
    fn putting_in_order_twice_gives_the_same_wherever_the_nodes_were() {
        let mut graph = NodeGraph::new();
        // Two chains; the one made first lies lower than the one made second.
        let a = graph.add_node(NodeType::OnStartup, [40.0, 500.0]);
        let a2 = graph.add_node(NodeType::FocusNextWindow, [300.0, 500.0]);
        let b = graph.add_node(NodeType::OnStartup, [40.0, 100.0]);
        let b2 = graph.add_node(NodeType::FocusPreviousWindow, [300.0, 100.0]);
        graph
            .add_connection_by_name(a, "Event", a2, "Trigger")
            .unwrap();
        graph
            .add_connection_by_name(b, "Event", b2, "Trigger")
            .unwrap();
        tidy(&mut graph);
        let first: Vec<[f32; 2]> = [a, a2, b, b2]
            .iter()
            .map(|id| graph.nodes[id].position)
            .collect();
        assert_eq!(tidy(&mut graph), 0, "a second time moved nodes");
        let second: Vec<[f32; 2]> = [a, a2, b, b2]
            .iter()
            .map(|id| graph.nodes[id].position)
            .collect();
        assert_eq!(first, second);
        // The chain that was higher comes first.
        assert!(
            graph.nodes[&b].position[0] < graph.nodes[&a].position[0]
                || graph.nodes[&b].position[1] < graph.nodes[&a].position[1]
        );
    }

    #[test]
    fn nodes_without_wires_and_loops_are_placed_too() {
        let mut graph = NodeGraph::new();
        let a = graph.add_node(NodeType::OnStartup, [5.0, 5.0]);
        let b = graph.add_node(NodeType::FocusNextWindow, [5.0, 5.0]);
        let c = graph.add_node(NodeType::FocusPreviousWindow, [5.0, 5.0]);
        let _alone = graph.add_node(NodeType::OnStartup, [5.0, 5.0]);
        graph
            .add_connection_by_name(a, "Event", b, "Trigger")
            .unwrap();
        graph
            .add_connection_by_name(b, "Then", c, "Trigger")
            .unwrap();
        // A loop back to the middle.
        let _ = graph.add_connection_by_name(c, "Then", b, "Trigger");
        tidy(&mut graph);
        assert_eq!(overlaps(&graph), Vec::new());
        assert!(graph.nodes[&a].position[0] < graph.nodes[&b].position[0]);
        assert!(tidy(&mut NodeGraph::new()) == 0);
    }
}
