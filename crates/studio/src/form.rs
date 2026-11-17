//! Forms made from the schema.
//!
//! One function draws any place of the configuration: it asks the schema
//! what the place holds and draws the control for it. A switch for a
//! switch, a slider for a number that says its range, a choice, a colour,
//! and for what has parts (a list, one of several forms) the parts, each
//! drawn the same way. A setting added to the model has its form here
//! without a line written for it; the pages only say which places they
//! show, and draw by hand what deserves better than a form.

use crate::document::{escape, Document};
use egui::{RichText, Ui};
use gevrek_schema::{hint, title, Kind, Node, Variant, VariantShape};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;
use serde_json::Value;

/// What a form reads and changes. The configuration ([`Document`]) is
/// one; a single value that stands alone (a command about to be run) is
/// another.
pub trait Store {
    fn get(&self, pointer: &str) -> Option<&Value>;
    fn set(&mut self, pointer: &str, new: Value) -> bool;
    fn edit_in_place(&mut self, pointer: &str, change: &mut dyn FnMut(&mut Value)) -> bool;
    /// gevrek's default at a place, when the place has one.
    fn default_at(&self, pointer: &str) -> Option<&Value>;
    fn reset(&mut self, pointer: &str) -> bool;
}

impl Store for Document {
    fn get(&self, pointer: &str) -> Option<&Value> {
        Document::get(self, pointer)
    }
    fn set(&mut self, pointer: &str, new: Value) -> bool {
        Document::set(self, pointer, new)
    }
    fn edit_in_place(&mut self, pointer: &str, change: &mut dyn FnMut(&mut Value)) -> bool {
        Document::edit(self, pointer, |value| change(value))
    }
    fn default_at(&self, pointer: &str) -> Option<&Value> {
        Document::default_at(self, pointer)
    }
    fn reset(&mut self, pointer: &str) -> bool {
        Document::reset(self, pointer)
    }
}

/// A value on its own: no defaults, no undo.
pub struct Loose(pub Value);

impl Store for Loose {
    fn get(&self, pointer: &str) -> Option<&Value> {
        self.0.pointer(pointer)
    }
    fn set(&mut self, pointer: &str, new: Value) -> bool {
        if pointer.is_empty() {
            self.0 = new;
            return true;
        }
        if let Some(place) = self.0.pointer_mut(pointer) {
            *place = new;
            return true;
        }
        let Some((parent, key)) = pointer.rsplit_once('/') else {
            return false;
        };
        let parent = if parent.is_empty() {
            Some(&mut self.0)
        } else {
            self.0.pointer_mut(parent)
        };
        match parent {
            Some(Value::Object(object)) => {
                object.insert(key.replace("~1", "/").replace("~0", "~"), new);
                true
            }
            _ => false,
        }
    }
    fn edit_in_place(&mut self, pointer: &str, change: &mut dyn FnMut(&mut Value)) -> bool {
        let place = if pointer.is_empty() {
            Some(&mut self.0)
        } else {
            self.0.pointer_mut(pointer)
        };
        match place {
            Some(value) => {
                change(value);
                true
            }
            None => false,
        }
    }
    fn default_at(&self, _: &str) -> Option<&Value> {
        None
    }
    fn reset(&mut self, _: &str) -> bool {
        false
    }
}

pub struct Form<'a> {
    pub document: &'a mut dyn Store,
    /// A setting to draw a ring around and scroll to (found by search).
    pub highlight: Option<&'a str>,
    /// For a narrow place (a panel beside something): the name of a value
    /// above its control, not beside it.
    pub stacked: bool,
}

/// How deep parts are drawn inside parts before the rest is left to the
/// raw configuration.
const DEPTH: usize = 8;

impl<'a> Form<'a> {
    pub fn new(document: &'a mut dyn Store) -> Self {
        Self {
            document,
            highlight: None,
            stacked: false,
        }
    }

    /// A value that stands alone, as `node` describes it: the store holds
    /// just that value.
    pub fn value(&mut self, ui: &mut Ui, node: Node<'_>, label: &str) {
        self.place(ui, "", node, label, 0);
    }

    /// The fields of the object at `pointer`, one below the other, except
    /// those named in `skip`.
    pub fn fields(&mut self, ui: &mut Ui, pointer: &str, skip: &[&str]) {
        let Some(node) = gevrek_schema::schema().at(pointer) else {
            return;
        };
        let fields = match node.kind() {
            Kind::Object(fields) => fields,
            Kind::Optional(inner) => match inner.kind() {
                Kind::Object(fields) => fields,
                _ => return,
            },
            _ => return,
        };
        for field in fields {
            if skip.contains(&field.name) {
                continue;
            }
            self.place(
                ui,
                &format!("{pointer}/{}", escape(field.name)),
                field.node,
                &title(field.name),
                0,
            );
        }
    }

    /// Some fields of the object at `pointer`, in the order given.
    pub fn only(&mut self, ui: &mut Ui, pointer: &str, names: &[&str]) {
        for name in names {
            let place = format!("{pointer}/{name}");
            if let Some(node) = gevrek_schema::schema().at(&place) {
                self.place(ui, &place, node, &title(name), 0);
            }
        }
    }

    /// One place, whatever it holds.
    pub fn place(&mut self, ui: &mut Ui, pointer: &str, node: Node<'_>, label: &str, depth: usize) {
        let kind = node.kind();
        if is_simple(&kind) {
            self.row(ui, pointer, node, &kind, label);
            return;
        }
        if depth > DEPTH {
            ui.weak(format!("{label}: edit in the raw configuration"));
            return;
        }
        ui.push_id(pointer, |ui| match kind {
            Kind::Object(fields) => {
                section(ui, label, node.description());
                ui.indent("fields", |ui| {
                    for field in fields {
                        self.place(
                            ui,
                            &format!("{pointer}/{}", escape(field.name)),
                            field.node,
                            &title(field.name),
                            depth + 1,
                        );
                    }
                });
            }
            Kind::Optional(inner) => self.optional(ui, pointer, node, inner, label, depth),
            Kind::List(item) => self.list(ui, pointer, node, item, label, depth),
            Kind::Map(item) => self.map(ui, pointer, node, item, label, depth),
            Kind::Union(variants) => self.union(ui, pointer, node, &variants, label, depth),
            Kind::Tuple(parts) => self.tuple(ui, pointer, node, &parts, label),
            _ => {}
        });
    }

    /// A value on one line: its name on the left, its control on the
    /// right, a dot that says whether it differs from gevrek's default.
    fn row(&mut self, ui: &mut Ui, pointer: &str, node: Node<'_>, kind: &Kind<'_>, label: &str) {
        let has_default = self.document.default_at(pointer).is_some();
        let is_set = has_default && self.document.default_at(pointer) != self.document.get(pointer);
        let highlight = self.highlight == Some(pointer);
        let current = self.document.get(pointer).cloned().unwrap_or(Value::Null);
        let mut changed = None;
        let workspaces = (matches!(kind, Kind::Text) && names_a_workspace(self.document, pointer))
            .then(|| workspaces_of(self.document))
            .filter(|workspaces| !workspaces.is_empty());
        // A setting that does not take effect at once says when it does,
        // beside its name, and why, with what it is for.
        let waits = gevrek_schema::waits(pointer);
        let (said_label, said_description);
        let (label, description) = match waits {
            Some((when, why)) => {
                said_label = format!("{label}  ({when})");
                said_description = format!("{}\n\n{why}", node.description());
                (said_label.as_str(), said_description.as_str())
            }
            None => (label, node.description()),
        };
        let ((), reset) = line(
            ui,
            self.stacked,
            label,
            description,
            is_set,
            highlight,
            |ui| {
                ui.push_id(pointer, |ui| {
                    // A workspace's name is chosen from the workspaces
                    // there are, not typed: a name that is typed and
                    // names none does nothing, and says nothing.
                    changed = match &workspaces {
                        Some(workspaces) => workspace_control(ui, workspaces, &current),
                        None => control(ui, pointer, kind, &current),
                    };
                });
            },
        );
        if reset {
            self.document.reset(pointer);
        } else if let Some(new) = changed {
            self.document.set(pointer, new);
        }
    }

    /// A value that may be absent: a switch says whether it is there.
    fn optional(
        &mut self,
        ui: &mut Ui,
        pointer: &str,
        node: Node<'_>,
        inner: Node<'_>,
        label: &str,
        depth: usize,
    ) {
        let present = self
            .document
            .get(pointer)
            .is_some_and(|value| !value.is_null());
        let mut wanted = present;
        ui.horizontal(|ui| {
            widgets::switch(ui, &mut wanted);
            ui.label(label).on_hover_text(node.description());
        });
        if wanted != present {
            // The default of the place is "absent", so the inside's own
            // emptiest value is taken, not the place's.
            let new = if wanted {
                blank_inside(inner)
            } else {
                Value::Null
            };
            self.document.set(pointer, new);
        }
        if wanted && present {
            ui.indent("inside", |ui| {
                self.place(ui, pointer, inner, label, depth + 1)
            });
        }
    }

    fn list(
        &mut self,
        ui: &mut Ui,
        pointer: &str,
        node: Node<'_>,
        item: Node<'_>,
        label: &str,
        depth: usize,
    ) {
        let count = self
            .document
            .get(pointer)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let ordered_here = !ORDERED_ELSEWHERE.contains(&pointer);
        let mut add = false;
        ui.horizontal(|ui| {
            section_title(ui, &format!("{label}  ·  {count}"), node.description());
            if widgets::small_icon_button(ui, icons::PLUS, "Add one").clicked() {
                add = true;
            }
        });
        if add {
            let mut new = item.blank();
            new_for_this_system(&mut new, gevrek_model::examples::default_terminal());
            self.document.edit_in_place(pointer, &mut |list| {
                if let Value::Array(items) = list {
                    items.push(new.clone());
                }
            });
        }
        let simple = is_simple(&item.kind());
        let mut action = None;
        for index in 0..count {
            let place = format!("{pointer}/{index}");
            if simple {
                ui.horizontal(|ui| {
                    let kind = item.kind();
                    let current = self.document.get(&place).cloned().unwrap_or(Value::Null);
                    let mut changed = None;
                    ui.push_id(&place, |ui| changed = control(ui, &place, &kind, &current));
                    if let Some(new) = changed {
                        self.document.set(&place, new);
                    }
                    item_buttons(ui, index, count, ordered_here, &mut action);
                });
                continue;
            }
            let summary = self
                .document
                .get(&place)
                .map(|value| summary(value, index))
                .unwrap_or_default();
            let id = ui.make_persistent_id(("item", &place));
            let header = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );
            header
                .show_header(ui, |ui| {
                    ui.label(RichText::new(summary).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        item_buttons(ui, index, count, ordered_here, &mut action);
                    });
                })
                .body(|ui| self.inside(ui, &place, item, depth + 1));
        }
        if let Some(action) = action {
            self.document.edit_in_place(pointer, &mut |list| {
                if let Value::Array(items) = list {
                    match action {
                        ItemAction::Remove(index) if index < items.len() => {
                            items.remove(index);
                        }
                        ItemAction::Up(index) if index > 0 && index < items.len() => {
                            items.swap(index, index - 1)
                        }
                        ItemAction::Down(index) if index + 1 < items.len() => {
                            items.swap(index, index + 1)
                        }
                        _ => {}
                    }
                }
            });
        }
        ui.add_space(4.0);
    }

    fn map(
        &mut self,
        ui: &mut Ui,
        pointer: &str,
        node: Node<'_>,
        item: Node<'_>,
        label: &str,
        depth: usize,
    ) {
        let keys: Vec<String> = self
            .document
            .get(pointer)
            .and_then(Value::as_object)
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        section(
            ui,
            &format!("{label}  ·  {}", keys.len()),
            node.description(),
        );
        let mut remove = None;
        ui.indent("entries", |ui| {
            for key in &keys {
                let place = format!("{pointer}/{}", escape(key));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(key).monospace());
                    if widgets::small_icon_button(ui, icons::TRASH, "Remove").clicked() {
                        remove = Some(key.clone());
                    }
                });
                ui.indent(("entry", key), |ui| {
                    self.inside(ui, &place, item, depth + 1)
                });
            }
            // A new entry: its name, then Enter or the button.
            let id = ui.make_persistent_id("new key");
            let mut name = ui
                .data_mut(|data| data.get_temp::<String>(id))
                .unwrap_or_default();
            let mut add = false;
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("name")
                        .desired_width(160.0),
                );
                add = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                add |= widgets::small_icon_button(ui, icons::PLUS, "Add an entry with this name")
                    .clicked();
            });
            let name_now = name.trim().to_string();
            if add && !name_now.is_empty() && !keys.contains(&name_now) {
                let new = item.blank();
                self.document.edit_in_place(pointer, &mut |map| {
                    if let Value::Object(object) = map {
                        object.insert(name_now.clone(), new.clone());
                    }
                });
                name.clear();
            }
            ui.data_mut(|data| data.insert_temp(id, name));
        });
        if let Some(key) = remove {
            self.document.edit_in_place(pointer, &mut |map| {
                if let Value::Object(object) = map {
                    object.remove(&key);
                }
            });
        }
    }

    /// The parts of a value that has parts, without a heading of its own.
    fn inside(&mut self, ui: &mut Ui, pointer: &str, node: Node<'_>, depth: usize) {
        match node.kind() {
            Kind::Object(fields) => {
                for field in fields {
                    self.place(
                        ui,
                        &format!("{pointer}/{}", escape(field.name)),
                        field.node,
                        &title(field.name),
                        depth,
                    );
                }
            }
            _ => self.place(ui, pointer, node, "Value", depth),
        }
    }

    /// One of several forms: which one, then that one's own fields.
    fn union(
        &mut self,
        ui: &mut Ui,
        pointer: &str,
        node: Node<'_>,
        variants: &[Variant<'_>],
        label: &str,
        depth: usize,
    ) {
        let current = self.document.get(pointer).cloned().unwrap_or(Value::Null);
        let chosen = variants.iter().find(|variant| variant.matches(&current));
        let mut pick = None;
        let has_default = self.document.default_at(pointer).is_some();
        let is_set = has_default && self.document.default_at(pointer) != self.document.get(pointer);
        let ((), reset) = line(
            ui,
            self.stacked,
            label,
            node.description(),
            is_set,
            self.highlight == Some(pointer),
            |ui| {
                let shown = chosen.map_or_else(
                    || "(not known to this version)".to_string(),
                    |variant| title(variant.name),
                );
                egui::ComboBox::from_id_salt("variant")
                    .selected_text(shown)
                    .width(220.0)
                    .height(420.0)
                    .show_ui(ui, |ui| {
                        for variant in variants {
                            let selected = chosen.is_some_and(|chosen| chosen.name == variant.name);
                            let response = ui.selectable_label(selected, title(variant.name));
                            let response = if variant.description.is_empty() {
                                response
                            } else {
                                response.on_hover_text(variant.description)
                            };
                            if response.clicked() && !selected {
                                pick = Some(variant.blank());
                            }
                        }
                    });
            },
        );
        if reset {
            self.document.reset(pointer);
            return;
        }
        if let Some(new) = pick {
            self.document.set(pointer, new);
            return;
        }
        let Some(variant) = chosen else {
            return;
        };
        match &variant.shape {
            VariantShape::Word => {}
            VariantShape::Tagged { fields, .. } => {
                if !fields.is_empty() {
                    ui.indent("form", |ui| {
                        for field in fields {
                            self.place(
                                ui,
                                &format!("{pointer}/{}", escape(field.name)),
                                field.node,
                                &title(field.name),
                                depth + 1,
                            );
                        }
                    });
                }
            }
            VariantShape::Wrapped(inner) => {
                let place = format!("{pointer}/{}", escape(variant.name));
                ui.indent("form", |ui| {
                    self.place(ui, &place, *inner, &title(variant.name), depth + 1)
                });
            }
        }
    }

    /// A few values in a row (a position, a size).
    fn tuple(
        &mut self,
        ui: &mut Ui,
        pointer: &str,
        node: Node<'_>,
        parts: &[Node<'_>],
        label: &str,
    ) {
        let has_default = self.document.default_at(pointer).is_some();
        let is_set = has_default && self.document.default_at(pointer) != self.document.get(pointer);
        let mut changes = Vec::new();
        let ((), reset) = line(
            ui,
            self.stacked,
            label,
            node.description(),
            is_set,
            self.highlight == Some(pointer),
            |ui| {
                for (index, part) in parts.iter().enumerate() {
                    let place = format!("{pointer}/{index}");
                    let current = self.document.get(&place).cloned().unwrap_or(Value::Null);
                    ui.push_id(index, |ui| {
                        if let Some(new) = control(ui, &place, &part.kind(), &current) {
                            changes.push((place.clone(), new));
                        }
                    });
                }
            },
        );
        if reset {
            self.document.reset(pointer);
        }
        for (place, new) in changes {
            self.document.set(&place, new);
        }
    }
}

/// One value with its name: beside each other where there is room,
/// `stacked` where there is not. Returns what the control returned, and
/// whether the default was asked back.
fn line<R>(
    ui: &mut Ui,
    stacked: bool,
    label: &str,
    help: &str,
    is_set: bool,
    highlight: bool,
    control: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    if !stacked {
        return widgets::row(ui, label, help, is_set, highlight, control);
    }
    let palette = theme::pal(ui.ctx());
    let mut reset = false;
    ui.add_space(4.0);
    let name = ui.horizontal(|ui| {
        let colour = if is_set { palette.text } else { palette.muted };
        let name = ui.label(RichText::new(label).size(12.5).color(colour));
        if !help.is_empty() {
            name.on_hover_text(help);
        }
        if is_set
            && widgets::small_icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Back to the default")
                .clicked()
        {
            reset = true;
        }
    });
    let inner = ui.horizontal(|ui| control(ui));
    if highlight {
        let ring = name.response.rect.union(inner.response.rect).expand(3.0);
        ui.painter().rect_stroke(
            ring,
            4.0,
            egui::Stroke::new(2.0, palette.accent_fg()),
            egui::StrokeKind::Outside,
        );
        inner.response.scroll_to_me(Some(egui::Align::Center));
    }
    (inner.inner, reset)
}

/// The emptiest value of what an optional place holds when it is there.
fn blank_inside(inner: Node<'_>) -> Value {
    match inner.kind() {
        Kind::Bool => Value::Bool(false),
        Kind::Integer { min, .. } => Value::from(min.unwrap_or(0).max(0)),
        Kind::Number => Value::from(1.0),
        Kind::Text => Value::String(String::new()),
        Kind::Color => Value::from(vec![128, 128, 128]),
        Kind::Choice(words) => words
            .first()
            .map_or(Value::Null, |word| Value::from(word.value)),
        Kind::Tuple(parts) => Value::Array(parts.iter().map(|part| blank_inside(*part)).collect()),
        Kind::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|field| field.required)
                .map(|field| (field.name.to_string(), blank_inside(field.node)))
                .collect(),
        ),
        Kind::List(_) => Value::Array(Vec::new()),
        Kind::Map(_) => Value::Object(Default::default()),
        Kind::Union(variants) => variants.first().map_or(Value::Null, Variant::blank),
        Kind::Optional(_) | Kind::Any => Value::Null,
    }
}

/// Whether a value fits on a line next to its name.
fn is_simple(kind: &Kind<'_>) -> bool {
    match kind {
        Kind::Bool
        | Kind::Integer { .. }
        | Kind::Number
        | Kind::Text
        | Kind::Choice(_)
        | Kind::Color
        | Kind::Any => true,
        Kind::Optional(inner) => matches!(
            inner.kind(),
            Kind::Bool
                | Kind::Integer { .. }
                | Kind::Number
                | Kind::Text
                | Kind::Choice(_)
                | Kind::Color
        ),
        _ => false,
    }
}

/// A new entry of a list begins as the model's own (`Default`), which
/// names Windows' terminal as the program to start and to look for (a
/// new startup program, a new scratchpad). Where the terminal of this
/// system is another, the new entry names that one: an entry that
/// starts nothing here is no place to begin from. (The model's own stays
/// as it is: what a file leaves out is read the same on every system.)
pub fn new_for_this_system(new: &mut Value, terminal: &str) {
    if terminal.eq_ignore_ascii_case("wt.exe") {
        return;
    }
    match new {
        Value::String(text) => {
            if text.eq_ignore_ascii_case("wt.exe")
                || text.eq_ignore_ascii_case("WindowsTerminal.exe")
            {
                *text = terminal.to_string();
            }
        }
        Value::Array(items) => {
            for item in items {
                new_for_this_system(item, terminal);
            }
        }
        Value::Object(fields) => {
            for (_, field) in fields.iter_mut() {
                new_for_this_system(field, terminal);
            }
        }
        _ => {}
    }
}

/// Whether the text at `pointer` is the name of a workspace: the name a
/// command goes by (`WorkspaceSelector::Name`, which the commands have
/// as `selector` or `workspace`), or what a rule holds a window's
/// workspace against where it asks for exactly that name.
pub fn names_a_workspace(document: &dyn Store, pointer: &str) -> bool {
    let Some((parent, "value")) = pointer.rsplit_once('/') else {
        return false;
    };
    if parent.ends_with("/selector") || parent.ends_with("/workspace") {
        // (Of its kinds only the name is a text.)
        return document
            .get(parent)
            .is_some_and(|selector| selector["kind"] == "name");
    }
    parent.contains("/predicates/")
        && document.get(parent).is_some_and(|predicate| {
            predicate["kind"] == "Workspace" && predicate["strategy"] == "Equals"
        })
}

/// The workspaces the configuration has, each once, in their order: the
/// name it is known by, and what is shown of it (its title, or the name
/// where it has none). Empty where the store is not a whole
/// configuration.
pub fn workspaces_of(document: &dyn Store) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for entry in document
        .get("/desktop/workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = entry["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let title = entry["title"].as_str().unwrap_or("").trim();
        match found
            .iter_mut()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
        {
            // (A display's own entry may be the one with the title.)
            Some((known, shown)) => {
                if shown == known && !title.is_empty() {
                    *shown = title.to_string();
                }
            }
            None => found.push((
                name.to_string(),
                if title.is_empty() { name } else { title }.to_string(),
            )),
        }
    }
    found
}

/// What the list shows of a workspace: its title, and its name beside it
/// where that is another.
pub fn workspace_line(name: &str, shown: &str) -> String {
    if shown == name {
        name.to_string()
    } else {
        format!("{shown}  ({name})")
    }
}

/// The control for a workspace's name: the one that is chosen, and the
/// others on a press. A name that none of them has is shown as it is,
/// and said to be none of them (a workspace of the moment may have it:
/// those go by their numbers). The pencil beside it is for such a name.
fn workspace_control(ui: &mut Ui, workspaces: &[(String, String)], current: &Value) -> Option<Value> {
    let name = current.as_str().unwrap_or("").to_string();
    let typed = ui.scope_id().with("a workspace's name is typed");
    let mut typing = ui.data(|data| data.get_temp::<bool>(typed)).unwrap_or(false);
    let mut new = None;
    ui.horizontal(|ui| {
        if typing {
            let mut text = name.clone();
            if widgets::text_field(ui, &mut text, 190.0).changed() {
                new = Some(Value::String(text));
            }
            if widgets::small_icon_button(ui, icons::CARET_DOWN, "Choose from the workspaces")
                .clicked()
            {
                typing = false;
            }
            return;
        }
        let mut options: Vec<(String, String, String)> = workspaces
            .iter()
            .map(|(name, shown)| (name.clone(), workspace_line(name, shown), String::new()))
            .collect();
        let known = options
            .iter()
            .find(|(known, _, _)| known.eq_ignore_ascii_case(&name))
            .map(|(known, _, _)| known.clone());
        let chosen = match known {
            Some(known) => known,
            None => {
                options.push((
                    name.clone(),
                    if name.trim().is_empty() {
                        "(none chosen)".to_string()
                    } else {
                        format!("{name}  (not one of them)")
                    },
                    "No workspace of the configuration has this name.".to_string(),
                ));
                name.clone()
            }
        };
        if let Some(picked) = widgets::dropdown(ui, "workspace", &chosen, &options, 220.0) {
            new = Some(Value::String(picked));
        }
        if widgets::small_icon_button(ui, icons::PENCIL_SIMPLE, "Type a name").clicked() {
            typing = true;
        }
    });
    ui.data_mut(|data| data.insert_temp(typed, typing));
    new
}

/// What a text names that is a place on the machine, by where it stands
/// in the configuration: a picture's file, a folder. `None` for a text
/// that is no path (or one that need not be there: a program by its
/// name is looked for where the system looks).
pub fn path_kind(pointer: &str) -> Option<widgets::PathKind> {
    let last = pointer.rsplit('/').next().unwrap_or_default();
    match last {
        "image" if pointer.starts_with("/desktop/wallpaper") => Some(widgets::PathKind::File),
        "folder" | "working_directory" => Some(widgets::PathKind::Folder),
        // (A program to start: a file, or its name alone.)
        "program" => Some(widgets::PathKind::Program),
        _ => None,
    }
}

/// The control for a value on a line. Returns the new value when it was
/// changed.
fn control(ui: &mut Ui, pointer: &str, kind: &Kind<'_>, current: &Value) -> Option<Value> {
    match kind {
        Kind::Bool => {
            let mut on = current.as_bool().unwrap_or(false);
            widgets::switch(ui, &mut on)
                .changed()
                .then(|| Value::Bool(on))
        }
        Kind::Integer { min, max } => {
            let mut number = current.as_i64().unwrap_or(0);
            let changed = match hint(pointer) {
                Some(hint) => ui
                    .add(
                        widgets::Slider::new(&mut number, hint.min as i64..=hint.max as i64)
                            .suffix(suffix(hint.unit))
                            .clamping(egui::SliderClamping::Never),
                    )
                    .changed(),
                None => widgets::number_box(ui, &mut number, 1.0).changed(),
            };
            let number = number.clamp(min.unwrap_or(i64::MIN), max.unwrap_or(i64::MAX));
            changed.then(|| Value::from(number))
        }
        Kind::Number => {
            let mut number = current.as_f64().unwrap_or(0.0);
            let changed = match hint(pointer) {
                Some(hint) => ui
                    .add(
                        widgets::Slider::new(&mut number, hint.min..=hint.max)
                            .suffix(suffix(hint.unit))
                            .max_decimals(decimals(hint.step))
                            .clamping(egui::SliderClamping::Never),
                    )
                    .changed(),
                None => widgets::number_box(ui, &mut number, 0.05).changed(),
            };
            // Kept to what was meant: 1.25, not 1.2500000000000002.
            let rounded = (number * 10_000.0).round() / 10_000.0;
            changed.then(|| Value::from(rounded))
        }
        Kind::Text => {
            let mut text = current.as_str().unwrap_or("").to_string();
            // A key's name: chosen by pressing the key.
            if pointer.ends_with("/key") {
                let changed = ui
                    .horizontal(|ui| crate::bindings::key_field(ui, pointer, &mut text))
                    .inner;
                return changed.then(|| Value::String(text));
            }
            let field = match path_kind(pointer) {
                // A place on the machine: said whether it is there.
                Some(kind) => {
                    let home = dirs::home_dir();
                    if kind == widgets::PathKind::File && pointer.starts_with("/desktop/wallpaper") {
                        // (Its browser shows pictures.)
                        widgets::path_field_of(
                            ui,
                            &mut text,
                            kind,
                            home.as_deref(),
                            260.0,
                            "Pictures",
                            &gevrek_ui::browser::PICTURES,
                        )
                    } else {
                        widgets::path_field(ui, &mut text, kind, home.as_deref(), 260.0)
                    }
                }
                None => widgets::text_field(ui, &mut text, 260.0),
            };
            field.changed().then(|| Value::String(text))
        }
        Kind::Choice(words) => {
            let chosen = current.as_str().unwrap_or("");
            let labels: Vec<String> = words.iter().map(|word| title(word.value)).collect();
            let short = words.len() <= 4 && labels.iter().map(String::len).sum::<usize>() <= 34;
            if short {
                let options: Vec<(String, &str)> = words
                    .iter()
                    .zip(&labels)
                    .map(|(word, label)| (word.value.to_string(), label.as_str()))
                    .collect();
                let selected = chosen.to_string();
                return widgets::segmented(
                    ui,
                    ui.scope_id().with("choice"),
                    Some(&selected),
                    &options,
                    false,
                )
                .map(Value::String);
            }
            let options: Vec<(String, String, String)> = words
                .iter()
                .zip(&labels)
                .map(|(word, label)| {
                    (
                        word.value.to_string(),
                        label.clone(),
                        word.description.to_string(),
                    )
                })
                .collect();
            // (A word the list does not have is shown as it is.)
            let mut options = options;
            if !options.iter().any(|(value, _, _)| value == chosen) {
                options.insert(0, (chosen.to_string(), title(chosen), String::new()));
            }
            widgets::dropdown(ui, "choice", &chosen.to_string(), &options, 200.0).map(Value::String)
        }
        Kind::Color => {
            let parts: Vec<u8> = current
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .map(|part| part.as_u64().unwrap_or(0).min(255) as u8)
                        .collect()
                })
                .unwrap_or_default();
            let text = match parts.as_slice() {
                [red, green, blue] => format!("#{red:02x}{green:02x}{blue:02x}"),
                _ => String::new(),
            };
            let new = widgets::color_field(ui, ui.scope_id().with("colour"), Some(&text))?;
            let colour = gevrek_ui::color::parse(&new)?;
            let (red, green, blue) = gevrek_ui::color::to_rgb(colour);
            Some(Value::from(vec![red, green, blue]))
        }
        Kind::Optional(inner) => {
            let mut present = !current.is_null();
            let mut out = None;
            if widgets::checkbox(ui, &mut present, "").changed() {
                out = Some(if present {
                    blank_inside(*inner)
                } else {
                    Value::Null
                });
            }
            if !current.is_null() {
                if let Some(new) = control(ui, pointer, &inner.kind(), current) {
                    out = Some(new);
                }
            } else {
                ui.weak("not set");
            }
            out
        }
        // What this version does not know: shown as it is written.
        _ => {
            ui.label(RichText::new(current.to_string()).monospace().weak());
            None
        }
    }
}

fn suffix(unit: &str) -> String {
    match unit {
        "" => String::new(),
        "×" | "°" | "%" => unit.to_string(),
        other => format!(" {other}"),
    }
}

fn decimals(step: f64) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    }
}

enum ItemAction {
    Remove(usize),
    Up(usize),
    Down(usize),
}

/// The lists whose order is set somewhere better than by an item's two
/// arrows (dragged, on their page): they have no arrows here.
const ORDERED_ELSEWHERE: [&str; 1] = ["/desktop/window_rules"];

fn item_buttons(
    ui: &mut Ui,
    index: usize,
    count: usize,
    ordered_here: bool,
    action: &mut Option<ItemAction>,
) {
    if widgets::small_icon_button(ui, icons::TRASH, "Remove").clicked() {
        *action = Some(ItemAction::Remove(index));
    }
    if !ordered_here {
        return;
    }
    if index + 1 < count && widgets::small_icon_button(ui, icons::ARROW_DOWN, "Move down").clicked()
    {
        *action = Some(ItemAction::Down(index));
    }
    if index > 0 && widgets::small_icon_button(ui, icons::ARROW_UP, "Move up").clicked() {
        *action = Some(ItemAction::Up(index));
    }
}

/// What to call an item of a list: the first thing in it that reads like
/// a name.
pub fn summary(value: &Value, index: usize) -> String {
    const NAMES: [&str; 9] = [
        "name", "title", "id", "label", "key", "program", "app_id", "class", "pattern",
    ];
    if let Some(object) = value.as_object() {
        for name in NAMES {
            if let Some(text) = object
                .get(name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                return text.to_string();
            }
        }
    }
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    format!("{}", index + 1)
}

fn section(ui: &mut Ui, label: &str, description: &str) {
    ui.horizontal(|ui| section_title(ui, label, description));
}

fn section_title(ui: &mut Ui, label: &str, description: &str) {
    let palette = theme::pal(ui.ctx());
    ui.add_space(2.0);
    let response = ui.label(RichText::new(label).strong().color(palette.text));
    if !description.is_empty() {
        response.on_hover_text(description);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
    use gevrek_model::settings::{
        BindingProfile, MonitorSelector, RulePredicate, WindowRuleProfile, WorkspaceProfile,
    };
    use gevrek_model::Config;

    fn document() -> Document {
        let mut config = Config::default();
        let workspace = |name: &str, title: Option<&str>, monitor: MonitorSelector| WorkspaceProfile {
            name: name.to_string(),
            title: title.map(str::to_string),
            monitor,
            ..WorkspaceProfile::default()
        };
        config.desktop.workspaces = vec![
            workspace("web", None, MonitorSelector::Any),
            workspace("code", Some("Work"), MonitorSelector::Any),
            // A display's own entry of one that is there already.
            workspace("web", Some("Browsing"), MonitorSelector::Name("DELL".to_string())),
        ];
        config.desktop.bindings = vec![
            BindingProfile {
                command: DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Name("code".to_string()),
                },
                ..BindingProfile::default()
            },
            BindingProfile {
                command: DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Next,
                },
                ..BindingProfile::default()
            },
        ];
        config.desktop.window_rules = vec![WindowRuleProfile {
            predicates: vec![
                RulePredicate {
                    kind: gevrek_model::rules::MatchKind::Workspace,
                    strategy: gevrek_model::rules::MatchStrategy::Equals,
                    value: "web".to_string(),
                    ..RulePredicate::default()
                },
                RulePredicate {
                    kind: gevrek_model::rules::MatchKind::Workspace,
                    strategy: gevrek_model::rules::MatchStrategy::Regex,
                    value: "w.*".to_string(),
                    ..RulePredicate::default()
                },
                RulePredicate {
                    kind: gevrek_model::rules::MatchKind::Title,
                    strategy: gevrek_model::rules::MatchStrategy::Equals,
                    value: "web".to_string(),
                    ..RulePredicate::default()
                },
            ],
            ..WindowRuleProfile::default()
        }];
        Document::new(&config)
    }

    #[test]
    fn the_workspaces_to_choose_from_are_each_there_once_with_what_is_shown_of_them() {
        let document = document();
        assert_eq!(
            workspaces_of(&document),
            vec![
                ("web".to_string(), "Browsing".to_string()),
                ("code".to_string(), "Work".to_string()),
            ]
        );
        assert_eq!(workspace_line("code", "Work"), "Work  (code)");
        assert_eq!(workspace_line("3", "3"), "3");
    }

    #[test]
    fn a_new_entry_names_the_terminal_of_the_system_it_is_made_on() {
        use gevrek_model::settings::{ScratchpadProfile, StartupProfile};
        let startup = serde_json::to_value(StartupProfile::default()).unwrap();
        let scratchpad = serde_json::to_value(ScratchpadProfile::default()).unwrap();
        // On Windows they are as the model has them.
        let mut here = startup.clone();
        new_for_this_system(&mut here, "wt.exe");
        assert_eq!(here, startup);
        // Elsewhere the program that is started, and the one looked for,
        // is that system's terminal; nothing else is another.
        let mut there = startup.clone();
        new_for_this_system(&mut there, "kitty");
        assert_eq!(there["launch"]["program"], "kitty");
        assert_eq!(there["name"], startup["name"]);
        let mut pad = scratchpad.clone();
        new_for_this_system(&mut pad, "kitty");
        assert_eq!(pad["launch"]["program"], "kitty");
        assert_eq!(pad["pattern"], "kitty");
        assert_eq!(pad["width_percent"], scratchpad["width_percent"]);
        assert!(!pad.to_string().to_lowercase().contains("wt.exe"));
        assert!(!pad.to_string().contains("WindowsTerminal"));
    }

    #[test]
    fn a_workspaces_name_is_known_by_where_it_stands() {
        let document = document();
        // The name a key's command goes by.
        let named = "/desktop/bindings/0/command/selector/value";
        assert_eq!(document.get(named), Some(&Value::from("code")));
        assert!(names_a_workspace(&document, named));
        // A selector that is no name has no text to choose.
        assert!(!names_a_workspace(
            &document,
            "/desktop/bindings/1/command/selector/value"
        ));
        // What a rule holds a window's workspace against: exactly that
        // name, not a pattern, and not what it holds a title against.
        let rule = "/desktop/window_rules/0/predicates";
        assert!(names_a_workspace(&document, &format!("{rule}/0/value")));
        assert!(!names_a_workspace(&document, &format!("{rule}/1/value")));
        assert!(!names_a_workspace(&document, &format!("{rule}/2/value")));
        assert!(!names_a_workspace(&document, "/desktop/workspaces/0/name"));
    }
}
