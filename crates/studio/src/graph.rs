//! The automation graph: nodes on a sheet, wired from pin to pin.
//!
//! The graph is a part of the configuration like any other. Every frame
//! the page takes the graph out of the document, lets the pointer and the
//! keys change that copy, and writes it back when it changed. Undo, the
//! draft, Live and Apply are therefore the document's, with nothing of
//! their own here. A selected node's settings are drawn by the form, at
//! their place in the document.
//!
//! - [`geometry`]: where things are, and what is under a point;
//! - [`edit`]: what can be done to a graph (wires, removal, copy, paste);
//! - [`colours`]: the colours of pins, wires and nodes, from the palette.

mod colours;
mod edit;
mod geometry;
mod special;

use crate::document::{escape, Document};
use crate::form::Form;
use crate::link::{GraphEvent, Request};
use edit::{Clip, Verdict};
use egui::{
    pos2, vec2, Align2, Color32, FontId, PointerButton, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};
use geometry::{PinRef, View, WireRef, HEADER, NODE_WIDTH};
use gevrek_model::graph::{Node, NodeGraph, NodeId, NodeType, PinType};
use gevrek_schema::{Kind, VariantShape};
use gevrek_ui::theme::{self, icons, Palette};
use gevrek_ui::widgets;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Where the graph is in the configuration.
const GRAPH: &str = "/node_graph";
/// The width of the panel beside the sheet.
const SIDE: f32 = 340.0;
/// How long a node stays lit after it ran.
const LIT: Duration = Duration::from_millis(1600);
/// A node that failed stays lit longer: there is something to read.
const LIT_FAILED: Duration = Duration::from_secs(8);
/// A node that began and never said it ended is not lit for ever.
const LIT_RUNNING: Duration = Duration::from_secs(10);
const NOTICE: Duration = Duration::from_secs(4);

/// What the pointer is doing on the sheet between a press and a release.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Idle,
    /// Moving the sheet.
    Pan,
    /// Moving the selected nodes.
    Move,
    /// Pulling a rubber band from this point of the graph; `add` keeps
    /// what was selected before.
    Band {
        from: Pos2,
        add: bool,
    },
    /// Pulling a wire from a pin. `clicked`: the pin was clicked, and the
    /// wire follows the pointer until the next click.
    Wire {
        from: PinRef,
        clicked: bool,
    },
}

/// How a change is to be remembered.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Change {
    None,
    /// One of many in a row (a node being dragged): together one step.
    Smooth,
    /// A step of its own.
    Step,
}

impl Change {
    fn and(&mut self, other: Change) {
        if other > *self {
            *self = other;
        }
    }
}

/// Something asked for by a key, a button or a menu, done in one place.
enum Deed {
    Add(NodeType, Option<Pos2>),
    Delete,
    Copy,
    Cut,
    /// What the system's clipboard held, if it was nodes.
    Paste(Option<Clip>),
    Duplicate,
    SelectAll,
    Fit,
    RemoveWire(WireRef),
    Disconnect(PinRef),
    /// Open the list of nodes in the middle of the sheet.
    OpenList,
}

enum RunState {
    Running,
    Done,
    Failed(String),
}

struct Run {
    state: RunState,
    since: Instant,
}

/// The list of nodes, opened on the sheet.
struct Popup {
    /// Where the chosen node goes.
    at: Pos2,
    screen: Pos2,
    search: String,
    /// Not drawn yet: the search field takes the keyboard, and the click
    /// that opened it does not close it.
    fresh: bool,
}

pub struct GraphPage {
    view: View,
    /// The person chose the view (moved the sheet, zoomed). Until then the
    /// whole graph is kept in sight, whatever size the window takes.
    touched: bool,
    /// The size of the sheet the graph was last fitted into.
    fitted_into: Vec2,
    selected: BTreeSet<NodeId>,
    wire: Option<WireRef>,
    action: Action,
    search: String,
    popup: Option<Popup>,
    /// What was copied last in this window.
    clip: Option<Clip>,
    runs: HashMap<NodeId, Run>,
    paused: bool,
    /// Something to say on the sheet for a moment (why no wire was made).
    notice: Option<(String, Instant)>,
    /// What is wrong at a node (`NodeGraph::problems`), to say under it.
    problems: HashMap<NodeId, String>,
    /// The chosen nodes are to be brought into sight at the next frame.
    bring_into_sight: bool,
    /// The next key pressed becomes this key node's key.
    recording: Option<NodeId>,
    /// Super is held, by the keys seen while recording.
    super_down: bool,
    /// While recording, every key is the studio's (as for a binding):
    /// the modifiers held with the key are recorded too.
    pub every_key: bool,
    /// What was pressed, where the keys do not come through the toolkit.
    pub heard: Option<crate::bindings::Heard>,
    /// The open windows, as the window manager last said: what a node's
    /// pattern is tried on.
    pub windows: Vec<serde_json::Value>,
}

impl Default for GraphPage {
    fn default() -> Self {
        Self {
            view: View::default(),
            touched: false,
            fitted_into: Vec2::ZERO,
            selected: BTreeSet::new(),
            wire: None,
            action: Action::Idle,
            search: String::new(),
            popup: None,
            clip: None,
            runs: HashMap::new(),
            paused: false,
            notice: None,
            problems: HashMap::new(),
            bring_into_sight: false,
            recording: None,
            super_down: false,
            every_key: false,
            heard: None,
            windows: Vec::new(),
        }
    }
}

impl GraphPage {
    /// Whether the keys of a node are being waited for.
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Wait for the keys of a node that starts at a key, as its button
    /// does: the node is the one chosen, so that its settings are shown.
    pub fn record(&mut self, node: NodeId) {
        self.selected = BTreeSet::from([node]);
        self.recording = Some(node);
        self.super_down = false;
        self.heard = None;
    }

    /// Say how large the sheet is shown, when asked to (a check reads it).
    fn say_zoom(&self) {
        log::debug!("the graph is shown at {:.3}", self.view.zoom);
    }

    /// What the windows manager says is open (`special::open_windows`).
    pub fn take_state(&mut self, state: Option<&serde_json::Value>) {
        self.windows = state.map(special::open_windows).unwrap_or_default();
    }

    /// The keys of a node that starts at a key: read as a chord, and
    /// recorded by pressing them.
    fn keys_of(&mut self, ui: &mut Ui, palette: &Palette, document: &mut Document, node: &Node) {
        let NodeType::OnKeyPress {
            key,
            ctrl,
            alt,
            shift,
            win,
        } = &node.node_type
        else {
            if self.recording == Some(node.id) {
                self.recording = None;
            }
            return;
        };
        let place = format!("{GRAPH}/nodes/{}/node_type", node.id);
        let inside = if document.get(&format!("{place}/OnKeyPress")).is_some() {
            format!("{place}/OnKeyPress")
        } else {
            place
        };
        let recording = self.recording == Some(node.id);
        ui.horizontal(|ui| {
            widgets::kbd(ui, &special::chord(key, *ctrl, *alt, *shift, *win));
            let text = if recording {
                "Press the keys (Escape: leave it)"
            } else {
                "Press the keys"
            };
            if ui
                .add(egui::Button::new(text).selected(recording))
                .clicked()
            {
                self.recording = (!recording).then_some(node.id);
                self.super_down = false;
                self.heard = None;
            }
        });
        if !recording {
            return;
        }
        let heard = self
            .heard
            .take()
            .or_else(|| crate::bindings::heard_now(ui, &mut self.super_down));
        if let Some(heard) = heard {
            if let crate::bindings::Heard::Key(name, modifiers) = heard {
                log::info!(
                    "recorded for node {}: {name} with {modifiers:?}",
                    node.id
                );
                document.set(&format!("{inside}/key"), serde_json::Value::String(name));
                // With every key the studio's, the modifiers that are
                // held are the node's too.
                if self.every_key {
                    for (field, on) in modifiers {
                        document.set(&format!("{inside}/{field}"), serde_json::Value::Bool(on));
                    }
                }
            }
            self.recording = None;
            self.super_down = false;
        } else if !self.every_key {
            ui.label(
                RichText::new("Only the key is recorded here; set what is held with it below.")
                    .size(12.0)
                    .color(palette.faint),
            );
        }
    }

    /// The open windows the pattern of a node holds for now.
    fn windows_of(&self, ui: &mut Ui, palette: &Palette, node: &Node) {
        let Some((kind, strategy, pattern)) = special::pattern(&node.node_type) else {
            return;
        };
        let Some(held) = special::held_for(kind, strategy, pattern, &self.windows) else {
            return;
        };
        let text = match held.len() {
            0 if self.windows.is_empty() => return,
            0 => "Holds for none of the open windows.".to_string(),
            1 => format!("Holds for one open window: {}", held[0]),
            many => format!(
                "Holds for {many} open windows: {}{}",
                held.iter().take(4).cloned().collect::<Vec<_>>().join(", "),
                if many > 4 { ", ..." } else { "" }
            ),
        };
        ui.add(
            egui::Label::new(RichText::new(text).size(12.0).color(if held.is_empty() {
                palette.warn
            } else {
                palette.muted
            }))
            .wrap(),
        );
    }

    /// Choose these nodes and no others (the ones something is wrong at,
    /// when the Problems page sends the person here).
    pub fn select_only(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        self.selected = nodes.into_iter().collect();
        self.wire = None;
        // They may lie anywhere on a large sheet.
        self.bring_into_sight = !self.selected.is_empty();
    }

    /// The nodes that are chosen.
    pub fn selected(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.selected.iter().copied()
    }

    /// What the running graph did: its nodes light up on the sheet.
    pub fn take_in(&mut self, event: GraphEvent) {
        let now = Instant::now();
        match event {
            GraphEvent::Started(node) => {
                self.runs.insert(
                    node,
                    Run {
                        state: RunState::Running,
                        since: now,
                    },
                );
            }
            GraphEvent::Completed(node) => {
                self.runs.insert(
                    node,
                    Run {
                        state: RunState::Done,
                        since: now,
                    },
                );
            }
            GraphEvent::Failed(node, error) => {
                self.runs.insert(
                    node,
                    Run {
                        state: RunState::Failed(error),
                        since: now,
                    },
                );
            }
            GraphEvent::Paused(paused) => self.paused = paused,
        }
    }

    /// Draw the page. `features` is what the connected window manager
    /// says it can do (`None`: there is none). Returns what to ask of it.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        features: Option<&[String]>,
    ) -> Vec<Request> {
        let palette = theme::pal(ui.ctx());
        let mut requests = Vec::new();
        let Some(mut graph) = working_copy(document) else {
            ui.label(
                RichText::new(
                    "The graph is shown again when the configuration is one gevrek reads.",
                )
                .color(palette.muted),
            );
            return requests;
        };
        // What is wrong, by the node to look at; several things at one
        // node are said one after the other.
        self.problems.clear();
        for problem in graph.problems() {
            if let Some(node) = problem.node {
                let said = self.problems.entry(node).or_default();
                if !said.is_empty() {
                    said.push_str("  ");
                }
                said.push_str(&problem.text);
            }
        }
        // An undo may have taken away what was selected.
        self.selected.retain(|id| graph.nodes.contains_key(id));
        if self.wire.is_some_and(|wire| !has_wire(&graph, wire)) {
            self.wire = None;
        }
        self.runs
            .retain(|_, run| run.since.elapsed() < lit_for(&run.state));

        let mut deeds = Vec::new();
        self.toolbar(
            ui,
            &palette,
            &graph,
            features.is_some(),
            &mut deeds,
            &mut requests,
        );

        // The sheet and the panel beside it take the rest of the window.
        let rest = ui.available_rect_before_wrap();
        let bottom = rest
            .bottom()
            .min(ui.ctx().content_rect().bottom() - 10.0)
            .max(rest.top() + 260.0);
        let rest = Rect::from_min_max(rest.min, pos2(rest.right(), bottom));
        ui.allocate_rect(rest, Sense::hover());
        let side = Rect::from_min_max(
            pos2((rest.right() - SIDE).max(rest.left() + 200.0), rest.top()),
            rest.max,
        );
        let canvas = Rect::from_min_max(rest.min, pos2(side.left() - 12.0, rest.bottom()));

        let mut change = self.sheet_input(ui, canvas, &mut graph, &mut deeds);
        // Asked to show the chosen nodes: the sheet goes to them if they
        // are not in sight (and then stays where it was put).
        if std::mem::take(&mut self.bring_into_sight) {
            if let Some(chosen) = geometry::bounds(&graph, Some(&self.selected)) {
                if self.view.bring_into_sight(chosen, canvas.size()) {
                    self.touched = true;
                }
            }
        }
        for deed in deeds.drain(..) {
            change.and(self.perform(ui.ctx(), deed, &mut graph, canvas));
        }
        commit(document, &graph, change);
        self.sheet_draw(ui, &palette, canvas, &graph);

        // The panel changes the document itself (the form), so what it
        // asks for is done to the graph as the document then has it.
        ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| {
            self.side(ui, &palette, document, features, &mut deeds, &mut requests);
        });
        self.popup_list(ui.ctx(), &palette, features, &mut deeds);
        if !deeds.is_empty() {
            if let Some(mut graph) = working_copy(document) {
                let mut change = Change::None;
                for deed in deeds {
                    change.and(self.perform(ui.ctx(), deed, &mut graph, canvas));
                }
                commit(document, &graph, change);
            }
        }
        requests
    }

    fn toolbar(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        graph: &NodeGraph,
        connected: bool,
        deeds: &mut Vec<Deed>,
        requests: &mut Vec<Request>,
    ) {
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, format!("{}  Add a node", icons::PLUS)).on_hover_text("Also: right-click on the sheet").clicked() {
                deeds.push(Deed::OpenList);
            }
            if widgets::secondary_button(ui, format!("{}  Fit", icons::CORNERS_OUT)).on_hover_text("Show the whole graph (F)").clicked() {
                deeds.push(Deed::Fit);
            }
            ui.separator();
            let some = !self.selected.is_empty();
            ui.add_enabled_ui(some, |ui| {
                if widgets::icon_button(ui, icons::COPY, "Copy the selected nodes, with the wires between them (Ctrl+C)").clicked() {
                    deeds.push(Deed::Copy);
                }
            });
            ui.add_enabled_ui(self.clip.is_some(), |ui| {
                if widgets::icon_button(ui, icons::CLIPBOARD, "Paste (Ctrl+V)").clicked() {
                    deeds.push(Deed::Paste(None));
                }
            });
            ui.add_enabled_ui(some, |ui| {
                if widgets::icon_button(ui, icons::COPY_SIMPLE, "Duplicate (Ctrl+D)").clicked() {
                    deeds.push(Deed::Duplicate);
                }
            });
            ui.add_enabled_ui(some || self.wire.is_some(), |ui| {
                if widgets::icon_button(ui, icons::TRASH, "Remove what is selected (Delete)").clicked() {
                    deeds.push(Deed::Delete);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if connected {
                    if self.paused {
                        if widgets::secondary_button(ui, format!("{}  Let it run", icons::PLAY)).on_hover_text("The graph runs again").clicked() {
                            requests.push(Request::Pause(false));
                        }
                        widgets::pill(ui, icons::PAUSE, "paused", palette.warn)
                            .on_hover_text("The window manager does not run the graph");
                    } else {
                        if ui
                            .button(format!("{}  Pause", icons::PAUSE))
                            .on_hover_text("Keep the window manager from running the graph, until you let it run again")
                            .clicked()
                        {
                            requests.push(Request::Pause(true));
                        }
                    }
                }
                let nodes = graph.nodes.len();
                let wires = graph.connections.len();
                ui.label(
                    RichText::new(format!(
                        "{nodes} {}, {wires} {}",
                        if nodes == 1 { "node" } else { "nodes" },
                        if wires == 1 { "wire" } else { "wires" }
                    ))
                    .color(palette.muted),
                );
            });
        });
        ui.add_space(6.0);
    }

    // --- the sheet: what the pointer and the keys do ---------------------

    fn sheet_input(
        &mut self,
        ui: &mut Ui,
        canvas: Rect,
        graph: &mut NodeGraph,
        deeds: &mut Vec<Deed>,
    ) -> Change {
        let mut change = Change::None;
        self.view.origin = canvas.min;
        if !self.touched
            && !graph.nodes.is_empty()
            && canvas.width() > 100.0
            && canvas.size() != self.fitted_into
        {
            self.fit(graph, canvas);
        }
        let response = ui.interact(canvas, ui.scope_id().with("sheet"), Sense::click_and_drag());
        let (latest, origin, shift, space, pressed) = ui.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.press_origin(),
                input.modifiers.shift,
                input.key_down(egui::Key::Space),
                input.pointer.any_pressed(),
            )
        });
        let reach = 12.0 / self.view.zoom.clamp(0.5, 1.5);
        let pile = geometry::pile(graph, &self.selected);

        if response.is_pointer_button_down_on() && pressed {
            // A press on the sheet ends typing elsewhere: the keys are the
            // sheet's again.
            if let Some(focused) = ui.memory(|memory| memory.focused()) {
                ui.memory_mut(|memory| memory.surrender_focus(focused));
            }
            self.popup = None;
        }

        // The wheel (and a pinch) zooms at the pointer.
        if let Some(at) = response.hover_pos() {
            let (scroll, pinch) =
                ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
            let factor = pinch * (scroll * 0.0025).exp();
            if (factor - 1.0).abs() > 0.0001 {
                self.view.zoom_at(at, factor);
                self.touched = true;
                self.say_zoom();
            }
        }

        if response.drag_started() {
            let start = self
                .view
                .to_graph(origin.or(latest).unwrap_or(canvas.center()));
            let primary = response.drag_started_by(PointerButton::Primary) && !space;
            self.action = if !primary {
                Action::Pan
            } else if let Some(pin) = geometry::pin_at(graph, start, reach) {
                Action::Wire {
                    from: pin,
                    clicked: false,
                }
            } else if let Some(node) = geometry::node_at(graph, &pile, start) {
                if !self.selected.contains(&node) {
                    if !shift {
                        self.selected.clear();
                    }
                    self.selected.insert(node);
                }
                self.wire = None;
                Action::Move
            } else {
                Action::Band {
                    from: start,
                    add: shift,
                }
            };
        }
        if response.dragged() {
            // A drag is known as one only after a few points: on its
            // first frame it has come all the way from the press.
            let moved = match (response.drag_started(), origin, latest) {
                (true, Some(origin), Some(latest)) => latest - origin,
                _ => response.drag_delta(),
            };
            match self.action {
                Action::Pan => {
                    self.view.pan += moved;
                    self.touched = true;
                }
                Action::Move if moved != Vec2::ZERO => {
                    for id in &self.selected {
                        if let Some(node) = graph.nodes.get_mut(id) {
                            node.position[0] += moved.x / self.view.zoom;
                            node.position[1] += moved.y / self.view.zoom;
                        }
                    }
                    change.and(Change::Smooth);
                }
                _ => {}
            }
        }
        if response.drag_stopped() {
            let end = latest.map(|latest| self.view.to_graph(latest));
            match (self.action, end) {
                (Action::Band { from, add }, Some(end)) => {
                    let touched = geometry::nodes_in(graph, Rect::from_two_pos(from, end));
                    if add {
                        self.selected.extend(touched);
                    } else {
                        self.selected = touched;
                    }
                    self.wire = None;
                }
                (Action::Wire { from, .. }, Some(end)) => {
                    if let Some(to) = geometry::pin_at(graph, end, reach).filter(|to| *to != from) {
                        change.and(self.wire_up(graph, from, to));
                    }
                }
                _ => {}
            }
            self.action = Action::Idle;
        }

        if response.clicked() {
            let at = response
                .interact_pointer_pos()
                .map(|at| self.view.to_graph(at));
            let pin = at.and_then(|at| geometry::pin_at(graph, at, reach));
            let node = at.and_then(|at| geometry::node_at(graph, &pile, at));
            match (self.action, pin) {
                // The second click of a wire made by two clicks.
                (
                    Action::Wire {
                        from,
                        clicked: true,
                    },
                    Some(to),
                ) => {
                    if to != from {
                        change.and(self.wire_up(graph, from, to));
                    }
                    self.action = Action::Idle;
                }
                (Action::Wire { clicked: true, .. }, None) => self.action = Action::Idle,
                (_, Some(pin)) => {
                    self.action = Action::Wire {
                        from: pin,
                        clicked: true,
                    }
                }
                (_, None) => {
                    if let Some(node) = node {
                        if shift {
                            if !self.selected.remove(&node) {
                                self.selected.insert(node);
                            }
                        } else {
                            self.selected = BTreeSet::from([node]);
                        }
                        self.wire = None;
                    } else {
                        if !shift {
                            self.selected.clear();
                        }
                        self.wire =
                            at.and_then(|at| geometry::wire_at(graph, at, 7.0 / self.view.zoom));
                    }
                }
            }
        }
        if response.secondary_clicked() {
            if let Some(screen) = response.interact_pointer_pos() {
                let at = self.view.to_graph(screen);
                if let Some(pin) = geometry::pin_at(graph, at, reach) {
                    deeds.push(Deed::Disconnect(pin));
                } else if let Some(node) = geometry::node_at(graph, &pile, at) {
                    if !self.selected.contains(&node) {
                        self.selected = BTreeSet::from([node]);
                    }
                } else if let Some(wire) = geometry::wire_at(graph, at, 7.0 / self.view.zoom) {
                    deeds.push(Deed::RemoveWire(wire));
                } else {
                    self.open_list(screen);
                }
            }
        }
        if response.double_clicked() {
            if let Some(screen) = response.interact_pointer_pos() {
                let at = self.view.to_graph(screen);
                if geometry::pin_at(graph, at, reach).is_none()
                    && geometry::node_at(graph, &pile, at).is_none()
                {
                    self.action = Action::Idle;
                    self.open_list(screen);
                }
            }
        }

        // The pointer says what a press would do.
        if let Some(at) = response.hover_pos() {
            let at = self.view.to_graph(at);
            let icon = match self.action {
                Action::Pan => Some(egui::CursorIcon::Grabbing),
                Action::Move => Some(egui::CursorIcon::Grabbing),
                Action::Wire { .. } => Some(egui::CursorIcon::Crosshair),
                Action::Band { .. } => None,
                Action::Idle if geometry::pin_at(graph, at, reach).is_some() => {
                    Some(egui::CursorIcon::Crosshair)
                }
                Action::Idle if space => Some(egui::CursorIcon::Grab),
                Action::Idle => None,
            };
            if let Some(icon) = icon {
                ui.ctx().set_cursor_icon(icon);
            }
        }

        // The keys are the sheet's while nothing is being typed.
        if ui.memory(|memory| memory.focused().is_none()) {
            let mut zoom = None;
            ui.input(|input| {
                for event in &input.events {
                    match event {
                        egui::Event::Copy => deeds.push(Deed::Copy),
                        egui::Event::Cut => deeds.push(Deed::Cut),
                        egui::Event::Paste(text) => deeds.push(Deed::Paste(Clip::from_text(text))),
                        _ => {}
                    }
                }
                let plain = input.modifiers.is_none();
                if input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace) {
                    deeds.push(Deed::Delete);
                }
                if input.modifiers.command && input.key_pressed(egui::Key::A) {
                    deeds.push(Deed::SelectAll);
                }
                if input.modifiers.command && input.key_pressed(egui::Key::D) {
                    deeds.push(Deed::Duplicate);
                }
                if plain && input.key_pressed(egui::Key::F) {
                    deeds.push(Deed::Fit);
                }
                // Plus and minus zoom like the wheel; 0 is life size.
                if plain || input.modifiers.shift_only() {
                    if input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals) {
                        zoom = Some(1.25);
                    } else if input.key_pressed(egui::Key::Minus) {
                        zoom = Some(0.8);
                    } else if plain && input.key_pressed(egui::Key::Num0) {
                        zoom = Some(1.0 / self.view.zoom);
                    }
                }
                if input.key_pressed(egui::Key::Escape) {
                    // First what is in progress, then the selection.
                    if self.action != Action::Idle {
                        self.action = Action::Idle;
                    } else {
                        self.selected.clear();
                        self.wire = None;
                    }
                }
            });
            if let Some(factor) = zoom {
                self.view
                    .zoom_at(response.hover_pos().unwrap_or(canvas.center()), factor);
                self.touched = true;
                self.say_zoom();
            }
        }
        change
    }

    /// Wire two pins, or say why not.
    fn wire_up(&mut self, graph: &mut NodeGraph, from: PinRef, to: PinRef) -> Change {
        match edit::verdict(graph, from, to) {
            Verdict::Refuse(why) => {
                self.notice = Some((why.to_string(), Instant::now()));
                Change::None
            }
            said => {
                if !edit::connect(graph, from, to) {
                    return Change::None;
                }
                if said == Verdict::Convert {
                    self.notice = Some((
                        "A node that converts one into the other was put in between.".to_string(),
                        Instant::now(),
                    ));
                }
                Change::Step
            }
        }
    }

    fn open_list(&mut self, screen: Pos2) {
        self.popup = Some(Popup {
            at: self.view.to_graph(screen),
            screen,
            search: String::new(),
            fresh: true,
        });
    }

    fn fit(&mut self, graph: &NodeGraph, canvas: Rect) {
        if let Some(bounds) = geometry::bounds(graph, None) {
            self.view = View::fit(canvas.min, bounds, canvas.size());
        }
        // Asked for, it holds again until the person moves the sheet.
        self.touched = false;
        self.fitted_into = canvas.size();
    }

    fn perform(
        &mut self,
        context: &egui::Context,
        deed: Deed,
        graph: &mut NodeGraph,
        canvas: Rect,
    ) -> Change {
        let pointer = context
            .input(|input| input.pointer.latest_pos())
            .filter(|at| canvas.contains(*at));
        match deed {
            Deed::Add(node_type, at) => {
                // Without a place of its own, in the middle of the sheet,
                // and not exactly on a node that is there already.
                let mut at = at.unwrap_or_else(|| {
                    self.view.to_graph(canvas.center()) - vec2(NODE_WIDTH / 2.0, HEADER)
                });
                while graph.nodes.values().any(|node| {
                    (node.position[0] - at.x).abs() < 8.0 && (node.position[1] - at.y).abs() < 8.0
                }) {
                    at += vec2(24.0, 24.0);
                }
                let id = graph.add_node(node_type, [at.x, at.y]);
                self.selected = BTreeSet::from([id]);
                self.wire = None;
                // The sheet stays where it is: the new node is not to jump.
                self.touched = true;
                Change::Step
            }
            Deed::Delete => {
                let mut removed = edit::remove_nodes(graph, &self.selected) > 0;
                if let Some(wire) = self.wire.take() {
                    removed |= edit::remove_wire(graph, wire);
                }
                self.selected.clear();
                if removed {
                    Change::Step
                } else {
                    Change::None
                }
            }
            Deed::Copy | Deed::Cut => {
                if self.selected.is_empty() {
                    return Change::None;
                }
                let clip = edit::copy(graph, &self.selected);
                context.copy_text(clip.to_text());
                self.clip = Some(clip);
                if matches!(deed, Deed::Cut) {
                    edit::remove_nodes(graph, &self.selected);
                    self.selected.clear();
                    return Change::Step;
                }
                Change::None
            }
            Deed::Paste(from_system) => {
                let Some(clip) = from_system.or_else(|| self.clip.clone()) else {
                    return Change::None;
                };
                // Under the pointer when it is on the sheet, else a little
                // down and right of where it was copied.
                let corner = clip.corner();
                let by = match pointer {
                    Some(pointer) => {
                        let at = self.view.to_graph(pointer);
                        [
                            at.x - corner[0] - NODE_WIDTH / 2.0,
                            at.y - corner[1] - HEADER / 2.0,
                        ]
                    }
                    None => [40.0, 40.0],
                };
                self.selected = edit::paste(graph, &clip, by).into_iter().collect();
                self.wire = None;
                Change::Step
            }
            Deed::Duplicate => {
                if self.selected.is_empty() {
                    return Change::None;
                }
                let clip = edit::copy(graph, &self.selected);
                self.selected = edit::paste(graph, &clip, [40.0, 40.0])
                    .into_iter()
                    .collect();
                Change::Step
            }
            Deed::SelectAll => {
                self.selected = graph.nodes.keys().copied().collect();
                Change::None
            }
            Deed::Fit => {
                self.fit(graph, canvas);
                Change::None
            }
            Deed::RemoveWire(wire) => {
                if self.wire == Some(wire) {
                    self.wire = None;
                }
                if edit::remove_wire(graph, wire) {
                    Change::Step
                } else {
                    Change::None
                }
            }
            Deed::Disconnect(pin) => {
                if edit::disconnect(graph, pin) > 0 {
                    Change::Step
                } else {
                    Change::None
                }
            }
            Deed::OpenList => {
                self.open_list(canvas.center() - vec2(150.0, 190.0));
                if let Some(popup) = &mut self.popup {
                    popup.at = self.view.to_graph(canvas.center()) - vec2(NODE_WIDTH / 2.0, HEADER);
                }
                Change::None
            }
        }
    }

    // --- the sheet: how it is drawn -------------------------------------

    fn sheet_draw(&mut self, ui: &mut Ui, palette: &Palette, canvas: Rect, graph: &NodeGraph) {
        let painter = ui.painter_at(canvas);
        let view = self.view;
        let zoom = view.zoom;
        let sheet = colours::sheet(palette);
        let radius = ui.visuals().window_corner_radius;
        painter.rect_filled(canvas, radius, sheet);
        self.dots(&painter, palette, canvas);

        let pointer = ui
            .input(|input| input.pointer.latest_pos())
            .filter(|at| canvas.contains(*at));
        let pointer_graph = pointer.map(|at| view.to_graph(at));
        let reach = 12.0 / zoom.clamp(0.5, 1.5);
        let pulling = match self.action {
            Action::Wire { from, .. } => Some(from),
            _ => None,
        };
        let idle = self.action == Action::Idle;
        let hot_pin = pointer_graph.and_then(|at| geometry::pin_at(graph, at, reach));
        let hot_wire = pointer_graph
            .filter(|_| idle && hot_pin.is_none())
            .filter(|at| {
                geometry::node_at(graph, &geometry::pile(graph, &self.selected), *at).is_none()
            })
            .and_then(|at| geometry::wire_at(graph, at, 7.0 / zoom));

        // Wires first: they run under the nodes.
        let width = zoom.clamp(0.6, 1.4);
        let mut wired: HashSet<PinRef> = HashSet::new();
        for connection in &graph.connections {
            let wire = WireRef {
                from_node: connection.from_node,
                from_pin: connection.from_pin,
                to_node: connection.to_node,
                to_pin: connection.to_pin,
            };
            wired.insert(PinRef {
                node: wire.from_node,
                pin: wire.from_pin,
                output: true,
            });
            wired.insert(PinRef {
                node: wire.to_node,
                pin: wire.to_pin,
                output: false,
            });
            let Some((from, to)) = geometry::wire_ends(graph, wire) else {
                continue;
            };
            if !view
                .rect_to_screen(Rect::from_two_pos(from, to).expand(80.0))
                .intersects(canvas)
            {
                continue;
            }
            let kind = pin_kind(
                graph,
                PinRef {
                    node: wire.from_node,
                    pin: wire.from_pin,
                    output: true,
                },
            );
            let colour = kind.map_or(palette.muted, |kind| colours::pin(palette, kind));
            // The order of things is a little stronger than what is carried.
            let base = if kind == Some(&PinType::Execution) {
                2.6
            } else {
                2.0
            } * width;
            let (thickness, colour) = if self.wire == Some(wire) {
                (base + 1.8, palette.accent_fg())
            } else if hot_wire == Some(wire) {
                (base + 1.0, theme::mix(colour, palette.text, 0.4))
            } else {
                (base, colour)
            };
            let points: Vec<Pos2> = geometry::wire_path(from, to)
                .into_iter()
                .map(|point| view.to_screen(point))
                .collect();
            // A line of the sheet's colour under the wire keeps it apart
            // from the wires it crosses.
            painter.add(egui::Shape::line(
                points.clone(),
                Stroke::new(thickness + 3.0, sheet.gamma_multiply(0.75)),
            ));
            painter.add(egui::Shape::line(points, Stroke::new(thickness, colour)));
        }

        // Nodes, from the bottom of the pile.
        for id in geometry::pile(graph, &self.selected) {
            let node = &graph.nodes[&id];
            let rect = view.rect_to_screen(geometry::node_rect(node));
            if !rect.expand(24.0).intersects(canvas) {
                continue;
            }
            self.node(
                &painter, palette, graph, node, rect, &wired, hot_pin, pulling,
            );
        }

        // The wire being pulled, on top of everything.
        if let (Some(from), Some(pointer)) = (pulling, pointer_graph) {
            if let (Some(start), Some(kind)) =
                (geometry::pin_place(graph, from), pin_kind(graph, from))
            {
                let target = hot_pin.filter(|target| *target != from);
                let said = target.map(|target| edit::verdict(graph, from, target));
                let end = match (target, said) {
                    (Some(target), Some(said)) if said.makes_a_wire() => {
                        geometry::pin_place(graph, target).unwrap_or(pointer)
                    }
                    _ => pointer,
                };
                let refused = matches!(said, Some(Verdict::Refuse(_)));
                let colour = if refused {
                    palette.error
                } else {
                    colours::pin(palette, kind)
                };
                let side = if from.output { 1.0 } else { -1.0 };
                let points: Vec<Pos2> = geometry::wire_path_sided(start, end, side)
                    .into_iter()
                    .map(|point| view.to_screen(point))
                    .collect();
                painter.extend(egui::Shape::dashed_line(
                    &points,
                    Stroke::new(2.2 * width, colour),
                    8.0,
                    5.0,
                ));
                painter.circle_filled(view.to_screen(end), 4.0 * width, colour);
                if let Some(Verdict::Refuse(why)) = said {
                    self.notice = Some((why.to_string(), Instant::now()));
                }
            }
        }

        // The rubber band.
        if let (Action::Band { from, .. }, Some(to)) = (self.action, pointer_graph) {
            let band = view.rect_to_screen(Rect::from_two_pos(from, to));
            painter.rect_filled(band, 2.0, palette.accent.gamma_multiply(0.14));
            painter.rect_stroke(
                band,
                2.0,
                Stroke::new(1.0, palette.accent_fg()),
                StrokeKind::Inside,
            );
        }

        painter.rect_stroke(
            canvas,
            radius,
            Stroke::new(1.0, palette.border),
            StrokeKind::Inside,
        );

        // Words on the sheet: what can be done, and how far it is zoomed.
        let small = FontId::proportional(12.0);
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, since)| since.elapsed() < NOTICE)
            .map(|(text, _)| text.clone());
        let (hint, hint_colour) = match (&notice, self.action) {
            (Some(notice), _) => (notice.as_str(), theme::readable(palette.warn, sheet, 4.5)),
            (None, Action::Wire { clicked: true, .. }) => ("Click the pin the wire goes to. Escape gives it up.", palette.muted),
            (None, Action::Wire { .. }) => ("Let go on the pin the wire goes to.", palette.muted),
            (None, _) => (
                "Drag from pin to pin for a wire. Right-click for the list of nodes. The wheel, or + and -, zooms; the middle button, or Space and a drag, moves the sheet.",
                palette.muted,
            ),
        };
        // On a ground of their own: a node may lie under them.
        let foot = Rect::from_min_max(
            pos2(canvas.left() + 1.0, canvas.bottom() - 30.0),
            canvas.right_bottom() - vec2(1.0, 1.0),
        );
        painter.rect_filled(
            foot,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: radius.sw,
                se: radius.se,
            },
            sheet.gamma_multiply(0.86),
        );
        painter.text(
            canvas.left_bottom() + vec2(12.0, -9.0),
            Align2::LEFT_BOTTOM,
            hint,
            small.clone(),
            hint_colour,
        );
        painter.text(
            canvas.right_bottom() + vec2(-12.0, -9.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} %", zoom * 100.0),
            small,
            palette.muted,
        );
        if graph.nodes.is_empty() {
            painter.text(
                canvas.center(),
                Align2::CENTER_CENTER,
                "No nodes yet. Right-click here, or choose one from the list on the right.",
                FontId::proportional(14.0),
                palette.muted,
            );
        }
        if self.notice.is_some() && notice.is_none() {
            self.notice = None;
        }
        // Lit nodes fade, and a notice goes away, without the pointer moving.
        if !self.runs.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        } else if notice.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Dots, every fifth one stronger. Zoomed far out only the strong
    /// ones are drawn: the others would be a haze.
    fn dots(&self, painter: &egui::Painter, palette: &Palette, canvas: Rect) {
        let mut step = 24.0 * self.view.zoom;
        let mut strong_every = 5;
        if step < 14.0 {
            step *= 5.0;
            strong_every = 1;
        }
        let weak = theme::mix(colours::sheet(palette), palette.text, 0.10);
        let strong = theme::mix(colours::sheet(palette), palette.text, 0.22);
        let start = self.view.origin + self.view.pan;
        let first_column = ((canvas.left() - start.x) / step).floor() as i64;
        let first_row = ((canvas.top() - start.y) / step).floor() as i64;
        let columns = (canvas.width() / step).ceil() as i64 + 1;
        let rows = (canvas.height() / step).ceil() as i64 + 1;
        for row in first_row..=first_row + rows {
            for column in first_column..=first_column + columns {
                let at = pos2(start.x + column as f32 * step, start.y + row as f32 * step);
                let is_strong =
                    column.rem_euclid(strong_every) == 0 && row.rem_euclid(strong_every) == 0;
                painter.circle_filled(
                    at,
                    if is_strong { 1.5 } else { 1.0 },
                    if is_strong { strong } else { weak },
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &self,
        painter: &egui::Painter,
        palette: &Palette,
        graph: &NodeGraph,
        node: &Node,
        rect: Rect,
        wired: &HashSet<PinRef>,
        hot_pin: Option<PinRef>,
        pulling: Option<PinRef>,
    ) {
        let zoom = self.view.zoom;
        let selected = self.selected.contains(&node.id);
        let group = colours::group(palette, node.node_type.category());
        let body = if selected {
            colours::body_selected(palette)
        } else {
            colours::body(palette)
        };
        let radius = (8.0 * zoom.min(1.5)).round();
        let corner = radius.clamp(0.0, 255.0) as u8;

        // Lit while it runs and for a moment after.
        let lit = self.runs.get(&node.id).map(|run| {
            let age = run.since.elapsed().as_secs_f32() / lit_for(&run.state).as_secs_f32();
            match &run.state {
                RunState::Running => (palette.accent_fg(), 1.0),
                RunState::Done => (
                    theme::readable(palette.ok, colours::sheet(palette), 3.0),
                    (1.0 - age).clamp(0.0, 1.0),
                ),
                // Strong for most of its time, then it goes.
                RunState::Failed(_) => (
                    theme::readable(palette.error, colours::sheet(palette), 3.0),
                    ((1.0 - age) * 4.0).clamp(0.0, 1.0),
                ),
            }
        });
        if let Some((colour, strength)) = lit {
            for (grow, alpha) in [(9.0, 0.10), (6.0, 0.18), (3.0, 0.32)] {
                painter.rect_filled(
                    rect.expand(grow * zoom),
                    radius + grow * zoom,
                    colour.gamma_multiply(alpha * strength),
                );
            }
        } else {
            // A soft shadow, so that the node lies on the sheet.
            let shadow = if palette.dark { 1.0 } else { 0.35 };
            for (grow, alpha) in [(6.0, 14.0), (3.0, 24.0), (1.0, 40.0)] {
                painter.rect_filled(
                    rect.expand(grow * zoom).translate(vec2(0.0, 3.0 * zoom)),
                    radius + grow * zoom,
                    Color32::from_black_alpha((alpha * shadow) as u8),
                );
            }
        }
        painter.rect_filled(rect, radius, body);
        // The header has some of the group's colour, and a line of it at the top.
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * zoom));
        painter.rect_filled(
            header,
            egui::CornerRadius {
                nw: corner,
                ne: corner,
                sw: 0,
                se: 0,
            },
            if selected {
                theme::mix(colours::header(palette, group), palette.accent, 0.10)
            } else {
                colours::header(palette, group)
            },
        );
        painter.rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + radius, rect.top()),
                vec2(rect.width() - radius * 2.0, (2.0 * zoom).max(1.5)),
            ),
            0.0,
            group,
        );
        let outline = match lit {
            Some((colour, strength)) if strength > 0.05 => Stroke::new(
                1.0 + 1.2 * strength,
                theme::mix(palette.border_strong(), colour, strength),
            ),
            _ if selected => Stroke::new(1.8, palette.accent_fg()),
            _ => Stroke::new(1.0, palette.border_strong()),
        };
        painter.rect_stroke(rect, radius, outline, StrokeKind::Inside);

        // The name on the left, what the node is set to on the right.
        if zoom >= 0.35 {
            let pad = 10.0 * zoom;
            let title = painter.layout_no_wrap(
                node.node_type.display_name().to_string(),
                FontId::proportional(13.5 * zoom),
                palette.text,
            );
            let title_width = title.size().x;
            painter.galley(
                pos2(
                    header.left() + pad,
                    header.center().y - title.size().y / 2.0,
                ),
                title,
                palette.text,
            );
            let room = rect.width() - title_width - pad * 3.0;
            if let Some(summary) = summary(&node.node_type).filter(|_| room > 24.0 * zoom) {
                let mut job = egui::text::LayoutJob::single_section(
                    summary,
                    egui::TextFormat {
                        font_id: FontId::monospace(11.5 * zoom),
                        color: palette.muted,
                        ..Default::default()
                    },
                );
                job.wrap = egui::text::TextWrapping {
                    max_width: room,
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let summary = painter.layout_job(job);
                painter.galley(
                    pos2(
                        header.right() - pad - summary.size().x,
                        header.center().y - summary.size().y / 2.0,
                    ),
                    summary,
                    palette.muted,
                );
            }
        }

        for (output, pins) in [(false, &node.inputs), (true, &node.outputs)] {
            for (index, pin) in pins.iter().enumerate() {
                let this = PinRef {
                    node: node.id,
                    pin: pin.id,
                    output,
                };
                let at = self.view.to_screen(geometry::pin_pos(node, index, output));
                let mut colour = colours::pin(palette, &pin.pin_type);
                let hot = hot_pin == Some(this) || pulling == Some(this);
                // While a wire is pulled, the pins it cannot go to step back.
                let mut out_of_reach = false;
                if let Some(from) = pulling.filter(|from| *from != this) {
                    if !edit::verdict(graph, from, this).makes_a_wire() {
                        out_of_reach = true;
                        colour = theme::mix(body, colour, 0.35);
                    }
                }
                let size = 5.5 * zoom * if hot && !out_of_reach { 1.3 } else { 1.0 };
                // A pin with a wire is filled; one without is a ring.
                let fill = if wired.contains(&this) || (hot && !out_of_reach) {
                    colour
                } else {
                    body
                };
                let stroke = Stroke::new((1.6 * zoom).clamp(1.0, 2.2), colour);
                if pin.pin_type == PinType::Execution {
                    // The order of things: an arrow.
                    let points = vec![
                        pos2(at.x - size * 0.8, at.y - size),
                        pos2(at.x + size, at.y),
                        pos2(at.x - size * 0.8, at.y + size),
                    ];
                    painter.add(egui::Shape::convex_polygon(points, fill, stroke));
                } else if colours::is_list(&pin.pin_type) {
                    // Many of a thing: a square.
                    let square = Rect::from_center_size(at, Vec2::splat(size * 1.8));
                    painter.rect_filled(square, 1.5 * zoom, fill);
                    painter.rect_stroke(square, 1.5 * zoom, stroke, StrokeKind::Inside);
                } else {
                    painter.circle(at, size, fill, stroke);
                }
                if zoom >= 0.45 {
                    let (text_at, anchor) = if output {
                        (pos2(at.x - 11.0 * zoom, at.y), Align2::RIGHT_CENTER)
                    } else {
                        (pos2(at.x + 11.0 * zoom, at.y), Align2::LEFT_CENTER)
                    };
                    let text = if out_of_reach {
                        palette.faint
                    } else if hot {
                        palette.text
                    } else {
                        palette.muted
                    };
                    painter.text(
                        text_at,
                        anchor,
                        &pin.name,
                        FontId::proportional(12.0 * zoom),
                        text,
                    );
                }
            }
        }

        // Why it failed, under the node, while it is lit.
        if let Some(Run {
            state: RunState::Failed(error),
            ..
        }) = self.runs.get(&node.id)
        {
            painter.text(
                rect.left_bottom() + vec2(2.0, 6.0),
                Align2::LEFT_TOP,
                widgets::ellipsize(error, 420.0),
                FontId::proportional(12.0),
                theme::readable(palette.error, colours::sheet(palette), 4.5),
            );
        }
        // What is wrong with its wires, as long as it is so.
        if let Some(problem) = self.problems.get(&node.id) {
            let below = if self.runs.contains_key(&node.id) {
                24.0
            } else {
                6.0
            };
            painter.text(
                rect.left_bottom() + vec2(2.0, below),
                Align2::LEFT_TOP,
                format!("{}  {}", icons::WARNING, widgets::ellipsize(problem, 420.0)),
                FontId::proportional(12.0),
                theme::readable(palette.warn, colours::sheet(palette), 4.5),
            );
        }
    }

    // --- beside the sheet ------------------------------------------------

    fn side(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        document: &mut Document,
        features: Option<&[String]>,
        deeds: &mut Vec<Deed>,
        requests: &mut Vec<Request>,
    ) {
        let whole = ui.max_rect();
        egui::Frame::new()
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_min_size(whole.size() - Vec2::splat(30.0));
                ui.set_max_width(whole.width() - 30.0);
                let only = (self.selected.len() == 1).then(|| self.selected.iter().next().copied()).flatten();
                if let Some(id) = only {
                    egui::ScrollArea::vertical().id_salt("node").auto_shrink([false, false]).show(ui, |ui| {
                        self.one_node(ui, palette, document, id, features, deeds, requests);
                    });
                } else if !self.selected.is_empty() {
                    ui.label(RichText::new(format!("{} nodes", self.selected.len())).strong().size(15.5).color(palette.text));
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Drag one to move them all. A copy takes the wires between them with it.").color(palette.muted),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    selection_buttons(ui, deeds);
                } else if let Some(wire) = self.wire {
                    self.one_wire(ui, palette, document, wire, deeds);
                } else {
                    widgets::caption(ui, "Add a node");
                    ui.add_space(4.0);
                    if let Some(node_type) = node_list(ui, palette, &mut self.search, false, features) {
                        deeds.push(Deed::Add(node_type, None));
                    }
                }
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn one_node(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        document: &mut Document,
        id: NodeId,
        features: Option<&[String]>,
        deeds: &mut Vec<Deed>,
        requests: &mut Vec<Request>,
    ) {
        let Some(graph) = document.config().map(|config| &config.node_graph) else {
            return;
        };
        let Some(node) = graph.nodes.get(&id).cloned() else {
            return;
        };
        // What each pin is wired to, in words.
        let other = |pin: PinRef| -> Vec<String> {
            graph
                .connections
                .iter()
                .filter_map(|wire| {
                    let (here, there) = if pin.output {
                        ((wire.from_node, wire.from_pin), wire.to_node)
                    } else {
                        ((wire.to_node, wire.to_pin), wire.from_node)
                    };
                    (here == (pin.node, pin.pin)).then(|| {
                        graph
                            .nodes
                            .get(&there)
                            .map(|node| node.node_type.display_name().to_string())
                    })?
                })
                .collect()
        };
        let pins: Vec<(PinRef, String, PinType, Vec<String>)> =
            [(false, &node.inputs), (true, &node.outputs)]
                .into_iter()
                .flat_map(|(output, pins)| {
                    pins.iter().map(move |pin| {
                        (
                            PinRef {
                                node: id,
                                pin: pin.id,
                                output,
                            },
                            pin.name.clone(),
                            pin.pin_type.clone(),
                        )
                    })
                })
                .map(|(pin, name, kind)| (pin, name, kind, other(pin)))
                .collect();

        let group = node.node_type.category().to_string();
        ui.horizontal(|ui| {
            widgets::dot(ui, true, colours::group(palette, &group));
            ui.label(
                RichText::new(node.node_type.display_name())
                    .strong()
                    .size(15.5)
                    .color(palette.text),
            );
        });
        ui.label(
            RichText::new(format!("{group}  ·  node {id}"))
                .size(12.0)
                .color(palette.faint),
        );
        if let Some(description) = description(&node.node_type) {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(description).color(palette.muted)).wrap());
        }
        if let NodeType::Unknown(_) = node.node_type {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("This version of gevrek does not know this node. It is kept as it is, and does nothing when the graph runs.")
                        .color(palette.warn),
                )
                .wrap(),
            );
        }
        if let (Some(feature), Some(features)) = (node.node_type.required_feature(), features) {
            if !features.iter().any(|have| have == feature) {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{}  The connected window manager cannot do this ({feature}).",
                            icons::WARNING
                        ))
                        .color(palette.warn),
                    )
                    .wrap(),
                );
            }
        }
        if let Some(Run {
            state: RunState::Failed(error),
            ..
        }) = self.runs.get(&id)
        {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{}  It failed: {error}", icons::X_CIRCLE))
                        .color(palette.error),
                )
                .wrap(),
            );
        }
        if node.node_type == NodeType::ManualTrigger {
            ui.add_space(8.0);
            let run = widgets::primary_button(ui, features.is_some(), "Run now");
            if run
                .on_hover_text("Start the graph here, to try what follows")
                .on_disabled_hover_text("No window manager to run it")
                .clicked()
            {
                requests.push(Request::Trigger(id));
            }
            ui.add(
                egui::Label::new(
                    RichText::new("It runs the graph the window manager has: apply first when you have changed it.")
                        .size(12.0)
                        .color(palette.faint),
                )
                .wrap(),
            );
        }

        ui.add_space(10.0);
        widgets::caption(ui, "Settings");
        ui.add_space(2.0);
        self.keys_of(ui, palette, document, &node);
        if !settings(ui, document, id) {
            ui.label(RichText::new("Nothing to set.").color(palette.muted));
        }
        self.windows_of(ui, palette, &node);

        if !pins.is_empty() {
            ui.add_space(10.0);
            widgets::caption(ui, "Pins");
            ui.add_space(2.0);
            for (pin, name, kind, others) in &pins {
                ui.horizontal(|ui| {
                    widgets::dot(ui, !others.is_empty(), colours::pin(palette, kind));
                    let arrow = if pin.output {
                        icons::ARROW_RIGHT
                    } else {
                        icons::ARROW_LEFT
                    };
                    ui.label(RichText::new(format!("{arrow} {name}")).color(palette.text))
                        .on_hover_text(format!(
                            "{}: {}",
                            if pin.output { "Gives" } else { "Takes" },
                            colours::pin_name(kind)
                        ));
                    if !others.is_empty() {
                        let text = widgets::ellipsize(&others.join(", "), 150.0);
                        ui.label(RichText::new(text).size(12.0).color(palette.muted));
                        if widgets::small_icon_button(ui, icons::LINK_BREAK, "Take its wires away")
                            .clicked()
                        {
                            deeds.push(Deed::Disconnect(*pin));
                        }
                    }
                });
            }
        }
        ui.add_space(12.0);
        selection_buttons(ui, deeds);
    }

    fn one_wire(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        document: &Document,
        wire: WireRef,
        deeds: &mut Vec<Deed>,
    ) {
        let Some(graph) = document.config().map(|config| &config.node_graph) else {
            return;
        };
        let end = |node: NodeId, pin: usize, output: bool| {
            let node = graph.nodes.get(&node)?;
            let pins = if output { &node.outputs } else { &node.inputs };
            let pin = pins.iter().find(|other| other.id == pin)?;
            Some((
                node.node_type.display_name().to_string(),
                pin.name.clone(),
                pin.pin_type.clone(),
            ))
        };
        let (Some(from), Some(to)) = (
            end(wire.from_node, wire.from_pin, true),
            end(wire.to_node, wire.to_pin, false),
        ) else {
            return;
        };
        ui.horizontal(|ui| {
            widgets::dot(ui, true, colours::pin(palette, &from.2));
            ui.label(
                RichText::new("A wire")
                    .strong()
                    .size(15.5)
                    .color(palette.text),
            );
        });
        ui.add_space(4.0);
        ui.label(format!("From  {}  ·  {}", from.0, from.1));
        ui.label(format!("To  {}  ·  {}", to.0, to.1));
        ui.label(
            RichText::new(format!("It carries {}.", colours::pin_name(&from.2)))
                .color(palette.muted),
        );
        ui.add_space(8.0);
        if ui
            .button(format!("{}  Remove", icons::TRASH))
            .on_hover_text("Delete")
            .clicked()
        {
            deeds.push(Deed::RemoveWire(wire));
        }
    }

    /// The list of nodes, opened on the sheet where the node is to go.
    fn popup_list(
        &mut self,
        context: &egui::Context,
        palette: &Palette,
        features: Option<&[String]>,
        deeds: &mut Vec<Deed>,
    ) {
        let Some(popup) = &mut self.popup else {
            return;
        };
        let fresh = std::mem::take(&mut popup.fresh);
        let at = popup.at;
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("graph-nodes"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup.screen)
            .constrain_to(context.content_rect().shrink(8.0))
            .show(context, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.set_max_height(380.0);
                    chosen = node_list(ui, palette, &mut popup.search, fresh, features);
                });
            });
        let escape = context.input(|input| input.key_pressed(egui::Key::Escape));
        if let Some(node_type) = chosen {
            deeds.push(Deed::Add(node_type, Some(at)));
            self.popup = None;
        } else if escape || (!fresh && area.response.clicked_elsewhere()) {
            self.popup = None;
        }
    }
}

fn selection_buttons(ui: &mut Ui, deeds: &mut Vec<Deed>) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(format!("{}  Duplicate", icons::COPY_SIMPLE))
            .on_hover_text("Ctrl+D")
            .clicked()
        {
            deeds.push(Deed::Duplicate);
        }
        if ui
            .button(format!("{}  Copy", icons::COPY))
            .on_hover_text("Ctrl+C")
            .clicked()
        {
            deeds.push(Deed::Copy);
        }
        if ui
            .button(format!("{}  Remove", icons::TRASH))
            .on_hover_text("Delete")
            .clicked()
        {
            deeds.push(Deed::Delete);
        }
    });
}

/// The graph of the document, ready to be changed.
fn working_copy(document: &Document) -> Option<NodeGraph> {
    let mut graph = document.config()?.node_graph.clone();
    // Reading does not build the graph's lookup tables.
    graph.rebuild_connection_indices();
    Some(graph)
}

/// Write the graph back into the document.
fn commit(document: &mut Document, graph: &NodeGraph, change: Change) {
    if change == Change::None {
        return;
    }
    let Ok(value) = serde_json::to_value(graph) else {
        return;
    };
    if change == Change::Smooth {
        // The document joins changes at one place that follow each other
        // closely: a node dragged across the sheet is one step to undo.
        document.set(GRAPH, value);
    } else {
        document.edit(GRAPH, |graph| *graph = value);
    }
}

fn has_wire(graph: &NodeGraph, wire: WireRef) -> bool {
    graph.connections.iter().any(|other| {
        other.from_node == wire.from_node
            && other.from_pin == wire.from_pin
            && other.to_node == wire.to_node
            && other.to_pin == wire.to_pin
    })
}

fn pin_kind(graph: &NodeGraph, pin: PinRef) -> Option<&PinType> {
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

fn lit_for(state: &RunState) -> Duration {
    match state {
        RunState::Running => LIT_RUNNING,
        RunState::Done => LIT,
        RunState::Failed(_) => LIT_FAILED,
    }
}

/// The settings of a node, drawn by the form at their place in the
/// document: whatever the model gives a node, it has its control here.
/// Returns whether the node has any.
fn settings(ui: &mut Ui, document: &mut Document, id: NodeId) -> bool {
    let place = format!("{GRAPH}/nodes/{id}/node_type");
    let Some(schema) = gevrek_schema::schema().at(&place) else {
        return false;
    };
    let Kind::Union(variants) = schema.kind() else {
        return false;
    };
    let Some(current) = document.get(&place).cloned() else {
        return false;
    };
    let Some(variant) = variants.iter().find(|variant| variant.matches(&current)) else {
        return false;
    };
    let (inside, fields) = match &variant.shape {
        VariantShape::Word => return false,
        VariantShape::Tagged { fields, .. } => (place.clone(), fields.clone()),
        VariantShape::Wrapped(inner) => match inner.kind() {
            Kind::Object(fields) => (format!("{place}/{}", escape(variant.name)), fields),
            _ => {
                let inside = format!("{place}/{}", escape(variant.name));
                let mut form = Form::new(document);
                form.stacked = true;
                form.place(ui, &inside, *inner, "Value", 1);
                return true;
            }
        },
    };
    let mut form = Form::new(document);
    // The panel is narrow: names above their controls.
    form.stacked = true;
    for field in &fields {
        form.place(
            ui,
            &format!("{inside}/{}", escape(field.name)),
            field.node,
            &gevrek_schema::title(field.name),
            1,
        );
    }
    !fields.is_empty()
}

/// The word that names a node's type in the file (`MoveWindow`).
fn type_key(node_type: &NodeType) -> Option<String> {
    match serde_json::to_value(node_type).ok()? {
        serde_json::Value::String(word) => Some(word),
        serde_json::Value::Object(object) if object.len() == 1 => object.keys().next().cloned(),
        _ => None,
    }
}

/// What a node does, from the documentation of its type in the model.
fn description(node_type: &NodeType) -> Option<&'static str> {
    static ALL: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        let Some(Kind::Union(variants)) = gevrek_schema::schema()
            .at(&format!("{GRAPH}/nodes/0/node_type"))
            .map(|node| node.kind())
        else {
            return HashMap::new();
        };
        variants
            .into_iter()
            .filter(|variant| !variant.description.is_empty())
            .map(|variant| (variant.name.to_string(), variant.description))
            .collect()
    });
    all.get(&type_key(node_type)?).copied()
}

/// The nodes the list offers for what was typed, by group. Every word
/// typed must be in the node's name, its group or its word in the file.
/// Nodes the connected window manager cannot run are left out.
fn offered(needle: &str, features: Option<&[String]>) -> Vec<(&'static str, Vec<NodeType>)> {
    let needle = needle.trim().to_lowercase();
    gevrek_model::graph::node_catalog()
        .into_iter()
        .map(|(group, types)| {
            let types: Vec<NodeType> = types
                .into_iter()
                .filter(|node_type| match (node_type.required_feature(), features) {
                    (Some(feature), Some(features)) => features.iter().any(|have| have == feature),
                    _ => true,
                })
                .filter(|node_type| {
                    let hay = format!(
                        "{} {group} {}",
                        node_type.display_name(),
                        type_key(node_type).unwrap_or_default()
                    )
                    .to_lowercase();
                    needle.split_whitespace().all(|word| hay.contains(word))
                })
                .collect();
            (group, types)
        })
        .filter(|(_, types)| !types.is_empty())
        .collect()
}

/// A field to search in and the nodes found, by group. Returns the node
/// that was chosen (by a click, or Enter for the first).
fn node_list(
    ui: &mut Ui,
    palette: &Palette,
    search: &mut String,
    take_keyboard: bool,
    features: Option<&[String]>,
) -> Option<NodeType> {
    let field = ui.add(
        egui::TextEdit::singleline(search)
            .hint_text(format!("{}  Find a node", icons::MAGNIFYING_GLASS))
            .desired_width(f32::INFINITY),
    );
    if take_keyboard {
        field.request_focus();
    }
    let groups = offered(search, features);
    let mut chosen = None;
    if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        chosen = groups.first().and_then(|(_, types)| types.first().cloned());
    }
    ui.add_space(4.0);
    if groups.is_empty() {
        ui.label(RichText::new("No node like that.").color(palette.muted));
    }
    let row = 28.0;
    egui::ScrollArea::vertical()
        .id_salt("nodes")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (group, types) in groups {
                ui.add_space(10.0);
                widgets::caption(ui, group);
                ui.add_space(3.0);
                let colour = colours::group(palette, group);
                for node_type in types {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), row), Sense::click());
                    widgets::list_row_bg(ui, &response, rect, false);
                    ui.painter().circle_filled(
                        pos2(rect.left() + 12.0, rect.center().y),
                        4.0,
                        colour,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 26.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        node_type.display_name(),
                        FontId::proportional(14.0),
                        palette.text,
                    );
                    let response = match description(&node_type) {
                        Some(description) => response.on_hover_text(description),
                        None => response,
                    };
                    if response.clicked() {
                        chosen = Some(node_type);
                    }
                }
            }
        });
    chosen
}

/// What a node is set to, in a few words, for its header: the keys of a
/// key node, the text of a text node. `None` when the name says it all.
fn summary(node_type: &NodeType) -> Option<String> {
    Some(match node_type {
        NodeType::OnKeyPress {
            key,
            ctrl,
            alt,
            shift,
            win,
        } => {
            if key.trim().is_empty() {
                "no key".to_string()
            } else {
                let mut parts: Vec<&str> = Vec::new();
                if *win {
                    parts.push(gevrek_model::shortcuts::super_key_name());
                }
                if *ctrl {
                    parts.push("Ctrl");
                }
                if *alt {
                    parts.push("Alt");
                }
                if *shift {
                    parts.push("Shift");
                }
                parts.push(key.trim());
                parts.join("+")
            }
        }
        NodeType::StringValue { value } => format!("\"{value}\""),
        NodeType::NumberValue { value } => value.to_string(),
        NodeType::BoolValue { value } => (if *value { "yes" } else { "no" }).to_string(),
        NodeType::LogMessage { message, .. } => message.clone(),
        NodeType::SetWindowArrangement { mode } => format!("{mode:?}"),
        NodeType::CreateRule { pattern, .. } if !pattern.is_empty() => format!("\"{pattern}\""),
        NodeType::ApplySystemFilters { enabled } => {
            (if *enabled { "on" } else { "off" }).to_string()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::Config;

    #[test]
    fn the_list_finds_nodes_by_any_of_their_words() {
        let all: usize = offered("", None).iter().map(|(_, types)| types.len()).sum();
        let catalog: usize = gevrek_model::graph::node_catalog()
            .iter()
            .map(|(_, types)| types.len())
            .sum();
        assert_eq!(all, catalog, "nothing typed, nothing left out");

        let found = offered("  key  ", None);
        assert!(found.iter().any(|(_, types)| types
            .iter()
            .any(|node_type| matches!(node_type, NodeType::OnKeyPress { .. }))));
        // Two words, in any order, one of them the group's.
        let found = offered("events startup", None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "Events");
        assert!(found[0]
            .1
            .iter()
            .all(|node_type| node_type.display_name().contains("Startup")));
        // The word in the file is a way to find it too.
        assert!(!offered("OnWindowCreated", None).is_empty());
        assert!(offered("no such node anywhere", None).is_empty());
    }

    #[test]
    fn what_the_window_manager_cannot_run_is_not_offered() {
        let needs = |found: &[(&str, Vec<NodeType>)]| {
            found
                .iter()
                .flat_map(|(_, types)| types)
                .filter(|node_type| node_type.required_feature().is_some())
                .count()
        };
        assert!(
            needs(&offered("", None)) > 0,
            "without a window manager everything is offered"
        );
        assert_eq!(needs(&offered("", Some(&[]))), 0);
        let can = [gevrek_model::graph::FEATURE_WINDOWS_WINDOW_EFFECTS.to_string()];
        assert_eq!(needs(&offered("", Some(&can))), needs(&offered("", None)));
    }

    #[test]
    fn a_node_is_described_by_the_model() {
        assert_eq!(type_key(&NodeType::OnStartup).as_deref(), Some("OnStartup"));
        assert_eq!(
            type_key(&NodeType::MoveWindow { x: 1, y: 2 }).as_deref(),
            Some("MoveWindow")
        );
        // The schema knows every node of the catalogue by the same word.
        let Some(Kind::Union(variants)) = gevrek_schema::schema()
            .at("/node_graph/nodes/0/node_type")
            .map(|node| node.kind())
        else {
            panic!("the schema describes a node's type as one of several forms");
        };
        for (_, types) in gevrek_model::graph::node_catalog() {
            for node_type in types {
                let key = type_key(&node_type).expect("a word");
                let value = serde_json::to_value(&node_type).expect("JSON");
                let variant = variants.iter().find(|variant| variant.matches(&value));
                assert_eq!(
                    variant.map(|variant| variant.name),
                    Some(key.as_str()),
                    "{}",
                    node_type.display_name()
                );
            }
        }
    }

    #[test]
    fn a_change_of_the_graph_is_a_step_of_the_document() {
        let mut document = Document::new(&Config::default());
        let mut graph = working_copy(&document).expect("a graph");
        assert!(graph.nodes.is_empty());
        let start = graph.add_node(NodeType::OnStartup, [0.0, 0.0]);
        let log = graph.add_node(
            NodeType::LogMessage {
                message: "hello".to_string(),
                level: gevrek_model::graph::LogLevel::Info,
            },
            [300.0, 0.0],
        );
        commit(&mut document, &graph, Change::Step);
        assert_eq!(document.config().expect("valid").node_graph.nodes.len(), 2);

        // A wire, from the document's own copy (its lookup tables are built).
        let mut graph = working_copy(&document).expect("a graph");
        let from = PinRef {
            node: start,
            pin: graph.nodes[&start].outputs[0].id,
            output: true,
        };
        let to = PinRef {
            node: log,
            pin: graph.nodes[&log].inputs[0].id,
            output: false,
        };
        assert!(edit::connect(&mut graph, from, to));
        commit(&mut document, &graph, Change::Step);
        assert_eq!(
            document
                .config()
                .expect("valid")
                .node_graph
                .connections
                .len(),
            1
        );

        // Dragging: many changes, one step.
        for step in 1..=10 {
            let mut graph = working_copy(&document).expect("a graph");
            graph.nodes.get_mut(&log).expect("node").position[0] = 300.0 + step as f32 * 5.0;
            commit(&mut document, &graph, Change::Smooth);
        }
        assert_eq!(
            document.config().expect("valid").node_graph.nodes[&log].position[0],
            350.0
        );
        assert!(document.undo());
        let graph = &document.config().expect("valid").node_graph;
        assert_eq!(
            graph.nodes[&log].position[0], 300.0,
            "the drag is taken back as one"
        );
        assert_eq!(graph.connections.len(), 1);
        assert!(document.undo());
        assert!(document
            .config()
            .expect("valid")
            .node_graph
            .connections
            .is_empty());
        assert!(document.undo());
        assert!(document
            .config()
            .expect("valid")
            .node_graph
            .nodes
            .is_empty());
        assert!(!document.can_undo());

        // Nothing changed, nothing remembered.
        let graph = working_copy(&document).expect("a graph");
        commit(&mut document, &graph, Change::None);
        commit(&mut document, &graph, Change::Step);
        assert!(!document.can_undo());
    }

    #[test]
    fn the_settings_of_every_node_have_a_place_in_the_document() {
        // The form draws a node's settings at these places: each must be
        // there, in the document and in the schema.
        let mut graph = NodeGraph::new();
        for (_, types) in gevrek_model::graph::node_catalog() {
            for node_type in types {
                graph.add_node(node_type, [0.0, 0.0]);
            }
        }
        let document = Document::new(&Config {
            node_graph: graph,
            ..Config::default()
        });
        let mut with_settings = 0;
        for (id, node) in &document.config().expect("valid").node_graph.nodes {
            let place = format!("{GRAPH}/nodes/{id}/node_type");
            let value = document.get(&place).expect("the node's type");
            let Some(Kind::Union(variants)) =
                gevrek_schema::schema().at(&place).map(|node| node.kind())
            else {
                panic!("{place} is one of several forms");
            };
            let variant = variants
                .iter()
                .find(|variant| variant.matches(value))
                .expect("a form of the schema");
            if let VariantShape::Wrapped(inner) = &variant.shape {
                let Kind::Object(fields) = inner.kind() else {
                    panic!("{}: its settings are named", node.node_type.display_name());
                };
                for field in fields {
                    let place = format!("{place}/{}/{}", variant.name, field.name);
                    assert!(document.get(&place).is_some(), "{place} is in the document");
                }
                with_settings += 1;
            }
        }
        assert!(
            with_settings > 20,
            "{with_settings} kinds of node have settings"
        );
    }

    #[test]
    fn a_node_says_what_it_is_set_to() {
        let key = NodeType::OnKeyPress {
            key: "J".to_string(),
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
        };
        assert_eq!(summary(&key).as_deref(), Some("Ctrl+Alt+J"));
        let unset = NodeType::OnKeyPress {
            key: String::new(),
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        };
        assert_eq!(summary(&unset).as_deref(), Some("no key"));
        assert_eq!(summary(&NodeType::OnStartup), None);
        assert_eq!(
            summary(&NodeType::StringValue {
                value: "Settings".to_string()
            })
            .as_deref(),
            Some("\"Settings\"")
        );
    }
}
