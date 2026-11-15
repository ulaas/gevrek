//! Where things are on the sheet: the view (pan and zoom), the size of a
//! node, the place of a pin, the path of a wire, and what lies under a
//! point. Nothing here draws or knows the toolkit's widgets, so all of it
//! is tested as plain arithmetic.

use egui::{pos2, vec2, Pos2, Rect, Vec2};
use gevrek_model::graph::{Node, NodeGraph, NodeId, PinId};
use std::collections::BTreeSet;

pub const NODE_WIDTH: f32 = 200.0;
pub const HEADER: f32 = 30.0;
pub const PIN_ROW: f32 = 20.0;
/// The room under the last pin.
pub const FOOT: f32 = 10.0;
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 2.5;
/// How many straight pieces a wire is drawn and hit with.
const WIRE_PIECES: usize = 28;

/// One pin of one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PinRef {
    pub node: NodeId,
    pub pin: PinId,
    pub output: bool,
}

/// A wire, by its two ends: from an output to an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WireRef {
    pub from_node: NodeId,
    pub from_pin: PinId,
    pub to_node: NodeId,
    pub to_pin: PinId,
}

/// How the sheet lies in the window: a point of the graph is at
/// `origin + pan + point * zoom` on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The canvas's top left corner on the screen (set every frame).
    pub origin: Pos2,
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            origin: Pos2::ZERO,
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl View {
    pub fn to_screen(self, point: Pos2) -> Pos2 {
        self.origin + self.pan + point.to_vec2() * self.zoom
    }

    pub fn to_graph(self, screen: Pos2) -> Pos2 {
        ((screen - self.origin - self.pan) / self.zoom).to_pos2()
    }

    pub fn rect_to_screen(self, rect: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(rect.min), self.to_screen(rect.max))
    }

    /// Zoom by `factor` so that what is under `screen` stays under it.
    pub fn zoom_at(&mut self, screen: Pos2, factor: f32) {
        let fixed = self.to_graph(screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = screen - self.origin - fixed.to_vec2() * self.zoom;
    }

    /// Move the sheet, without changing its size, so that `rect` (of the
    /// graph) is in a canvas of `size`: to the middle when it is not wholly
    /// in sight, and not at all when it is. Says whether the sheet moved.
    pub fn bring_into_sight(&mut self, rect: Rect, size: Vec2) -> bool {
        const EDGE: f32 = 24.0;
        let min = self.pan + rect.min.to_vec2() * self.zoom;
        let max = self.pan + rect.max.to_vec2() * self.zoom;
        let in_sight =
            min.x >= EDGE && min.y >= EDGE && max.x <= size.x - EDGE && max.y <= size.y - EDGE;
        if in_sight {
            return false;
        }
        self.pan = size / 2.0 - rect.center().to_vec2() * self.zoom;
        true
    }

    /// The view that shows all of `bounds` in a canvas of `size`, no
    /// larger than life.
    pub fn fit(origin: Pos2, bounds: Rect, size: Vec2) -> View {
        let room = size - Vec2::splat(80.0);
        if room.x <= 0.0 || room.y <= 0.0 {
            return View {
                origin,
                ..View::default()
            };
        }
        let zoom = (room.x / bounds.width().max(1.0))
            .min(room.y / bounds.height().max(1.0))
            .clamp(MIN_ZOOM, 1.0);
        View {
            origin,
            pan: size / 2.0 - bounds.center().to_vec2() * zoom,
            zoom,
        }
    }
}

pub fn node_size(node: &Node) -> Vec2 {
    let rows = node.inputs.len().max(node.outputs.len()) as f32;
    vec2(NODE_WIDTH, HEADER + rows * PIN_ROW + FOOT)
}

pub fn node_rect(node: &Node) -> Rect {
    Rect::from_min_size(pos2(node.position[0], node.position[1]), node_size(node))
}

/// The centre of the pin at `index` of a node's inputs or outputs: inputs
/// on the left edge, outputs on the right.
pub fn pin_pos(node: &Node, index: usize, output: bool) -> Pos2 {
    pos2(
        node.position[0] + if output { NODE_WIDTH } else { 0.0 },
        node.position[1] + HEADER + index as f32 * PIN_ROW + PIN_ROW / 2.0,
    )
}

pub fn pin_place(graph: &NodeGraph, pin: PinRef) -> Option<Pos2> {
    let node = graph.nodes.get(&pin.node)?;
    let pins = if pin.output {
        &node.outputs
    } else {
        &node.inputs
    };
    let index = pins.iter().position(|other| other.id == pin.pin)?;
    Some(pin_pos(node, index, pin.output))
}

/// The nodes from the bottom of the pile to the top: by id, the selected
/// ones above the others (they are the ones being moved).
pub fn pile(graph: &NodeGraph, selected: &BTreeSet<NodeId>) -> Vec<NodeId> {
    let mut ids: Vec<NodeId> = graph.nodes.keys().copied().collect();
    ids.sort_by_key(|id| (selected.contains(id), *id));
    ids
}

/// The topmost node under a point.
pub fn node_at(graph: &NodeGraph, pile: &[NodeId], point: Pos2) -> Option<NodeId> {
    pile.iter().rev().copied().find(|id| {
        graph
            .nodes
            .get(id)
            .is_some_and(|node| node_rect(node).contains(point))
    })
}

/// The nearest pin within `reach` of a point: a wire need not be dropped
/// exactly on the dot.
pub fn pin_at(graph: &NodeGraph, point: Pos2, reach: f32) -> Option<PinRef> {
    let mut best: Option<(f32, PinRef)> = None;
    for node in graph.nodes.values() {
        for (output, pins) in [(false, &node.inputs), (true, &node.outputs)] {
            for (index, pin) in pins.iter().enumerate() {
                let distance = pin_pos(node, index, output).distance(point);
                if distance <= reach && best.is_none_or(|(nearest, _)| distance < nearest) {
                    best = Some((
                        distance,
                        PinRef {
                            node: node.id,
                            pin: pin.id,
                            output,
                        },
                    ));
                }
            }
        }
    }
    best.map(|(_, pin)| pin)
}

/// The path of a wire that leaves `from` to the right and arrives at `to`
/// from the left, as points to join with straight lines.
pub fn wire_path(from: Pos2, to: Pos2) -> Vec<Pos2> {
    wire_path_sided(from, to, 1.0)
}

/// A wire being pulled: from an output it leaves to the right (`side` 1),
/// from an input to the left (`side` -1).
pub fn wire_path_sided(from: Pos2, to: Pos2, side: f32) -> Vec<Pos2> {
    let reach = ((to.x - from.x).abs() * 0.5).max(50.0) * side;
    let (first, second) = (pos2(from.x + reach, from.y), pos2(to.x - reach, to.y));
    (0..=WIRE_PIECES)
        .map(|step| {
            let t = step as f32 / WIRE_PIECES as f32;
            let s = 1.0 - t;
            let (a, b, c, d) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
            pos2(
                a * from.x + b * first.x + c * second.x + d * to.x,
                a * from.y + b * first.y + c * second.y + d * to.y,
            )
        })
        .collect()
}

/// How far a point is from the piece of line between two points.
pub fn distance_to_piece(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let along = end - start;
    let length = along.length_sq();
    if length < 0.0001 {
        return point.distance(start);
    }
    let t = ((point - start).dot(along) / length).clamp(0.0, 1.0);
    point.distance(start + along * t)
}

pub fn wire_ends(graph: &NodeGraph, wire: WireRef) -> Option<(Pos2, Pos2)> {
    Some((
        pin_place(
            graph,
            PinRef {
                node: wire.from_node,
                pin: wire.from_pin,
                output: true,
            },
        )?,
        pin_place(
            graph,
            PinRef {
                node: wire.to_node,
                pin: wire.to_pin,
                output: false,
            },
        )?,
    ))
}

/// The wire that passes within `reach` of a point (the last one drawn
/// first: it lies on top).
pub fn wire_at(graph: &NodeGraph, point: Pos2, reach: f32) -> Option<WireRef> {
    graph.connections.iter().rev().find_map(|connection| {
        let wire = WireRef {
            from_node: connection.from_node,
            from_pin: connection.from_pin,
            to_node: connection.to_node,
            to_pin: connection.to_pin,
        };
        let (from, to) = wire_ends(graph, wire)?;
        // Most wires are nowhere near the point.
        if !Rect::from_two_pos(from, to)
            .expand(60.0 + reach)
            .contains(point)
        {
            return None;
        }
        let path = wire_path(from, to);
        path.windows(2)
            .any(|piece| distance_to_piece(point, piece[0], piece[1]) <= reach)
            .then_some(wire)
    })
}

/// The nodes a rubber band touches.
pub fn nodes_in(graph: &NodeGraph, band: Rect) -> BTreeSet<NodeId> {
    graph
        .nodes
        .values()
        .filter(|node| node_rect(node).intersects(band))
        .map(|node| node.id)
        .collect()
}

/// The rectangle around these nodes, or around all when `only` is `None`.
pub fn bounds(graph: &NodeGraph, only: Option<&BTreeSet<NodeId>>) -> Option<Rect> {
    graph
        .nodes
        .values()
        .filter(|node| only.is_none_or(|only| only.contains(&node.id)))
        .map(node_rect)
        .reduce(|all, rect| all.union(rect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::graph::NodeType;

    #[test]
    fn a_node_out_of_sight_is_brought_to_the_middle_and_one_in_sight_is_left() {
        let size = vec2(800.0, 600.0);
        let mut view = View {
            origin: pos2(300.0, 100.0),
            pan: vec2(40.0, 40.0),
            zoom: 0.5,
        };
        // In sight: nothing moves.
        let near = Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 120.0));
        assert!(!view.bring_into_sight(near, size));
        assert_eq!(view.pan, vec2(40.0, 40.0));
        // Far off to the right and below: its middle comes to the canvas's.
        let far = Rect::from_min_size(pos2(4000.0, 3000.0), vec2(200.0, 120.0));
        assert!(view.bring_into_sight(far, size));
        let middle = view.to_screen(far.center()) - view.origin;
        assert!((middle.x - 400.0).abs() < 0.01 && (middle.y - 300.0).abs() < 0.01);
        assert_eq!(view.zoom, 0.5, "the size of things stays");
        // Now it is in sight.
        assert!(!view.bring_into_sight(far, size));
        // Half over the edge counts as out of sight.
        let mut at_the_edge = View {
            pan: vec2(-150.0, 40.0),
            ..view
        };
        assert!(at_the_edge.bring_into_sight(far, size));
    }

    fn close(a: Pos2, b: Pos2) -> bool {
        a.distance(b) < 0.01
    }

    #[test]
    fn the_screen_and_the_graph_are_each_others_inverse() {
        let view = View {
            origin: pos2(240.0, 90.0),
            pan: vec2(-35.0, 12.0),
            zoom: 0.6,
        };
        for point in [pos2(0.0, 0.0), pos2(410.0, -80.0), pos2(-3.5, 999.0)] {
            assert!(close(view.to_graph(view.to_screen(point)), point));
        }
        assert!(close(
            view.to_screen(pos2(100.0, 100.0)),
            pos2(240.0 - 35.0 + 60.0, 90.0 + 12.0 + 60.0)
        ));
    }

    #[test]
    fn zooming_keeps_what_is_under_the_pointer_under_it() {
        let mut view = View {
            origin: pos2(240.0, 90.0),
            pan: vec2(40.0, 25.0),
            zoom: 1.0,
        };
        let pointer = pos2(700.0, 400.0);
        let under = view.to_graph(pointer);
        for factor in [1.3, 1.3, 0.5, 0.9, 2.0] {
            view.zoom_at(pointer, factor);
            assert!(
                close(view.to_screen(under), pointer),
                "after {factor}: {:?}",
                view.to_screen(under)
            );
        }
        // Also at the ends of the range, where the factor is cut short.
        for _ in 0..40 {
            view.zoom_at(pointer, 1.5);
        }
        assert_eq!(view.zoom, MAX_ZOOM);
        assert!(close(view.to_screen(under), pointer));
        for _ in 0..40 {
            view.zoom_at(pointer, 0.5);
        }
        assert_eq!(view.zoom, MIN_ZOOM);
        assert!(close(view.to_screen(under), pointer));
    }

    #[test]
    fn a_fitted_graph_is_in_the_middle_and_all_of_it_is_seen() {
        let bounds = Rect::from_min_max(pos2(-500.0, 100.0), pos2(2500.0, 900.0));
        let size = vec2(1000.0, 700.0);
        let view = View::fit(pos2(10.0, 20.0), bounds, size);
        let canvas = Rect::from_min_size(pos2(10.0, 20.0), size);
        assert!(canvas.contains_rect(view.rect_to_screen(bounds)));
        assert!(close(view.to_screen(bounds.center()), canvas.center()));
        // A small graph is not blown up.
        let small = View::fit(
            Pos2::ZERO,
            Rect::from_min_size(Pos2::ZERO, vec2(200.0, 60.0)),
            size,
        );
        assert_eq!(small.zoom, 1.0);
    }

    fn two_nodes() -> (NodeGraph, NodeId, NodeId) {
        let mut graph = NodeGraph::new();
        let start = graph.add_node(NodeType::OnStartup, [0.0, 0.0]);
        let log = graph.add_node(
            NodeType::LogMessage {
                message: "hello".to_string(),
                level: gevrek_model::graph::LogLevel::Info,
            },
            [400.0, 100.0],
        );
        (graph, start, log)
    }

    #[test]
    fn pins_are_on_the_edges_and_found_near_their_dot() {
        let (graph, start, log) = two_nodes();
        let out = graph.nodes[&start].outputs[0].id;
        let place = pin_place(
            &graph,
            PinRef {
                node: start,
                pin: out,
                output: true,
            },
        )
        .expect("the pin");
        assert!(close(place, pos2(NODE_WIDTH, HEADER + PIN_ROW / 2.0)));
        let found = pin_at(&graph, place + vec2(7.0, -5.0), 12.0).expect("near enough");
        assert_eq!(
            found,
            PinRef {
                node: start,
                pin: out,
                output: true
            }
        );
        assert_eq!(pin_at(&graph, place + vec2(30.0, 0.0), 12.0), None);
        // The first input of the other node is on its left edge.
        let input = graph.nodes[&log].inputs[0].id;
        let found =
            pin_at(&graph, pos2(401.0, 100.0 + HEADER + PIN_ROW / 2.0), 12.0).expect("the input");
        assert_eq!(
            found,
            PinRef {
                node: log,
                pin: input,
                output: false
            }
        );
    }

    #[test]
    fn the_topmost_node_is_the_one_hit() {
        let (mut graph, start, log) = two_nodes();
        // One over the other.
        graph.nodes.get_mut(&log).expect("node").position = [50.0, 10.0];
        let point = pos2(100.0, 20.0);
        let none = BTreeSet::new();
        assert_eq!(
            node_at(&graph, &pile(&graph, &none), point),
            Some(log),
            "the later node lies on top"
        );
        let chosen = BTreeSet::from([start]);
        assert_eq!(
            node_at(&graph, &pile(&graph, &chosen), point),
            Some(start),
            "a selected node lies above"
        );
        assert_eq!(
            node_at(&graph, &pile(&graph, &none), pos2(-5.0, 20.0)),
            None
        );
    }

    #[test]
    fn a_wire_is_hit_along_its_path_and_not_beside_it() {
        let (mut graph, start, log) = two_nodes();
        let (out, input) = (
            graph.nodes[&start].outputs[0].name.clone(),
            graph.nodes[&log].inputs[0].name.clone(),
        );
        graph
            .add_connection_by_name(start, &out, log, &input)
            .expect("a wire");
        let wire = wire_at(&graph, pos2(300.0, 92.0), 400.0).expect("the only wire");
        let (from, to) = wire_ends(&graph, wire).expect("ends");
        let path = wire_path(from, to);
        assert!(close(path[0], from) && close(*path.last().expect("points"), to));
        let middle = path[path.len() / 2];
        assert_eq!(wire_at(&graph, middle + vec2(0.0, 4.0), 7.0), Some(wire));
        assert_eq!(wire_at(&graph, middle + vec2(0.0, 40.0), 7.0), None);
        assert!(
            (distance_to_piece(pos2(5.0, 5.0), pos2(0.0, 0.0), pos2(10.0, 0.0)) - 5.0).abs()
                < 0.001
        );
        assert!(
            (distance_to_piece(pos2(13.0, 4.0), pos2(0.0, 0.0), pos2(10.0, 0.0)) - 5.0).abs()
                < 0.001
        );
    }

    #[test]
    fn a_rubber_band_takes_what_it_touches() {
        let (graph, start, log) = two_nodes();
        let band = Rect::from_two_pos(pos2(150.0, 20.0), pos2(-40.0, -40.0));
        assert_eq!(nodes_in(&graph, band), BTreeSet::from([start]));
        let wide = Rect::from_two_pos(pos2(150.0, 20.0), pos2(450.0, 120.0));
        assert_eq!(nodes_in(&graph, wide), BTreeSet::from([start, log]));
        assert!(nodes_in(
            &graph,
            Rect::from_two_pos(pos2(250.0, 0.0), pos2(350.0, 50.0))
        )
        .is_empty());
        let all = bounds(&graph, None).expect("two nodes");
        assert!(
            all.contains_rect(node_rect(&graph.nodes[&start]))
                && all.contains_rect(node_rect(&graph.nodes[&log]))
        );
        assert_eq!(
            bounds(&graph, Some(&BTreeSet::from([log]))),
            Some(node_rect(&graph.nodes[&log]))
        );
    }
}
