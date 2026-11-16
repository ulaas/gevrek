use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub use crate::settings::ScratchpadPosition;

pub type NodeId = usize;
pub type PinId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

/// A log line shown in the editor and emitted by the runtime.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: std::time::Instant,
    pub level: LogLevel,
    pub message: String,
}

impl LogLevel {
    pub fn as_str(&self) -> &str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
        }
    }

    pub fn color(&self) -> [u8; 3] {
        match self {
            LogLevel::Debug => [150, 150, 150],   // Gray
            LogLevel::Info => [100, 150, 255],    // Blue
            LogLevel::Warning => [255, 200, 100], // Orange
            LogLevel::Error => [255, 100, 100],   // Red
        }
    }
}

/// Definition of a window matching rule for dynamic rule creation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RuleDefinition {
    pub pattern: String,
    pub match_kind: crate::rules::MatchKind,
    pub match_strategy: crate::rules::MatchStrategy,
}

impl RuleDefinition {
    pub fn new(
        pattern: String,
        match_kind: crate::rules::MatchKind,
        match_strategy: crate::rules::MatchStrategy,
    ) -> Self {
        Self {
            pattern,
            match_kind,
            match_strategy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NodeGraph {
    /// Written in the order of the ids, so the same graph is always the
    /// same text (a file that does not churn, and the same bytes on every
    /// system).
    #[serde(serialize_with = "crate::ordered_map")]
    // In the file the ids are the keys of an object. The schema says so
    // plainly (an object of nodes), which is what lets a form find a
    // node, and its settings, by the node's place in the file.
    #[schemars(with = "HashMap<String, Node>")]
    pub nodes: HashMap<NodeId, Node>,
    pub connections: Vec<Connection>,
    #[serde(default)]
    next_node_id: NodeId,
    #[serde(default)]
    next_pin_id: PinId,
    /// O(1) lookup set for checking if a connection exists
    /// Key format: (from_node, from_pin, to_node, to_pin)
    #[serde(skip)]
    connection_set: HashSet<(NodeId, PinId, NodeId, PinId)>,
    /// O(1) lookup for connections TO an input pin (for checking if input is connected)
    /// Maps (to_node, to_pin) -> exists
    #[serde(skip)]
    input_connection_set: HashSet<(NodeId, PinId)>,
}

/// Something wrong in a graph (`NodeGraph::problems`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphProblem {
    /// The node to look at, when one of the wire's ends is there.
    pub node: Option<NodeId>,
    pub text: String,
}

fn problem_at(node: Option<NodeId>, text: String) -> GraphProblem {
    GraphProblem { node, text }
}

impl NodeGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            connections: Vec::new(),
            next_node_id: 0,
            next_pin_id: 0,
            connection_set: HashSet::new(),
            input_connection_set: HashSet::new(),
        }
    }

    /// Create a NodeGraph from parts (used for proto conversion)
    /// The next node and pin IDs this graph will allocate.
    pub fn id_counters(&self) -> (NodeId, PinId) {
        (self.next_node_id, self.next_pin_id)
    }

    pub fn from_parts(
        nodes: HashMap<NodeId, Node>,
        connections: Vec<Connection>,
        next_node_id: NodeId,
        next_pin_id: PinId,
    ) -> Self {
        // Build connection sets from connections vec
        let mut connection_set = HashSet::new();
        let mut input_connection_set = HashSet::new();
        for conn in &connections {
            connection_set.insert((conn.from_node, conn.from_pin, conn.to_node, conn.to_pin));
            input_connection_set.insert((conn.to_node, conn.to_pin));
        }

        Self {
            nodes,
            connections,
            next_node_id,
            next_pin_id,
            connection_set,
            input_connection_set,
        }
    }

    /// Rebuild connection indices after deserialization or bulk operations
    pub fn rebuild_connection_indices(&mut self) {
        // What a file says of the numbers is not taken on trust: a node
        // is known by the number it is filed under, and the next number
        // to give out lies above every one in use. (A file put together
        // by hand, with the counter left at nought, had its first node
        // written over by the next one added.)
        let mut highest_pin = 0;
        for (key, node) in &mut self.nodes {
            node.id = *key;
            for pin in node.inputs.iter().chain(node.outputs.iter()) {
                highest_pin = highest_pin.max(pin.id + 1);
            }
        }
        if let Some(highest) = self.nodes.keys().max() {
            self.next_node_id = self.next_node_id.max(highest + 1);
        }
        self.next_pin_id = self.next_pin_id.max(highest_pin);
        self.connection_set.clear();
        self.input_connection_set.clear();
        for conn in &self.connections {
            self.connection_set
                .insert((conn.from_node, conn.from_pin, conn.to_node, conn.to_pin));
            self.input_connection_set
                .insert((conn.to_node, conn.to_pin));
        }
    }

    pub fn add_node(&mut self, node_type: NodeType, position: [f32; 2]) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;

        let inputs = self.create_inputs_for_type(&node_type);
        let outputs = self.create_outputs_for_type(&node_type);

        let node = Node {
            id,
            node_type,
            position,
            inputs,
            outputs,
        };

        self.nodes.insert(id, node);
        id
    }

    pub fn remove_node(&mut self, id: NodeId) {
        self.nodes.remove(&id);
        self.connections
            .retain(|c| c.from_node != id && c.to_node != id);
        self.rebuild_connection_indices();
    }

    /// Refresh all nodes to match current pin definitions
    /// This ensures loaded configs work with updated node types
    pub fn refresh_node_pins(&mut self) {
        log::info!("Refreshing node pins to match current definitions...");

        // Build mapping of old pin IDs to their indices BEFORE regenerating
        // This allows us to restore connections by position
        let mut pin_id_to_index: HashMap<(NodeId, PinId, bool), usize> = HashMap::new(); // (node_id, pin_id, is_output) -> index
                                                                                         // The old names too: when a definition gained or reordered pins, a
                                                                                         // connection belongs to the pin with the same name, not to whatever
                                                                                         // now sits at the old position.
        let mut pin_id_to_name: HashMap<(NodeId, PinId, bool), String> = HashMap::new();

        for (node_id, node) in &self.nodes {
            for (idx, input) in node.inputs.iter().enumerate() {
                pin_id_to_index.insert((*node_id, input.id, false), idx);
                pin_id_to_name.insert((*node_id, input.id, false), input.name.clone());
            }
            for (idx, output) in node.outputs.iter().enumerate() {
                pin_id_to_index.insert((*node_id, output.id, true), idx);
                pin_id_to_name.insert((*node_id, output.id, true), output.name.clone());
            }
        }

        // Store old connections before regenerating pins
        let old_connections = self.connections.clone();
        self.connections.clear();
        // CRITICAL: Also clear the connection index sets, they will be rebuilt by add_connection()
        self.connection_set.clear();
        self.input_connection_set.clear();

        // Collect node info before regenerating (to avoid borrow conflicts)
        let mut node_info: Vec<(NodeId, NodeType, usize, usize)> = self
            .nodes
            .iter()
            .map(|(id, node)| {
                (
                    *id,
                    node.node_type.clone(),
                    node.inputs.len(),
                    node.outputs.len(),
                )
            })
            .collect();
        // In id order, so the new pin ids do not depend on the map's order:
        // two processes refreshing the same file get the same graph.
        node_info.sort_by_key(|(id, ..)| *id);

        // Regenerate pins for each node based on current NodeType definition
        for (node_id, node_type, old_input_count, old_output_count) in node_info {
            // A node this version does not know keeps the pins it was read
            // with: there is no definition to refresh them from.
            if matches!(node_type, NodeType::Unknown(_)) {
                continue;
            }
            // Create new pins based on current type definition
            let pin_counter = self.next_pin_id;
            let new_inputs = self.create_inputs_for_type(&node_type);
            let new_outputs = self.create_outputs_for_type(&node_type);

            // A node whose pins already match the definition keeps them, ids
            // included: refreshing a current graph must not change it (pin
            // ids are its identity for the editor and across the wire).
            let same_pins = |old: &[Pin], new: &[Pin]| {
                old.len() == new.len()
                    && old
                        .iter()
                        .zip(new)
                        .all(|(old, new)| old.name == new.name && old.pin_type == new.pin_type)
            };
            if self.nodes.get(&node_id).is_some_and(|node| {
                same_pins(&node.inputs, &new_inputs) && same_pins(&node.outputs, &new_outputs)
            }) {
                self.next_pin_id = pin_counter;
                continue;
            }

            // Update the node's pins
            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.inputs = new_inputs;
                node.outputs = new_outputs;

                log::debug!(
                    "Node {} ({}): refreshed pins (inputs: {} -> {}, outputs: {} -> {})",
                    node_id,
                    node_type.display_name(),
                    old_input_count,
                    node.inputs.len(),
                    old_output_count,
                    node.outputs.len()
                );
            }
        }

        // Restore connections that are still valid
        let mut restored = 0;
        let mut invalid = 0;

        for conn in old_connections {
            // Find old pin indices using the mapping we built earlier
            let from_idx = pin_id_to_index.get(&(conn.from_node, conn.from_pin, true));
            let to_idx = pin_id_to_index.get(&(conn.to_node, conn.to_pin, false));

            if let (Some(&from_idx), Some(&to_idx)) = (from_idx, to_idx) {
                // Get pin information and validate (without holding references)
                let pin_info = {
                    let from_node = self.nodes.get(&conn.from_node);
                    let to_node = self.nodes.get(&conn.to_node);

                    match (from_node, to_node) {
                        (Some(from_node), Some(to_node)) => {
                            // The pin with the old name; at the old
                            // position only if no pin has that name.
                            let by_name = |pins: &[Pin], key: (NodeId, PinId, bool)| {
                                pin_id_to_name
                                    .get(&key)
                                    .and_then(|name| pins.iter().position(|pin| pin.name == *name))
                            };
                            let from_idx =
                                by_name(&from_node.outputs, (conn.from_node, conn.from_pin, true))
                                    .unwrap_or(from_idx);
                            let to_idx =
                                by_name(&to_node.inputs, (conn.to_node, conn.to_pin, false))
                                    .unwrap_or(to_idx);
                            match (from_node.outputs.get(from_idx), to_node.inputs.get(to_idx)) {
                                (Some(new_from_pin), Some(new_to_pin)) => {
                                    // Check type compatibility
                                    if pin_takes(
                                        &to_node.node_type,
                                        &new_to_pin.pin_type,
                                        &new_from_pin.pin_type,
                                    ) {
                                        Some((
                                            new_from_pin.id,
                                            new_to_pin.id,
                                            from_node.node_type.display_name().to_string(),
                                            new_from_pin.name.clone(),
                                            to_node.node_type.display_name().to_string(),
                                            new_to_pin.name.clone(),
                                        ))
                                    } else {
                                        log::debug!(
                                            "Connection invalid (type mismatch): {:?} -> {:?}",
                                            new_from_pin.pin_type,
                                            new_to_pin.pin_type
                                        );
                                        None
                                    }
                                }
                                _ => {
                                    log::debug!("Connection invalid (pin index out of bounds): from_idx={}, to_idx={}",
                                        from_idx, to_idx);
                                    None
                                }
                            }
                        }
                        _ => {
                            log::debug!(
                                "Connection invalid (node not found): from_node={}, to_node={}",
                                conn.from_node,
                                conn.to_node
                            );
                            None
                        }
                    }
                }; // References dropped here

                // Now add the connection without holding any references
                if let Some((
                    new_from_pin_id,
                    new_to_pin_id,
                    from_name,
                    from_pin_name,
                    to_name,
                    to_pin_name,
                )) = pin_info
                {
                    let new_conn = Connection {
                        from_node: conn.from_node,
                        from_pin: new_from_pin_id,
                        to_node: conn.to_node,
                        to_pin: new_to_pin_id,
                    };

                    if self.add_connection(new_conn) {
                        restored += 1;
                        log::debug!(
                            "Restored connection: {} [{}] -> {} [{}]",
                            from_name,
                            from_pin_name,
                            to_name,
                            to_pin_name
                        );
                    } else {
                        invalid += 1;
                        log::debug!(
                            "Failed to restore connection (validation failed): {} -> {}",
                            conn.from_node,
                            conn.to_node
                        );
                    }
                } else {
                    invalid += 1;
                }
            } else {
                invalid += 1;
                log::debug!("Connection invalid (pin ID not found in old mapping): from_pin={:?}, to_pin={:?}",
                    from_idx.is_some(), to_idx.is_some());
            }
        }

        log::info!(
            "Pin refresh complete: {} connections restored, {} invalid/removed",
            restored,
            invalid
        );
    }

    pub fn add_connection(&mut self, connection: Connection) -> bool {
        // O(1) duplicate check using HashSet
        let conn_key = (
            connection.from_node,
            connection.from_pin,
            connection.to_node,
            connection.to_pin,
        );
        if self.connection_set.contains(&conn_key) {
            log::warn!(
                "Duplicate connection: node {} pin {} -> node {} pin {}",
                connection.from_node,
                connection.from_pin,
                connection.to_node,
                connection.to_pin
            );
            return false;
        }

        // Validate connection types - find pins by ID, not index!
        if let (Some(from_node), Some(to_node)) = (
            self.nodes.get(&connection.from_node),
            self.nodes.get(&connection.to_node),
        ) {
            // Find output pin by ID
            let from_pin = from_node
                .outputs
                .iter()
                .find(|p| p.id == connection.from_pin);
            // Find input pin by ID
            let to_pin = to_node.inputs.iter().find(|p| p.id == connection.to_pin);

            if let (Some(from_pin), Some(to_pin)) = (from_pin, to_pin) {
                log::trace!(
                    "Found pins: {} ({:?}) -> {} ({:?})",
                    from_pin.name,
                    from_pin.pin_type,
                    to_pin.name,
                    to_pin.pin_type
                );

                if pin_takes(&to_node.node_type, &to_pin.pin_type, &from_pin.pin_type) {
                    // O(1) check if target input pin already has a connection
                    // Execution pins can have multiple inputs, data pins cannot
                    let already_connected = self
                        .input_connection_set
                        .contains(&(connection.to_node, connection.to_pin));

                    if already_connected {
                        // Allow multiple connections for execution pins
                        if to_pin.pin_type == PinType::Execution {
                            self.connection_set.insert(conn_key);
                            // Note: input_connection_set already has this entry
                            self.connections.push(connection);
                            log::info!("Added execution connection (multiple inputs allowed)");
                            return true;
                        } else {
                            // For data pins, only allow one input
                            log::warn!("Data pin already connected: node {} pin {} - replacing old connection",
                                connection.to_node, connection.to_pin);
                            // Remove old connection to this input (this also updates the sets)
                            self.remove_all_connections_to_input(
                                connection.to_node,
                                connection.to_pin,
                            );
                            self.connection_set.insert(conn_key);
                            self.input_connection_set
                                .insert((connection.to_node, connection.to_pin));
                            self.connections.push(connection);
                            log::info!(
                                "Replaced data connection (data pins accept only one input)"
                            );
                            return true;
                        }
                    } else {
                        // No existing connection, add it
                        self.connection_set.insert(conn_key);
                        self.input_connection_set
                            .insert((connection.to_node, connection.to_pin));
                        self.connections.push(connection);
                        log::trace!("Connection added successfully!");
                        return true;
                    }
                } else {
                    // Types are incompatible - check if we can auto-insert a conversion node
                    let needs_to_string =
                        matches!(from_pin.pin_type, PinType::Number | PinType::Bool)
                            && to_pin.pin_type == PinType::String;
                    let needs_from_string =
                        from_pin.pin_type == PinType::String && to_pin.pin_type == PinType::Number;

                    if needs_to_string || needs_from_string {
                        log::info!(
                            "Auto-inserting conversion node: {:?} -> {:?}",
                            from_pin.pin_type,
                            to_pin.pin_type
                        );

                        // Create the conversion node
                        let converter_type = if needs_to_string {
                            NodeType::ToString
                        } else {
                            NodeType::FromString
                        };

                        // Position the converter node between the two nodes
                        let from_node_pos = from_node.position;
                        let to_node_pos = to_node.position;
                        let converter_pos = [
                            (from_node_pos[0] + to_node_pos[0]) / 2.0,
                            (from_node_pos[1] + to_node_pos[1]) / 2.0,
                        ];

                        // Create the converter node
                        let converter_id = self.next_node_id;
                        self.next_node_id += 1;

                        let converter_inputs = self.create_inputs_for_type(&converter_type);
                        let converter_outputs = self.create_outputs_for_type(&converter_type);

                        // Get the pin IDs before creating the node
                        let converter_input_pin = converter_inputs[0].id;
                        let converter_output_pin = converter_outputs[0].id;

                        let converter_node = Node {
                            id: converter_id,
                            node_type: converter_type,
                            position: converter_pos,
                            inputs: converter_inputs,
                            outputs: converter_outputs,
                        };

                        // Insert the converter node
                        self.nodes.insert(converter_id, converter_node);

                        // Create two connections:
                        // 1. from_pin -> converter input
                        let conn1 = Connection {
                            from_node: connection.from_node,
                            from_pin: connection.from_pin,
                            to_node: converter_id,
                            to_pin: converter_input_pin,
                        };

                        // 2. converter output -> to_pin
                        let conn2 = Connection {
                            from_node: converter_id,
                            from_pin: converter_output_pin,
                            to_node: connection.to_node,
                            to_pin: connection.to_pin,
                        };

                        // Remove any existing connection to the target pin
                        self.remove_all_connections_to_input(connection.to_node, connection.to_pin);

                        // Add both connections through the normal path so the
                        // connection indexes remain consistent.
                        let first_added = self.add_connection(conn1);
                        let second_added = self.add_connection(conn2);
                        if !first_added || !second_added {
                            self.remove_node(converter_id);
                            log::error!("Failed to connect auto-inserted conversion node");
                            return false;
                        }

                        log::info!(
                            "Auto-inserted {} node and created connections",
                            if needs_to_string {
                                "ToString"
                            } else {
                                "FromString"
                            }
                        );
                        return true;
                    } else {
                        log::warn!(
                            "Pin type mismatch: {:?} vs {:?}",
                            from_pin.pin_type,
                            to_pin.pin_type
                        );
                    }
                }
            } else {
                log::error!(
                    "Pin not found: from_pin={:?}, to_pin={:?}",
                    from_pin.is_some(),
                    to_pin.is_some()
                );
            }
        } else {
            log::error!(
                "Node not found: from_node={}, to_node={}",
                connection.from_node,
                connection.to_node
            );
        }

        false
    }

    pub fn remove_connection(
        &mut self,
        from_node: NodeId,
        from_pin: PinId,
        to_node: NodeId,
        to_pin: PinId,
    ) {
        // Remove from HashSet indices
        self.connection_set
            .remove(&(from_node, from_pin, to_node, to_pin));
        // Check if this was the last connection to this input
        let other_connections_to_input = self.connections.iter().any(|c| {
            c.to_node == to_node
                && c.to_pin == to_pin
                && !(c.from_node == from_node && c.from_pin == from_pin)
        });
        if !other_connections_to_input {
            self.input_connection_set.remove(&(to_node, to_pin));
        }
        self.connections.retain(|c| {
            !(c.from_node == from_node
                && c.from_pin == from_pin
                && c.to_node == to_node
                && c.to_pin == to_pin)
        });
    }

    /// Remove all connections from a specific output pin
    pub fn remove_all_connections_from_output(&mut self, from_node: NodeId, from_pin: PinId) {
        // Update indices before removing
        for conn in self
            .connections
            .iter()
            .filter(|c| c.from_node == from_node && c.from_pin == from_pin)
        {
            self.connection_set
                .remove(&(conn.from_node, conn.from_pin, conn.to_node, conn.to_pin));
        }
        self.connections
            .retain(|c| !(c.from_node == from_node && c.from_pin == from_pin));
        // Rebuild input_connection_set after bulk removal
        self.input_connection_set.clear();
        for conn in &self.connections {
            self.input_connection_set
                .insert((conn.to_node, conn.to_pin));
        }
    }

    /// Remove all connections to a specific input pin
    pub fn remove_all_connections_to_input(&mut self, to_node: NodeId, to_pin: PinId) {
        // Update indices before removing
        for conn in self
            .connections
            .iter()
            .filter(|c| c.to_node == to_node && c.to_pin == to_pin)
        {
            self.connection_set
                .remove(&(conn.from_node, conn.from_pin, conn.to_node, conn.to_pin));
        }
        self.input_connection_set.remove(&(to_node, to_pin));
        self.connections
            .retain(|c| !(c.to_node == to_node && c.to_pin == to_pin));
    }

    /// Check if an input pin is connected and return the source node/pin information
    pub fn get_input_connection(
        &self,
        to_node: NodeId,
        to_pin: PinId,
    ) -> Option<(NodeId, PinId, String)> {
        self.connections
            .iter()
            .find(|c| c.to_node == to_node && c.to_pin == to_pin)
            .and_then(|conn| {
                // Get the source node to retrieve its display name
                self.nodes.get(&conn.from_node).map(|node| {
                    (
                        conn.from_node,
                        conn.from_pin,
                        node.node_type.display_name().to_string(),
                    )
                })
            })
    }

    /// Check if an input pin by name is connected
    pub fn is_input_pin_connected(
        &self,
        node_id: NodeId,
        pin_name: &str,
    ) -> Option<(NodeId, PinId, String)> {
        if let Some(node) = self.nodes.get(&node_id) {
            // Find the pin by name
            if let Some(pin) = node.inputs.iter().find(|p| p.name == pin_name) {
                return self.get_input_connection(node_id, pin.id);
            }
        }
        None
    }

    /// What is wrong with the graph as it stands, without changing it: the
    /// wires `validate_and_clean` would take out when the window manager
    /// loads it (an end that is not there, two ends that do not fit). Each
    /// with the node to look at.
    pub fn problems(&self) -> Vec<GraphProblem> {
        let name = |node: &Node| node.node_type.display_name().to_string();
        let mut problems = Vec::new();
        for wire in &self.connections {
            let (from, to) = (
                self.nodes.get(&wire.from_node),
                self.nodes.get(&wire.to_node),
            );
            let found = match (from, to) {
                (None, None) => problem_at(
                    None,
                    "A wire joins two nodes that are not there".to_string(),
                ),
                (None, Some(to)) => problem_at(
                    Some(wire.to_node),
                    format!(
                        "A wire into {} comes from a node that is not there",
                        name(to)
                    ),
                ),
                (Some(from), None) => problem_at(
                    Some(wire.from_node),
                    format!(
                        "A wire out of {} goes to a node that is not there",
                        name(from)
                    ),
                ),
                (Some(from), Some(to)) => {
                    let out = from.outputs.iter().find(|pin| pin.id == wire.from_pin);
                    let into = to.inputs.iter().find(|pin| pin.id == wire.to_pin);
                    match (out, into) {
                        (None, _) => problem_at(
                            Some(wire.from_node),
                            format!(
                                "A wire to {} starts at an output {} does not have",
                                name(to),
                                name(from)
                            ),
                        ),
                        (_, None) => problem_at(
                            Some(wire.to_node),
                            format!(
                                "A wire from {} ends at an input {} does not have",
                                name(from),
                                name(to)
                            ),
                        ),
                        (Some(out), Some(into))
                            if !pin_takes(&to.node_type, &into.pin_type, &out.pin_type) =>
                        {
                            problem_at(
                                Some(wire.to_node),
                                format!(
                                    "The wire from {} ({}) to {} ({}) joins two ends that do not fit",
                                    name(from),
                                    out.name,
                                    name(to),
                                    into.name
                                ),
                            )
                        }
                        _ => continue,
                    }
                }
            };
            problems.push(found);
        }
        problems
    }

    /// Validate and clean the graph after loading from config
    /// Removes invalid connections where nodes or pins don't exist
    /// Returns (removed_nodes_count, removed_connections_count)
    pub fn validate_and_clean(&mut self) -> (usize, usize) {
        let original_node_count = self.nodes.len();
        let original_connection_count = self.connections.len();

        // Remove invalid connections (where nodes or pins don't exist)
        self.connections.retain(|conn| {
            // Check if both nodes exist
            let from_node = self.nodes.get(&conn.from_node);
            let to_node = self.nodes.get(&conn.to_node);

            if from_node.is_none() {
                log::warn!(
                    "Removing connection: source node {} not found",
                    conn.from_node
                );
                return false;
            }
            if to_node.is_none() {
                log::warn!(
                    "Removing connection: target node {} not found",
                    conn.to_node
                );
                return false;
            }

            let from_node = from_node.unwrap();
            let to_node = to_node.unwrap();

            // Check if output pin exists on source node
            let from_pin_exists = from_node.outputs.iter().any(|p| p.id == conn.from_pin);
            if !from_pin_exists {
                log::warn!(
                    "Removing connection: output pin {} not found on node {} ({})",
                    conn.from_pin,
                    conn.from_node,
                    from_node.node_type.display_name()
                );
                return false;
            }

            // Check if input pin exists on target node
            let to_pin_exists = to_node.inputs.iter().any(|p| p.id == conn.to_pin);
            if !to_pin_exists {
                log::warn!(
                    "Removing connection: input pin {} not found on node {} ({})",
                    conn.to_pin,
                    conn.to_node,
                    to_node.node_type.display_name()
                );
                return false;
            }

            // Check if pin types are compatible
            let from_pin = from_node
                .outputs
                .iter()
                .find(|p| p.id == conn.from_pin)
                .unwrap();
            let to_pin = to_node.inputs.iter().find(|p| p.id == conn.to_pin).unwrap();

            if !pin_takes(&to_node.node_type, &to_pin.pin_type, &from_pin.pin_type) {
                log::warn!(
                    "Removing connection: incompatible pin types {:?} -> {:?} (node {} -> node {})",
                    from_pin.pin_type,
                    to_pin.pin_type,
                    conn.from_node,
                    conn.to_node
                );
                return false;
            }

            true
        });

        let removed_nodes = original_node_count - self.nodes.len();
        let removed_connections = original_connection_count - self.connections.len();

        if removed_nodes > 0 || removed_connections > 0 {
            log::warn!(
                "Graph validation: removed {} invalid nodes, {} invalid connections",
                removed_nodes,
                removed_connections
            );
        } else {
            log::info!("Graph validation: all nodes and connections are valid");
        }

        (removed_nodes, removed_connections)
    }

    pub fn add_connection_by_name(
        &mut self,
        from_node_id: NodeId,
        from_pin_name: &str,
        to_node_id: NodeId,
        to_pin_name: &str,
    ) -> Result<(), anyhow::Error> {
        let from_node = self
            .nodes
            .get(&from_node_id)
            .ok_or_else(|| anyhow::anyhow!("from_node not found"))?;
        let from_pin = from_node
            .outputs
            .iter()
            .find(|p| p.name == from_pin_name)
            .ok_or_else(|| anyhow::anyhow!("from_pin not found"))?;
        let to_node = self
            .nodes
            .get(&to_node_id)
            .ok_or_else(|| anyhow::anyhow!("to_node not found"))?;
        let to_pin = to_node
            .inputs
            .iter()
            .find(|p| p.name == to_pin_name)
            .ok_or_else(|| anyhow::anyhow!("to_pin not found"))?;

        let conn = Connection {
            from_node: from_node_id,
            from_pin: from_pin.id,
            to_node: to_node_id,
            to_pin: to_pin.id,
        };

        self.add_connection(conn);
        Ok(())
    }

    fn create_inputs_for_type(&mut self, node_type: &NodeType) -> Vec<Pin> {
        use NodeType::*;
        match node_type {
            // Its pins are those it was read with (`refresh` leaves it alone).
            Unknown(_) => Vec::new(),
            // Event nodes have no inputs
            OnWindowCreated
            | OnWindowDestroyed
            | OnWindowFocused
            | OnWindowMoved
            | OnWindowResized
            | OnKeyPress { .. }
            | OnStartup
            | OnStartupCompleted
            | ManualTrigger => vec![],

            // Action nodes
            LaunchProcess { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Program", PinType::String),
            ],

            MoveWindow { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("X", PinType::Number),
                self.create_pin("Y", PinType::Number),
            ],

            ResizeWindow { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Width", PinType::Number),
                self.create_pin("Height", PinType::Number),
            ],

            CloseWindow => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            SetWindowTitle { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Title", PinType::String),
            ],

            AnimateWindow { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Duration", PinType::Number),
            ],

            SetWindowArrangement { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            CreateFloatRule { .. } | CreateIgnoreRule { .. } | CreateManageRule { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Rules", PinType::RuleList), // Optional: dynamic rule list input
                self.create_pin("Pattern", PinType::String), // For single-rule mode (legacy)
            ],

            ApplySystemFilters { .. } => vec![self.create_pin("Trigger", PinType::Execution)],

            CreateRule { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Pattern", PinType::String),
                self.create_pin("Match Kind", PinType::MatchKind),
                self.create_pin("Match Strategy", PinType::MatchStrategy),
            ],

            CombineRules => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Rule 1", PinType::Rule),
                self.create_pin("Rule 2", PinType::Rule),
                self.create_pin("Rule 3", PinType::Rule),
                self.create_pin("Rule 4", PinType::Rule),
            ],

            BringToFront | SendToBack => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            SetAlwaysOnTop { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Enabled", PinType::Bool),
            ],

            // Windows 11 DWM feature nodes
            CloakWindow { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Cloak", PinType::Bool),
            ],

            SetNativeBorderColor { .. } | SetCaptionColor { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Color", PinType::Color),
            ],

            SetSystemBackdrop { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            SetWindowCorner { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            // Condition nodes
            WindowExists => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            WindowHasTitle { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Title", PinType::String),
            ],

            MatchWindow { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Pattern", PinType::String),
            ],

            WindowModeSelector { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            // Data nodes
            GetWindowPosition => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            GetWindowSize => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            GetActiveWindow => vec![self.create_pin("Trigger", PinType::Execution)],

            GetAllWindows => vec![self.create_pin("Trigger", PinType::Execution)],

            GetWindowInfo => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            GetMonitorForWindow => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            // Helper nodes
            ForEach => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("List", PinType::List), // Generic list input (accepts WindowList or MonitorList)
            ],

            ForLoop { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Start", PinType::Number), // Start value (inclusive)
                self.create_pin("End", PinType::Number),   // End value (inclusive)
                self.create_pin("Step", PinType::Number),  // Step value (default: 1)
            ],

            WindowListToSingle { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Windows", PinType::WindowList),
                self.create_pin("Index", PinType::Number),
            ],

            // Workspace nodes
            SwitchToWorkspace { .. } | FocusWorkspace { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Workspace", PinType::Number), // Optional: override workspace index
            ],

            MoveWindowToWorkspace { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Workspace", PinType::Number), // Optional: override workspace index
            ],

            GetCurrentWorkspace => vec![self.create_pin("Trigger", PinType::Execution)],

            NextWorkspace | PreviousWorkspace => {
                vec![self.create_pin("Trigger", PinType::Execution)]
            }

            FocusNextWindow
            | FocusPreviousWindow
            | CycleFocusNext
            | CycleFocusPrevious
            | SwapWithNext
            | SwapWithPrevious
            | MoveToNextWorkspace
            | MoveToPreviousWorkspace
            | ToggleFloating
            | ToggleExposeMode => vec![self.create_pin("Trigger", PinType::Execution)],

            // Monitor and Workspace Creation Nodes
            GetMonitors => vec![self.create_pin("Trigger", PinType::Execution)],

            GetMonitorInfo => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Monitor", PinType::Monitor),
            ],

            CreateWorkspace { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Monitor", PinType::Monitor),
                self.create_pin("Name", PinType::String),
                self.create_pin("Layout", PinType::Layout),
                self.create_pin("PaddingX", PinType::Number),
                self.create_pin("PaddingY", PinType::Number),
                self.create_pin("WorkspacePaddingX", PinType::Number),
                self.create_pin("WorkspacePaddingY", PinType::Number),
                // Border configuration inputs
                self.create_pin("BorderEnabled", PinType::Bool),
                self.create_pin("BorderWidth", PinType::Number),
                self.create_pin("BorderOffset", PinType::Number),
                self.create_pin("BorderStyle", PinType::BorderStyle),
                self.create_pin("ActiveBorderColor", PinType::Color),
                self.create_pin("InactiveBorderColor", PinType::Color),
                self.create_pin("ActiveBorderAlpha", PinType::Number),
                self.create_pin("InactiveBorderAlpha", PinType::Number),
            ],

            // Math nodes
            Add => vec![
                self.create_pin("A", PinType::Number),
                self.create_pin("B", PinType::Number),
            ],

            Subtract => vec![
                self.create_pin("A", PinType::Number),
                self.create_pin("B", PinType::Number),
            ],

            Multiply => vec![
                self.create_pin("A", PinType::Number),
                self.create_pin("B", PinType::Number),
            ],

            Divide => vec![
                self.create_pin("A", PinType::Number),
                self.create_pin("B", PinType::Number),
            ],

            // Value nodes
            NumberValue { .. }
            | StringValue { .. }
            | BoolValue { .. }
            | ColorValue { .. }
            | BorderStyleValue { .. } => vec![],

            // Conversion nodes
            ToString => vec![
                self.create_pin("Value", PinType::Number), // Accepts Number or Bool (will be compatible)
            ],

            FromString => vec![self.create_pin("String", PinType::String)],

            // Debug nodes
            LogMessage { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Text", PinType::String), // Optional string to append to message
            ],

            GetWorkspace { .. } => vec![self.create_pin("Trigger", PinType::Execution)],

            Retile { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Workspace", PinType::Number),
            ],

            // Layout configuration nodes
            SetLayoutRatio { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Ratio", PinType::Number), // Optional: override ratio
            ],

            IncrementLayoutRatio { .. } | DecrementLayoutRatio { .. } => {
                vec![self.create_pin("Trigger", PinType::Execution)]
            }

            SetMasterCount { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Count", PinType::Number), // Optional: override count
            ],

            IncrementMasterCount | DecrementMasterCount => {
                vec![self.create_pin("Trigger", PinType::Execution)]
            }

            SetLayoutGap { .. } => vec![
                self.create_pin("Trigger", PinType::Execution),
                self.create_pin("Gap", PinType::Number), // Optional: override gap
            ],

            CycleNextLayout | CyclePreviousLayout | RotateLayout => {
                vec![self.create_pin("Trigger", PinType::Execution)]
            }

            // Scratchpad nodes
            ToggleScratchpad { .. } | ShowScratchpad { .. } | HideScratchpad { .. } => {
                vec![self.create_pin("Trigger", PinType::Execution)]
            }
            HideAllScratchpads => vec![self.create_pin("Trigger", PinType::Execution)],
        }
    }

    fn create_outputs_for_type(&mut self, node_type: &NodeType) -> Vec<Pin> {
        use NodeType::*;
        match node_type {
            Unknown(_) => Vec::new(),
            // Event nodes output execution and event data
            OnWindowCreated => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            OnWindowDestroyed => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            OnWindowFocused => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            OnWindowMoved => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("X", PinType::Number),
                self.create_pin("Y", PinType::Number),
            ],

            OnWindowResized => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
                self.create_pin("Width", PinType::Number),
                self.create_pin("Height", PinType::Number),
            ],

            OnKeyPress { .. } => vec![
                self.create_pin("Event", PinType::Execution),
                self.create_pin("Key", PinType::String),
                self.create_pin("Ctrl", PinType::Bool),
                self.create_pin("Alt", PinType::Bool),
                self.create_pin("Shift", PinType::Bool),
                self.create_pin("Win", PinType::Bool),
            ],

            OnStartup => vec![self.create_pin("Event", PinType::Execution)],

            OnStartupCompleted => vec![self.create_pin("Event", PinType::Execution)],

            ManualTrigger => vec![self.create_pin("Trigger", PinType::Execution)],

            // Action nodes that operate on windows (pass through window)
            MoveWindow { .. }
            | ResizeWindow { .. }
            | CloseWindow
            | SetWindowTitle { .. }
            | AnimateWindow { .. }
            | SetWindowArrangement { .. }
            | BringToFront
            | SendToBack
            | SetAlwaysOnTop { .. }
            | CloakWindow { .. }
            | SetNativeBorderColor { .. }
            | SetCaptionColor { .. }
            | SetSystemBackdrop { .. }
            | SetWindowCorner { .. }
            | MoveWindowToWorkspace { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle), // Pass-through for chaining
            ],

            // Action nodes without window pass-through
            LaunchProcess { .. }
            | CreateFloatRule { .. }
            | CreateIgnoreRule { .. }
            | CreateManageRule { .. } => vec![self.create_pin("Then", PinType::Execution)],

            ApplySystemFilters { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Count", PinType::Number), // Number of rules added
            ],

            CreateRule { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Rule", PinType::Rule), // Output the created rule
            ],

            CombineRules => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Rules", PinType::RuleList), // Combined rule list
            ],

            // Condition nodes
            WindowExists => vec![
                self.create_pin("True", PinType::Execution),
                self.create_pin("False", PinType::Execution),
            ],

            WindowHasTitle { .. } => vec![
                self.create_pin("True", PinType::Execution),
                self.create_pin("False", PinType::Execution),
            ],

            MatchWindow { .. } => vec![
                self.create_pin("Matches", PinType::Execution),
                self.create_pin("No Match", PinType::Execution),
            ],

            WindowModeSelector { .. } => vec![
                self.create_pin("Tile", PinType::Execution),
                self.create_pin("Float", PinType::Execution),
            ],

            // Data nodes
            GetWindowPosition => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("X", PinType::Number),
                self.create_pin("Y", PinType::Number),
            ],

            GetWindowSize => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Width", PinType::Number),
                self.create_pin("Height", PinType::Number),
            ],

            GetActiveWindow => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            GetAllWindows => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Windows", PinType::WindowList),
            ],

            GetWindowInfo => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Title", PinType::String),
                self.create_pin("X", PinType::Number),
                self.create_pin("Y", PinType::Number),
                self.create_pin("Width", PinType::Number),
                self.create_pin("Height", PinType::Number),
            ],

            GetMonitorForWindow => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Monitor", PinType::Monitor),
                self.create_pin("Index", PinType::Number),
            ],

            // Helper nodes
            ForEach => vec![
                self.create_pin("Loop Body", PinType::Execution), // Executes for each item
                self.create_pin("Item", PinType::Item), // Current item in iteration (generic: WindowHandle or Monitor)
                self.create_pin("Index", PinType::Number), // Current index
                self.create_pin("Completed", PinType::Execution), // Executes after all items processed
            ],

            ForLoop { .. } => vec![
                self.create_pin("Loop Body", PinType::Execution), // Executes for each iteration
                self.create_pin("Index", PinType::Number),        // Current counter value
                self.create_pin("Completed", PinType::Execution), // Executes after loop completes
            ],

            WindowListToSingle { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle),
            ],

            // Workspace nodes
            SwitchToWorkspace { .. }
            | FocusWorkspace { .. }
            | NextWorkspace
            | PreviousWorkspace => vec![self.create_pin("Then", PinType::Execution)],

            FocusNextWindow
            | FocusPreviousWindow
            | CycleFocusNext
            | CycleFocusPrevious
            | SwapWithNext
            | SwapWithPrevious
            | MoveToNextWorkspace
            | MoveToPreviousWorkspace
            | ToggleFloating
            | ToggleExposeMode => vec![self.create_pin("Then", PinType::Execution)],

            GetCurrentWorkspace => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Workspace", PinType::Number),
            ],

            // Monitor and Workspace Creation Nodes
            GetMonitors => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Monitors", PinType::MonitorList),
            ],

            GetMonitorInfo => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Name", PinType::String),
                self.create_pin("X", PinType::Number),
                self.create_pin("Y", PinType::Number),
                self.create_pin("Width", PinType::Number),
                self.create_pin("Height", PinType::Number),
                self.create_pin("WorkX", PinType::Number),
                self.create_pin("WorkY", PinType::Number),
                self.create_pin("WorkWidth", PinType::Number),
                self.create_pin("WorkHeight", PinType::Number),
                self.create_pin("IsPrimary", PinType::Bool),
            ],

            CreateWorkspace { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Workspace", PinType::Number), // Created workspace index
                self.create_pin("Name", PinType::String),      // Passthrough name
                self.create_pin("Layout", PinType::Layout),    // Passthrough layout
            ],

            // Math nodes
            Add | Subtract | Multiply | Divide => vec![self.create_pin("Result", PinType::Number)],

            // Value nodes
            NumberValue { .. } => vec![self.create_pin("Value", PinType::Number)],
            StringValue { .. } => vec![self.create_pin("Value", PinType::String)],
            BoolValue { .. } => vec![self.create_pin("Value", PinType::Bool)],
            ColorValue { .. } => vec![self.create_pin("Value", PinType::Color)],
            BorderStyleValue { .. } => vec![self.create_pin("Value", PinType::BorderStyle)],

            // Conversion nodes
            ToString => vec![self.create_pin("String", PinType::String)],

            FromString => vec![self.create_pin("Number", PinType::Number)],

            // Debug nodes
            LogMessage { .. } => vec![self.create_pin("Then", PinType::Execution)],

            GetWorkspace { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Workspace", PinType::Number),
            ],

            Retile { .. } => vec![self.create_pin("Then", PinType::Execution)],

            // Layout configuration nodes
            SetLayoutRatio { .. }
            | IncrementLayoutRatio { .. }
            | DecrementLayoutRatio { .. }
            | SetMasterCount { .. }
            | IncrementMasterCount
            | DecrementMasterCount
            | SetLayoutGap { .. }
            | CycleNextLayout
            | CyclePreviousLayout
            | RotateLayout => vec![self.create_pin("Then", PinType::Execution)],

            // Scratchpad nodes
            ToggleScratchpad { .. } => vec![
                self.create_pin("Then", PinType::Execution),
                self.create_pin("Window", PinType::WindowHandle), // Returns the scratchpad window
                self.create_pin("Visible", PinType::Bool),        // Whether it's now visible
            ],
            ShowScratchpad { .. } | HideScratchpad { .. } => {
                vec![self.create_pin("Then", PinType::Execution)]
            }
            HideAllScratchpads => vec![self.create_pin("Then", PinType::Execution)],
        }
    }

    fn create_pin(&mut self, name: &str, pin_type: PinType) -> Pin {
        let id = self.next_pin_id;
        self.next_pin_id += 1;
        Pin {
            id,
            name: name.to_string(),
            pin_type,
        }
    }
}

impl Default for NodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Node {
    pub id: NodeId,
    pub node_type: NodeType,
    pub position: [f32; 2],
    pub inputs: Vec<Pin>,
    pub outputs: Vec<Pin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pin {
    pub id: PinId,
    pub name: String,
    pub pin_type: PinType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub enum PinType {
    Execution,
    WindowHandle,
    WindowList,
    Number,
    String,
    Bool,
    Color,         // RGB color (0-255 for each component)
    BorderStyle,   // Square, Rounded
    WindowRule,    // For passing window matching rules (DEPRECATED - use Rule instead)
    MatchKind,     // Exe, Class, Title, Path
    MatchStrategy, // Equals, Contains, StartsWith, etc
    Monitor,       // Single monitor reference
    MonitorList,   // List of monitors
    Layout,        // Workspace layout type (BSP, Grid, Columns, Rows, etc.)
    Rule,          // Single rule definition (pattern + match_kind + match_strategy)
    RuleList,      // List of rule definitions
    List,          // Generic list type (accepts WindowList, MonitorList, RuleList)
    Item,          // Generic item type (accepts WindowHandle, Monitor, Rule)
}

/// Whether what comes out of a pin of the kind `out` may go into an
/// input of the kind `into` on a node of this type: the same kind of
/// thing (`PinType::is_compatible`), and into the node that turns a
/// value into text a yes-or-no as well as the number its pin is named
/// for (the node writes `true` or `false`).
pub fn pin_takes(node: &NodeType, into: &PinType, out: &PinType) -> bool {
    out.is_compatible(into)
        || (matches!(node, NodeType::ToString) && *into == PinType::Number && *out == PinType::Bool)
}

impl PinType {
    pub fn is_compatible(&self, other: &PinType) -> bool {
        // Exact match
        if self == other {
            return true;
        }

        // Generic List type accepts WindowList, MonitorList, and RuleList
        // Generic Item type accepts WindowHandle, Monitor, and Rule
        match (self, other) {
            // List compatibility
            (PinType::List, PinType::WindowList) => true,
            (PinType::List, PinType::MonitorList) => true,
            (PinType::List, PinType::RuleList) => true,
            (PinType::WindowList, PinType::List) => true,
            (PinType::MonitorList, PinType::List) => true,
            (PinType::RuleList, PinType::List) => true,

            // Item compatibility
            (PinType::Item, PinType::WindowHandle) => true,
            (PinType::Item, PinType::Monitor) => true,
            (PinType::Item, PinType::Rule) => true,
            (PinType::WindowHandle, PinType::Item) => true,
            (PinType::Monitor, PinType::Item) => true,
            (PinType::Rule, PinType::Item) => true,

            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Connection {
    pub from_node: NodeId,
    pub from_pin: PinId,
    pub to_node: NodeId,
    pub to_pin: PinId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(remote = "Self")]
pub enum NodeType {
    /// Something this version does not know (written by a newer gevrek, or
    /// mistyped): kept as it was read and written back unchanged. It does
    /// nothing when run; `validate` reports it.
    #[serde(skip)]
    Unknown(serde_json::Value),
    // Event Nodes
    OnWindowCreated,
    OnWindowDestroyed,
    OnWindowFocused,
    OnWindowMoved,
    OnWindowResized,
    OnKeyPress {
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
        /// The Super key (the Windows key on Windows). Written as `super`;
        /// `win` is accepted when loading older files.
        #[serde(rename = "super", alias = "win")]
        win: bool,
    },
    OnStartup,
    OnStartupCompleted, // Fires after all OnStartup nodes have finished executing
    ManualTrigger,      // Test node that can be manually triggered by clicking a button

    // Action Nodes
    LaunchProcess {
        program: String,
        args: Vec<String>,
        working_directory: String,
        create_no_window: bool,
    },
    MoveWindow {
        x: i32,
        y: i32,
    },
    ResizeWindow {
        width: i32,
        height: i32,
    },
    CloseWindow,
    SetWindowTitle {
        title: String,
    },
    AnimateWindow {
        animation_type: AnimationType,
    },
    SetWindowArrangement {
        mode: WindowMode,
    }, // Mark window as tiling or floating

    // Rule Action Nodes
    CreateFloatRule {
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
        pattern: String,
        rules: Vec<RuleDefinition>, // Static rules configured in properties
    },
    CreateIgnoreRule {
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
        pattern: String,
        rules: Vec<RuleDefinition>, // Static rules configured in properties
    },
    CreateManageRule {
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
        pattern: String,
        rules: Vec<RuleDefinition>, // Static rules configured in properties
    },
    ApplySystemFilters {
        enabled: bool,
    }, // Apply built-in system filters (tooltips, taskbar, etc.)

    // Rule Helper Nodes
    CreateRule {
        pattern: String,
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
    }, // Build a single Rule from pattern + match_kind + match_strategy
    CombineRules, // Combine multiple Rules into a RuleList

    // Z-Order Management Nodes
    BringToFront,
    SendToBack,
    SetAlwaysOnTop {
        enabled: bool,
    },

    // Windows 11 DWM Feature Nodes
    /// Cloak a window (hide without minimize animation)
    CloakWindow {
        cloak: bool,
    },
    /// Set native Windows 11 border color
    SetNativeBorderColor {
        r: u8,
        g: u8,
        b: u8,
    },
    /// Set window caption/titlebar color
    SetCaptionColor {
        r: u8,
        g: u8,
        b: u8,
    },
    /// Set system backdrop type (Mica, Acrylic, etc.)
    SetSystemBackdrop {
        backdrop_type: SystemBackdropType,
    },
    /// Set window corner preference (rounded, square, etc.)
    SetWindowCorner {
        corner_type: WindowCornerType,
    },

    // Condition Nodes
    WindowExists,
    WindowHasTitle {
        title: String,
    },
    MatchWindow {
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
        pattern: String,
    },

    // Window Mode Selector - Decides if window should be tiled or floating
    WindowModeSelector {
        default_mode: WindowMode,
        // Rules are checked in order - first match wins
    },

    // Data Nodes
    GetWindowPosition,
    GetWindowSize,
    GetActiveWindow,
    GetAllWindows,
    GetWindowInfo,       // Gets window title, class, exe from a WindowHandle
    GetMonitorForWindow, // Gets the monitor for a window

    // Helper Nodes
    ForEach, // Generic iterator - iterates over any list type (WindowList, etc.)
    ForLoop {
        start: i32,
        end: i32,
        step: i32,
    }, // Numeric for loop - iterates from start to end
    WindowListToSingle {
        index: usize,
    }, // Extracts single window from WindowList by index

    // Workspace Nodes
    SwitchToWorkspace {
        workspace_index: usize,
    }, // Switch to a specific workspace
    MoveWindowToWorkspace {
        workspace_index: usize,
    }, // Move window to workspace
    GetCurrentWorkspace, // Get current workspace index
    GetWorkspace {
        workspace_index: usize,
    }, // Get a specific workspace index (for flexibility)
    NextWorkspace,       // Switch to next workspace
    PreviousWorkspace,   // Switch to previous workspace
    FocusWorkspace {
        workspace_index: usize,
    }, // Alias for SwitchToWorkspace
    Retile {
        layout: TileLayout,
    }, // Retile a workspace with a specific layout

    // Layout Configuration Nodes
    /// Set the master area ratio for layouts that support it (0.1 to 0.9)
    SetLayoutRatio {
        ratio: f32,
    },
    /// Increment master area ratio by a step
    IncrementLayoutRatio {
        step: f32,
    },
    /// Decrement master area ratio by a step
    DecrementLayoutRatio {
        step: f32,
    },
    /// Set the number of master windows (1-4)
    SetMasterCount {
        count: usize,
    },
    /// Increment master window count
    IncrementMasterCount,
    /// Decrement master window count
    DecrementMasterCount,
    /// Set window gap/padding between tiled windows
    SetLayoutGap {
        gap: i32,
    },
    /// Cycle to next layout in the list
    CycleNextLayout,
    /// Cycle to previous layout in the list
    CyclePreviousLayout,
    /// Rotate layout orientation (e.g., master from left to right)
    RotateLayout,

    // Window Navigation and Management
    FocusNextWindow,         // Focus next window in current workspace
    FocusPreviousWindow,     // Focus previous window in current workspace
    CycleFocusNext,          // Cycle to next window in focus history (Alt-Tab style)
    CycleFocusPrevious,      // Cycle to previous window in focus history (Alt-Tab style)
    SwapWithNext,            // Swap current window position with next window
    SwapWithPrevious,        // Swap current window position with previous window
    MoveToNextWorkspace,     // Move active window to next workspace (with wrap-around)
    MoveToPreviousWorkspace, // Move active window to previous workspace (with wrap-around)
    ToggleFloating,          // Toggle active window between floating and tiled modes
    ToggleExposeMode,        // Toggle Expose mode (window overview grid)

    // Monitor and Workspace Creation Nodes
    GetMonitors,    // Get all monitors as MonitorList
    GetMonitorInfo, // Get info about a specific monitor (name, dimensions, etc.)
    CreateWorkspace {
        workspace_name: String,
        layout: TileLayout,
        padding_x: i32,
        padding_y: i32,
        workspace_padding_x: i32,
        workspace_padding_y: i32,
        // Border configuration
        border_enabled: bool,
        border_width: i32,
        border_offset: i32,
        border_style: BorderStyle,
        active_border_color: (u8, u8, u8),
        inactive_border_color: (u8, u8, u8),
        active_border_alpha: u8,
        inactive_border_alpha: u8,
    }, // Create workspace on a monitor

    // Math Nodes
    Add,
    Subtract,
    Multiply,
    Divide,

    // Value Nodes
    NumberValue {
        value: f64,
    },
    StringValue {
        value: String,
    },
    BoolValue {
        value: bool,
    },
    ColorValue {
        r: u8,
        g: u8,
        b: u8,
    }, // RGB color value (0-255 each)
    BorderStyleValue {
        value: BorderStyle,
    }, // Border style value (Square/Rounded)

    // Conversion Nodes
    ToString,   // Converts Number/Bool to String
    FromString, // Converts String to Number (parses)

    // Debug Nodes
    LogMessage {
        message: String,
        level: LogLevel,
    }, // Logs a message to the visual log panel

    // Scratchpad Nodes - Toggle-able dropdown windows
    ToggleScratchpad {
        /// Identifier for this scratchpad (can have multiple)
        scratchpad_id: String,
        /// Pattern to match window (exe, class, or title)
        pattern: String,
        /// How to match the pattern
        match_kind: RuleMatchKind,
        match_strategy: RuleMatchStrategy,
        /// Command to run if window doesn't exist (optional)
        spawn_command: Option<String>,
        /// Width as percentage of screen (0-100)
        width_percent: u8,
        /// Height as percentage of screen (0-100)
        height_percent: u8,
        /// Position: Top, Bottom, Center
        position: ScratchpadPosition,
    },
    ShowScratchpad {
        scratchpad_id: String,
    }, // Show scratchpad (no toggle)
    HideScratchpad {
        scratchpad_id: String,
    }, // Hide scratchpad (no toggle)
    HideAllScratchpads, // Hide all scratchpads
}

/// Capability a backend reports when it can apply the Windows-only window
/// effects (DWM backdrop, corner preference, caption and native border
/// colour, cloaking).
pub const FEATURE_WINDOWS_WINDOW_EFFECTS: &str = "platform.windows.window_effects";

impl NodeType {
    /// The capability a backend must report for this node to do anything;
    /// `None` for nodes every backend supports.
    pub fn required_feature(&self) -> Option<&'static str> {
        match self {
            NodeType::CloakWindow { .. }
            | NodeType::SetNativeBorderColor { .. }
            | NodeType::SetCaptionColor { .. }
            | NodeType::SetSystemBackdrop { .. }
            | NodeType::SetWindowCorner { .. } => Some(FEATURE_WINDOWS_WINDOW_EFFECTS),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            NodeType::Unknown(_) => "Unknown node",
            NodeType::OnWindowCreated => "On Window Created",
            NodeType::OnWindowDestroyed => "On Window Destroyed",
            NodeType::OnWindowFocused => "On Window Focused",
            NodeType::OnWindowMoved => "On Window Moved",
            NodeType::OnWindowResized => "On Window Resized",
            NodeType::OnKeyPress { .. } => "On Key Press",
            NodeType::OnStartup => "On Startup",
            NodeType::OnStartupCompleted => "On Startup Completed",
            NodeType::ManualTrigger => "Manual Trigger",
            NodeType::LaunchProcess { .. } => "Launch Process",
            NodeType::MoveWindow { .. } => "Move Window",
            NodeType::ResizeWindow { .. } => "Resize Window",
            NodeType::CloseWindow => "Close Window",
            NodeType::SetWindowTitle { .. } => "Set Window Title",
            NodeType::AnimateWindow { .. } => "Animate Window",
            NodeType::SetWindowArrangement { .. } => "Set Window Arrangement",
            NodeType::CreateFloatRule { .. } => "Create Float Rule",
            NodeType::CreateIgnoreRule { .. } => "Create Ignore Rule",
            NodeType::CreateManageRule { .. } => "Create Manage Rule",
            NodeType::ApplySystemFilters { .. } => "Apply System Filters",
            NodeType::CreateRule { .. } => "Create Rule",
            NodeType::CombineRules => "Combine Rules",
            NodeType::BringToFront => "Bring To Front",
            NodeType::SendToBack => "Send To Back",
            NodeType::SetAlwaysOnTop { .. } => "Set Always On Top",
            NodeType::CloakWindow { .. } => "Cloak Window",
            NodeType::SetNativeBorderColor { .. } => "Set Native Border Color",
            NodeType::SetCaptionColor { .. } => "Set Caption Color",
            NodeType::SetSystemBackdrop { .. } => "Set System Backdrop",
            NodeType::SetWindowCorner { .. } => "Set Window Corner",
            NodeType::WindowExists => "Window Exists?",
            NodeType::WindowHasTitle { .. } => "Window Has Title?",
            NodeType::MatchWindow { .. } => "Match Window?",
            NodeType::WindowModeSelector { .. } => "Window Mode Selector",
            NodeType::GetWindowPosition => "Get Window Position",
            NodeType::GetWindowSize => "Get Window Size",
            NodeType::GetActiveWindow => "Get Active Window",
            NodeType::GetAllWindows => "Get All Windows",
            NodeType::GetWindowInfo => "Get Window Info",
            NodeType::GetMonitorForWindow => "Get Monitor for Window",
            NodeType::ForEach => "For Each",
            NodeType::ForLoop { .. } => "For Loop",
            NodeType::WindowListToSingle { .. } => "Get Window At Index",
            NodeType::SwitchToWorkspace { .. } => "Switch To Workspace",
            NodeType::MoveWindowToWorkspace { .. } => "Move Window To Workspace",
            NodeType::GetCurrentWorkspace => "Get Current Workspace",
            NodeType::GetWorkspace { .. } => "Get Workspace",
            NodeType::NextWorkspace => "Next Workspace",
            NodeType::PreviousWorkspace => "Previous Workspace",
            NodeType::FocusNextWindow => "Focus Next Window",
            NodeType::FocusPreviousWindow => "Focus Previous Window",
            NodeType::CycleFocusNext => "Cycle Focus Next",
            NodeType::CycleFocusPrevious => "Cycle Focus Previous",
            NodeType::SwapWithNext => "Swap With Next",
            NodeType::SwapWithPrevious => "Swap With Previous",
            NodeType::MoveToNextWorkspace => "Move To Next Workspace",
            NodeType::MoveToPreviousWorkspace => "Move To Previous Workspace",
            NodeType::ToggleFloating => "Toggle Floating",
            NodeType::ToggleExposeMode => "Toggle Expose Mode",
            NodeType::FocusWorkspace { .. } => "Focus Workspace",
            NodeType::Retile { .. } => "Retile Workspace",
            NodeType::SetLayoutRatio { .. } => "Set Layout Ratio",
            NodeType::IncrementLayoutRatio { .. } => "Increment Layout Ratio",
            NodeType::DecrementLayoutRatio { .. } => "Decrement Layout Ratio",
            NodeType::SetMasterCount { .. } => "Set Master Count",
            NodeType::IncrementMasterCount => "Increment Master Count",
            NodeType::DecrementMasterCount => "Decrement Master Count",
            NodeType::SetLayoutGap { .. } => "Set Layout Gap",
            NodeType::CycleNextLayout => "Cycle Next Layout",
            NodeType::CyclePreviousLayout => "Cycle Previous Layout",
            NodeType::RotateLayout => "Rotate Layout",
            NodeType::GetMonitors => "Get Monitors",
            NodeType::GetMonitorInfo => "Get Monitor Info",
            NodeType::CreateWorkspace { .. } => "Create Workspace",
            NodeType::Add => "Add",
            NodeType::Subtract => "Subtract",
            NodeType::Multiply => "Multiply",
            NodeType::Divide => "Divide",
            NodeType::NumberValue { .. } => "Number",
            NodeType::StringValue { .. } => "String",
            NodeType::BoolValue { .. } => "Boolean",
            NodeType::ColorValue { .. } => "Color",
            NodeType::BorderStyleValue { .. } => "Border Style",
            NodeType::ToString => "To String",
            NodeType::FromString => "From String",
            NodeType::LogMessage { .. } => "Log Message",
            NodeType::ToggleScratchpad { .. } => "Toggle Scratchpad",
            NodeType::ShowScratchpad { .. } => "Show Scratchpad",
            NodeType::HideScratchpad { .. } => "Hide Scratchpad",
            NodeType::HideAllScratchpads => "Hide All Scratchpads",
        }
    }

    pub fn category(&self) -> &str {
        match self {
            NodeType::Unknown(_) => "Debug",
            NodeType::OnWindowCreated
            | NodeType::OnWindowDestroyed
            | NodeType::OnWindowFocused
            | NodeType::OnWindowMoved
            | NodeType::OnWindowResized
            | NodeType::OnKeyPress { .. }
            | NodeType::OnStartup
            | NodeType::OnStartupCompleted
            | NodeType::ManualTrigger => "Events",

            NodeType::LaunchProcess { .. }
            | NodeType::MoveWindow { .. }
            | NodeType::ResizeWindow { .. }
            | NodeType::CloseWindow
            | NodeType::SetWindowTitle { .. }
            | NodeType::AnimateWindow { .. }
            | NodeType::SetWindowArrangement { .. }
            | NodeType::BringToFront
            | NodeType::SendToBack
            | NodeType::SetAlwaysOnTop { .. }
            | NodeType::CloakWindow { .. }
            | NodeType::SetNativeBorderColor { .. }
            | NodeType::SetCaptionColor { .. }
            | NodeType::SetSystemBackdrop { .. }
            | NodeType::SetWindowCorner { .. } => "Actions",

            NodeType::CreateFloatRule { .. }
            | NodeType::CreateIgnoreRule { .. }
            | NodeType::CreateManageRule { .. }
            | NodeType::ApplySystemFilters { .. } => "Rules",

            NodeType::WindowExists
            | NodeType::WindowHasTitle { .. }
            | NodeType::MatchWindow { .. }
            | NodeType::WindowModeSelector { .. } => "Conditions",

            NodeType::GetWindowPosition
            | NodeType::GetWindowSize
            | NodeType::GetActiveWindow
            | NodeType::GetAllWindows
            | NodeType::GetWindowInfo
            | NodeType::GetMonitorForWindow => "Data",

            NodeType::ForEach
            | NodeType::ForLoop { .. }
            | NodeType::WindowListToSingle { .. }
            | NodeType::CreateRule { .. }
            | NodeType::CombineRules => "Helpers",

            NodeType::SwitchToWorkspace { .. }
            | NodeType::MoveWindowToWorkspace { .. }
            | NodeType::GetCurrentWorkspace
            | NodeType::NextWorkspace
            | NodeType::PreviousWorkspace
            | NodeType::FocusWorkspace { .. }
            | NodeType::GetMonitors
            | NodeType::GetMonitorInfo
            | NodeType::CreateWorkspace { .. }
            | NodeType::GetWorkspace { .. }
            | NodeType::Retile { .. }
            | NodeType::SetLayoutRatio { .. }
            | NodeType::IncrementLayoutRatio { .. }
            | NodeType::DecrementLayoutRatio { .. }
            | NodeType::SetMasterCount { .. }
            | NodeType::IncrementMasterCount
            | NodeType::DecrementMasterCount
            | NodeType::SetLayoutGap { .. }
            | NodeType::CycleNextLayout
            | NodeType::CyclePreviousLayout
            | NodeType::RotateLayout
            | NodeType::FocusNextWindow
            | NodeType::FocusPreviousWindow
            | NodeType::CycleFocusNext
            | NodeType::CycleFocusPrevious
            | NodeType::SwapWithNext
            | NodeType::SwapWithPrevious
            | NodeType::MoveToNextWorkspace
            | NodeType::MoveToPreviousWorkspace
            | NodeType::ToggleFloating
            | NodeType::ToggleExposeMode => "Workspaces",

            NodeType::Add | NodeType::Subtract | NodeType::Multiply | NodeType::Divide => "Math",

            NodeType::NumberValue { .. }
            | NodeType::StringValue { .. }
            | NodeType::BoolValue { .. }
            | NodeType::ColorValue { .. }
            | NodeType::BorderStyleValue { .. } => "Values",

            NodeType::ToString | NodeType::FromString => "Conversion",

            NodeType::LogMessage { .. } => "Debug",

            NodeType::ToggleScratchpad { .. }
            | NodeType::ShowScratchpad { .. }
            | NodeType::HideScratchpad { .. }
            | NodeType::HideAllScratchpads => "Scratchpad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum AnimationType {
    Fade,
    Slide,
    Scale,
    Expand,
    Contract,
}

// TileLayout is now imported from dwm::layouts::LayoutType
pub use crate::layout::LayoutType as TileLayout;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum StackDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum RuleMatchKind {
    Exe,
    Class,
    Title,
    Path,
    /// The application id (see `rules::MatchKind::AppId`).
    AppId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum RuleMatchStrategy {
    Equals,
    Contains,
    StartsWith,
    EndsWith,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum FloatStyle {
    Cascade, // Windows cascade down-right
    Center,  // Windows centered on screen
    Natural, // Keep current positions
    Grid,    // Arrange in overlapping grid
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum WindowMode {
    Auto,     // Automatically detect based on window type (smart detection)
    Tiled,    // Force tiled mode
    Floating, // Force floating mode
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub enum BorderStyle {
    /// Sharp corners (classic style)
    Square,
    /// Rounded corners (Windows 11 style)
    #[default]
    Rounded,
    /// Soft outer glow effect
    Glow,
    /// Drop shadow behind window
    Shadow,
    /// Dashed line border
    Dashed,
    /// Double-line border
    Double,
    /// Gradient-filled border (use with GradientBorderConfig)
    Gradient,
    /// Native Windows 11 border (uses DWMWA_BORDER_COLOR)
    Native,
}

/// Gradient direction for border gradients
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub enum GradientDirection {
    /// Top to bottom
    #[default]
    Vertical,
    /// Left to right
    Horizontal,
    /// Diagonal from top-left to bottom-right
    Diagonal,
    /// Corner to corner (all 4 corners can have different colors)
    Corner,
    /// Radial from center outward
    Radial,
}

/// Configuration for gradient borders
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GradientBorderConfig {
    /// Start color (RGB)
    pub start_color: (u8, u8, u8),
    /// End color (RGB)
    pub end_color: (u8, u8, u8),
    /// Direction of the gradient
    pub direction: GradientDirection,
    /// Optional middle color for 3-stop gradients
    pub mid_color: Option<(u8, u8, u8)>,
}

impl Default for GradientBorderConfig {
    fn default() -> Self {
        Self {
            start_color: (0, 120, 215), // Windows blue
            end_color: (156, 39, 176),  // Purple
            direction: GradientDirection::Diagonal,
            mid_color: None,
        }
    }
}

/// Windows 11 system backdrop types for node editor
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
pub enum SystemBackdropType {
    /// Let the system decide
    #[default]
    Auto,
    /// No backdrop effect
    None,
    /// Mica effect (main material)
    Mica,
    /// Acrylic blur effect
    Acrylic,
    /// Mica Alt (tabbed variant)
    MicaAlt,
}

/// Windows 11 window corner types for node editor
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
pub enum WindowCornerType {
    /// System default
    #[default]
    Default,
    /// Do not round corners (square)
    Square,
    /// Standard rounded corners
    Rounded,
    /// Small rounded corners
    RoundedSmall,
}

/// The groups of the node menu, in display order.
pub const NODE_CATALOG_GROUPS: [&str; 17] = [
    "Events",
    "Actions",
    "Focus",
    "Layout",
    "Workspaces",
    "Workspace navigation",
    "Scratchpads",
    "Window appearance",
    "Rules",
    "Conditions",
    "Data",
    "Monitors",
    "Helpers",
    "Math",
    "Values",
    "Conversion",
    "Debug",
];

/// Every node type a graph can contain, with its default settings, grouped
/// for the node menu. This is the one list of node types: the editor builds
/// its menu from it, with or without a window manager connected.
pub fn node_catalog() -> Vec<(&'static str, Vec<NodeType>)> {
    NODE_CATALOG_GROUPS
        .iter()
        .map(|group| (*group, catalog_group(group)))
        .collect()
}

fn catalog_group(category: &str) -> Vec<NodeType> {
    match category {
        "Events" => vec![
            NodeType::OnStartup,
            NodeType::OnStartupCompleted,
            NodeType::ManualTrigger,
            NodeType::OnWindowCreated,
            NodeType::OnWindowDestroyed,
            NodeType::OnWindowFocused,
            NodeType::OnWindowMoved,
            NodeType::OnWindowResized,
            NodeType::OnKeyPress {
                key: String::new(),
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
        ],
        "Actions" => vec![
            NodeType::LaunchProcess {
                program: "notepad.exe".to_string(),
                args: Vec::new(),
                working_directory: String::new(),
                create_no_window: false,
            },
            NodeType::MoveWindow { x: 0, y: 0 },
            NodeType::ResizeWindow {
                width: 800,
                height: 600,
            },
            NodeType::CloseWindow,
            NodeType::SetWindowTitle {
                title: String::new(),
            },
            NodeType::AnimateWindow {
                animation_type: AnimationType::Fade,
            },
            NodeType::SetWindowArrangement {
                mode: WindowMode::Auto,
            },
            NodeType::BringToFront,
            NodeType::SendToBack,
            NodeType::SetAlwaysOnTop { enabled: true },
        ],
        "Rules" => vec![
            NodeType::ApplySystemFilters { enabled: true },
            NodeType::CreateFloatRule {
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                pattern: String::new(),
                rules: Vec::new(),
            },
            NodeType::CreateIgnoreRule {
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                pattern: String::new(),
                rules: Vec::new(),
            },
            NodeType::CreateManageRule {
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                pattern: String::new(),
                rules: Vec::new(),
            },
        ],
        "Conditions" => vec![
            NodeType::WindowExists,
            NodeType::WindowHasTitle {
                title: String::new(),
            },
            NodeType::MatchWindow {
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                pattern: String::new(),
            },
            NodeType::WindowModeSelector {
                default_mode: WindowMode::Tiled,
            },
        ],
        "Data" => vec![
            NodeType::GetWindowPosition,
            NodeType::GetWindowSize,
            NodeType::GetActiveWindow,
            NodeType::GetAllWindows,
            NodeType::GetWindowInfo,
        ],
        "Helpers" => vec![
            NodeType::ForEach,
            NodeType::ForLoop {
                start: 0,
                end: 5,
                step: 1,
            },
            NodeType::WindowListToSingle { index: 0 },
            NodeType::CreateRule {
                pattern: String::new(),
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
            },
            NodeType::CombineRules,
        ],
        "Workspaces" => vec![
            NodeType::GetMonitors,
            NodeType::GetMonitorInfo,
            NodeType::CreateWorkspace {
                workspace_name: String::new(),
                layout: TileLayout::BSP,
                padding_x: 5,
                padding_y: 5,
                workspace_padding_x: 10,
                workspace_padding_y: 10,
                border_enabled: false,
                border_width: 4,
                border_offset: -1,
                border_style: BorderStyle::Square,
                active_border_color: (0, 255, 0),
                inactive_border_color: (100, 100, 100),
                active_border_alpha: 255,
                inactive_border_alpha: 200,
            },
            NodeType::SwitchToWorkspace { workspace_index: 0 },
            NodeType::MoveWindowToWorkspace { workspace_index: 0 },
            NodeType::GetCurrentWorkspace,
            NodeType::NextWorkspace,
            NodeType::PreviousWorkspace,
        ],
        "Math" => vec![
            NodeType::Add,
            NodeType::Subtract,
            NodeType::Multiply,
            NodeType::Divide,
        ],
        "Values" => vec![
            NodeType::NumberValue { value: 0.0 },
            NodeType::StringValue {
                value: String::new(),
            },
            NodeType::BoolValue { value: false },
            NodeType::ColorValue {
                r: 255,
                g: 100,
                b: 0,
            }, // Default orange color
            NodeType::BorderStyleValue {
                value: BorderStyle::Square,
            }, // Default square style
        ],
        "Conversion" => vec![NodeType::ToString, NodeType::FromString],
        "Debug" => vec![NodeType::LogMessage {
            message: "Log message here".to_string(),
            level: LogLevel::Info,
        }],
        "Focus" => vec![
            NodeType::FocusNextWindow,
            NodeType::FocusPreviousWindow,
            NodeType::CycleFocusNext,
            NodeType::CycleFocusPrevious,
            NodeType::SwapWithNext,
            NodeType::SwapWithPrevious,
            NodeType::ToggleFloating,
            NodeType::ToggleExposeMode,
        ],
        "Layout" => vec![
            NodeType::CycleNextLayout,
            NodeType::CyclePreviousLayout,
            NodeType::RotateLayout,
            NodeType::Retile {
                layout: TileLayout::BSP,
            },
            NodeType::SetLayoutRatio { ratio: 0.5 },
            NodeType::IncrementLayoutRatio { step: 0.05 },
            NodeType::DecrementLayoutRatio { step: 0.05 },
            NodeType::SetMasterCount { count: 1 },
            NodeType::IncrementMasterCount,
            NodeType::DecrementMasterCount,
            NodeType::SetLayoutGap { gap: 10 },
        ],
        "Workspace navigation" => vec![
            NodeType::FocusWorkspace { workspace_index: 0 },
            NodeType::GetWorkspace { workspace_index: 0 },
            NodeType::MoveToNextWorkspace,
            NodeType::MoveToPreviousWorkspace,
        ],
        "Scratchpads" => vec![
            NodeType::ToggleScratchpad {
                scratchpad_id: "scratchpad".to_string(),
                pattern: String::new(),
                match_kind: RuleMatchKind::Title,
                match_strategy: RuleMatchStrategy::Contains,
                spawn_command: None,
                width_percent: 60,
                height_percent: 60,
                position: ScratchpadPosition::Center,
            },
            NodeType::ShowScratchpad {
                scratchpad_id: "scratchpad".to_string(),
            },
            NodeType::HideScratchpad {
                scratchpad_id: "scratchpad".to_string(),
            },
            NodeType::HideAllScratchpads,
        ],
        // Windows-only (DWM): shown when the backend reports the capability.
        "Window appearance" => vec![
            NodeType::CloakWindow { cloak: true },
            NodeType::SetNativeBorderColor {
                r: 255,
                g: 100,
                b: 0,
            },
            NodeType::SetCaptionColor {
                r: 30,
                g: 30,
                b: 30,
            },
            NodeType::SetSystemBackdrop {
                backdrop_type: SystemBackdropType::Mica,
            },
            NodeType::SetWindowCorner {
                corner_type: WindowCornerType::Rounded,
            },
        ],
        "Monitors" => vec![NodeType::GetMonitorForWindow],
        _ => vec![],
    }
}

#[cfg(test)]
mod graph_index_tests {
    use super::*;

    /// A file written when a node had fewer pins: its connections follow
    /// the pin names, and refreshing twice from the same file gives the
    /// same pin ids.
    #[test]
    fn refreshed_pins_keep_connections_by_name_and_get_stable_ids() {
        let build = || {
            let mut graph = NodeGraph::new();
            let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
            let log = graph.add_node(
                NodeType::LogMessage {
                    message: "x".to_string(),
                    level: LogLevel::Info,
                },
                [100.0, 0.0],
            );
            let text = graph.add_node(
                NodeType::StringValue {
                    value: "hello".to_string(),
                },
                [0.0, 100.0],
            );
            assert!(graph
                .add_connection_by_name(trigger, "Trigger", log, "Trigger")
                .is_ok());
            let value_out = graph.nodes[&text].outputs[0].name.clone();
            assert!(graph
                .add_connection_by_name(text, &value_out, log, "Text")
                .is_ok());
            // The file is from a time when the log node's pins were in the
            // other order (and one more node type had extra pins).
            let node = graph.nodes.get_mut(&log).unwrap();
            node.inputs.reverse();
            (graph, log, text)
        };

        let (mut first, log, text) = build();
        first.refresh_node_pins();
        let text_pin = first.nodes[&log]
            .inputs
            .iter()
            .find(|pin| pin.name == "Text")
            .unwrap()
            .id;
        assert!(
            first
                .connections
                .iter()
                .any(|c| c.from_node == text && c.to_node == log && c.to_pin == text_pin),
            "the string still feeds the Text pin, not the pin at its old position"
        );
        assert_eq!(first.connections.len(), 2);

        let (mut second, ..) = build();
        second.refresh_node_pins();
        let ids = |graph: &NodeGraph| {
            let mut nodes: Vec<_> = graph.nodes.values().collect();
            nodes.sort_by_key(|node| node.id);
            nodes
                .iter()
                .flat_map(|node| node.inputs.iter().chain(&node.outputs).map(|pin| pin.id))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&first), ids(&second));
    }

    #[test]
    fn the_super_modifier_is_written_as_super_and_win_still_loads() {
        let node = NodeType::OnKeyPress {
            key: "J".to_string(),
            ctrl: false,
            alt: true,
            shift: false,
            win: true,
        };
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["OnKeyPress"]["super"], true);
        assert!(json["OnKeyPress"].get("win").is_none());

        let old = serde_json::json!({
            "OnKeyPress": {"key": "J", "ctrl": false, "alt": true, "shift": false, "win": true}
        });
        assert_eq!(serde_json::from_value::<NodeType>(old).unwrap(), node);

        let binding: crate::settings::BindingProfile =
            serde_json::from_value(serde_json::json!({"key": "K", "win": true})).unwrap();
        assert!(binding.win);
        assert_eq!(serde_json::to_value(&binding).unwrap()["super"], true);
    }

    #[test]
    fn only_the_windows_effect_nodes_need_a_capability() {
        let gated: Vec<String> = node_catalog()
            .into_iter()
            .flat_map(|(_, nodes)| nodes)
            .filter(|node| node.required_feature().is_some())
            .map(|node| variant_name(&node))
            .collect();
        assert_eq!(
            gated,
            [
                "CloakWindow",
                "SetNativeBorderColor",
                "SetCaptionColor",
                "SetSystemBackdrop",
                "SetWindowCorner"
            ]
        );
    }

    /// The variant name of a node type, from its debug form.
    fn variant_name(node_type: &NodeType) -> String {
        format!("{node_type:?}")
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn the_node_catalog_lists_every_node_type_exactly_once() {
        // The variant names are read from the enum's source, so a new
        // variant that is not added to the catalog fails here.
        let source = include_str!("graph.rs");
        let body = source
            .split_once("pub enum NodeType {")
            .expect("NodeType enum")
            .1
            .split_once("\n}")
            .expect("end of NodeType enum")
            .0;
        let declared: std::collections::BTreeSet<String> = body
            .lines()
            .map(str::trim)
            .filter(|line| line.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
            .map(|line| {
                line.split(|c: char| !c.is_alphanumeric())
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            // What a newer gevrek wrote: not something to offer.
            .filter(|name| name != "Unknown")
            .collect();

        let mut listed = Vec::new();
        for (_, nodes) in node_catalog() {
            listed.extend(nodes.iter().map(variant_name));
        }
        let unique: std::collections::BTreeSet<String> = listed.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            listed.len(),
            "a node type is listed twice: {listed:?}"
        );

        let missing: Vec<&String> = declared.difference(&unique).collect();
        let unknown: Vec<&String> = unique.difference(&declared).collect();
        assert!(
            missing.is_empty() && unknown.is_empty(),
            "missing from the catalog: {missing:?}; not a NodeType variant: {unknown:?}"
        );
    }

    #[test]
    fn the_problems_of_a_graph_are_the_wires_loading_would_take_out() {
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let log_node = graph.add_node(
            NodeType::LogMessage {
                message: String::new(),
                level: LogLevel::Info,
            },
            [100.0, 0.0],
        );
        graph
            .add_connection_by_name(trigger, "Trigger", log_node, "Trigger")
            .expect("nodes should connect");
        assert_eq!(graph.problems(), Vec::new(), "a sound graph has none");

        let good = graph.connections[0].clone();
        let nowhere: NodeId = good.from_node.max(good.to_node) + 100;
        // An end that is not there, at either side, and a pin no node has.
        graph.connections.push(Connection {
            from_node: nowhere,
            ..good.clone()
        });
        graph.connections.push(Connection {
            to_node: nowhere,
            ..good.clone()
        });
        graph.connections.push(Connection {
            to_pin: good.to_pin + 1000,
            ..good.clone()
        });
        graph.connections.push(Connection {
            from_pin: good.from_pin + 1000,
            ..good.clone()
        });

        let problems = graph.problems();
        assert_eq!(problems.len(), 4);
        let at: Vec<Option<NodeId>> = problems.iter().map(|problem| problem.node).collect();
        assert_eq!(
            at,
            [Some(log_node), Some(trigger), Some(log_node), Some(trigger)]
        );
        assert!(problems[0]
            .text
            .contains("comes from a node that is not there"));
        assert!(problems[2].text.contains("input"));

        // Exactly what loading takes out; afterwards nothing is wrong.
        let (_, removed) = graph.validate_and_clean();
        assert_eq!(removed, problems.len());
        assert_eq!(graph.problems(), Vec::new());
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].to_pin, good.to_pin);
    }

    #[test]
    fn removing_node_rebuilds_connection_indexes() {
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let log_node = graph.add_node(
            NodeType::LogMessage {
                message: String::new(),
                level: LogLevel::Info,
            },
            [100.0, 0.0],
        );
        graph
            .add_connection_by_name(trigger, "Trigger", log_node, "Trigger")
            .expect("nodes should connect");

        let connection = graph.connections[0].clone();
        graph.remove_node(trigger);

        assert!(graph.connections.is_empty());
        assert!(!graph.connection_set.contains(&(
            connection.from_node,
            connection.from_pin,
            connection.to_node,
            connection.to_pin,
        )));
        assert!(!graph
            .input_connection_set
            .contains(&(connection.to_node, connection.to_pin)));
    }

    #[test]
    fn auto_conversion_updates_connection_indexes() {
        let mut graph = NodeGraph::new();
        let number = graph.add_node(NodeType::NumberValue { value: 42.0 }, [0.0, 0.0]);
        let log_node = graph.add_node(
            NodeType::LogMessage {
                message: String::new(),
                level: LogLevel::Info,
            },
            [200.0, 0.0],
        );

        graph
            .add_connection_by_name(number, "Value", log_node, "Text")
            .expect("number-to-string connection should insert a converter");

        assert_eq!(graph.connections.len(), 2);
        assert_eq!(graph.connection_set.len(), 2);
        for connection in &graph.connections {
            assert!(graph.connection_set.contains(&(
                connection.from_node,
                connection.from_pin,
                connection.to_node,
                connection.to_pin,
            )));
            assert!(graph
                .input_connection_set
                .contains(&(connection.to_node, connection.to_pin)));
        }
    }
}

crate::tolerant_enum!(NodeType);
