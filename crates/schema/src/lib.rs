//! What the configuration is made of, as data.
//!
//! The model's types carry their own description: every one derives a JSON
//! Schema, with the documentation of each field as its description, its
//! default, the words of each enum. This crate turns that schema into what
//! a program can build on:
//!
//! - [`schema`] is the schema itself (`gevrek schema` prints it; an editor
//!   uses it to complete and check a configuration file);
//! - [`Node`] reads one place of it as a [`Kind`]: a switch, a number, a
//!   choice, a colour, a list, one of several variants. The studio draws
//!   its forms from that, so a setting added to the model has a form the
//!   moment it exists;
//! - [`catalogue`] lists every setting with its path, for search;
//! - [`hint`] says what range and unit a number has, which the types do
//!   not know.
//!
//! Nothing here can drift from the model: the tests read gevrek's own
//! configurations through it and fail on a value no [`Kind`] explains.

use serde_json::Value;
use std::sync::OnceLock;

mod hints;
pub use hints::{hint, waits, Hint};

/// The JSON Schema of a configuration file.
pub struct Schema {
    root: Value,
}

/// The schema of `gevrek_model::Config`, built once.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| Schema {
        root: serde_json::to_value(schemars::schema_for!(gevrek_model::Config))
            .expect("a schema is JSON"),
    })
}

impl Schema {
    /// The whole document, as JSON Schema (draft 2020-12).
    pub fn document(&self) -> &Value {
        &self.root
    }

    /// The configuration as a whole.
    pub fn root(&self) -> Node<'_> {
        Node::new(self, &self.root)
    }

    /// The place a JSON pointer (`/desktop/bar/height`) names. Steps
    /// through objects, optional values, lists (any index), maps (any key)
    /// and tuples.
    pub fn at(&self, pointer: &str) -> Option<Node<'_>> {
        let mut node = self.root();
        for step in pointer.split('/').skip(1) {
            node = node.child(step)?;
        }
        Some(node)
    }

    fn definition(&self, reference: &str) -> Option<&Value> {
        let name = reference.strip_prefix("#/$defs/")?;
        self.root.get("$defs")?.get(name)
    }
}

/// One place in the schema.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    schema: &'a Schema,
    /// The place as written (it may be a reference with a description of
    /// its own).
    outer: &'a Value,
    /// What it refers to.
    inner: &'a Value,
    /// Read as if it could not be `null` (the inside of an optional value
    /// that the schema writes as `["integer", "null"]`).
    never_null: bool,
}

/// One word of a choice.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice<'a> {
    pub value: &'a str,
    pub description: &'a str,
}

/// A named part of an object.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub name: &'a str,
    pub node: Node<'a>,
    /// The object is not valid without it.
    pub required: bool,
}

/// One of the forms a value of several can take.
#[derive(Clone)]
pub struct Variant<'a> {
    /// The word that names it in the file.
    pub name: &'a str,
    pub description: &'a str,
    pub shape: VariantShape<'a>,
}

#[derive(Clone)]
pub enum VariantShape<'a> {
    /// The value is just the word: `"dwindle"`.
    Word,
    /// An object that says which it is in one key, next to its own
    /// fields: `{"command": "exec", "program": "foot"}`.
    Tagged {
        key: &'a str,
        fields: Vec<Field<'a>>,
    },
    /// An object with the word as its only key: `{"index": 2}`.
    Wrapped(Node<'a>),
}

/// What a place holds, as a form would ask.
#[derive(Clone)]
pub enum Kind<'a> {
    Bool,
    /// A whole number, within what its type can hold.
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Number,
    Text,
    /// One of a few words.
    Choice(Vec<Choice<'a>>),
    /// Red, green and blue, 0 to 255 each.
    Color,
    /// The value, or nothing (`null`).
    Optional(Node<'a>),
    Object(Vec<Field<'a>>),
    List(Node<'a>),
    /// Named values of one kind.
    Map(Node<'a>),
    /// A fixed number of values in a row.
    Tuple(Vec<Node<'a>>),
    /// One of several forms.
    Union(Vec<Variant<'a>>),
    /// Any JSON (keys gevrek does not know are kept this way).
    Any,
}

impl<'a> Node<'a> {
    fn new(schema: &'a Schema, outer: &'a Value) -> Self {
        let mut inner = outer;
        // A reference, also when it is wrapped to carry a description.
        for _ in 0..16 {
            let reference = inner.get("$ref").and_then(Value::as_str).or_else(|| {
                let all = inner.get("allOf")?.as_array()?;
                (all.len() == 1)
                    .then(|| all[0].get("$ref")?.as_str())
                    .flatten()
            });
            match reference.and_then(|reference| schema.definition(reference)) {
                Some(target) => inner = target,
                None => break,
            }
        }
        Self {
            schema,
            outer,
            inner,
            never_null: false,
        }
    }

    /// What the place is for, from the documentation of the field, or of
    /// its type when the field has none.
    pub fn description(&self) -> &'a str {
        let own = self.outer.get("description").and_then(Value::as_str);
        own.or_else(|| self.inner.get("description").and_then(Value::as_str))
            .unwrap_or("")
    }

    /// The value the model gives the place when the file has none.
    pub fn default(&self) -> Option<&'a Value> {
        self.outer
            .get("default")
            .or_else(|| self.inner.get("default"))
    }

    /// The name of the type behind the place, when it has one of its own
    /// (`BarConfig`).
    pub fn type_name(&self) -> Option<&'a str> {
        let reference = self.outer.get("$ref").and_then(Value::as_str).or_else(|| {
            self.outer
                .get("allOf")?
                .as_array()?
                .first()?
                .get("$ref")?
                .as_str()
        })?;
        reference.strip_prefix("#/$defs/")
    }

    pub fn kind(&self) -> Kind<'a> {
        let node = self.inner;
        if let Some(options) = node.get("anyOf").and_then(Value::as_array) {
            let (nulls, others): (Vec<&Value>, Vec<&Value>) =
                options.iter().partition(|option| is_null(option));
            if !nulls.is_empty() && others.len() == 1 {
                return Kind::Optional(Node::new(self.schema, others[0]));
            }
            // Words only, as an enum with a variant that is left out of
            // the schema is written: a choice all the same.
            if nulls.is_empty() {
                if let Kind::Choice(words) = self.one_of(options) {
                    return Kind::Choice(words);
                }
            }
            return Kind::Any;
        }
        if let Some(options) = node.get("oneOf").and_then(Value::as_array) {
            return self.one_of(options);
        }
        if let Some(words) = node.get("enum").and_then(Value::as_array) {
            return Kind::Choice(
                words
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|value| Choice {
                        value,
                        description: "",
                    })
                    .collect(),
            );
        }
        match node.get("type") {
            // `["integer", "null"]`: the value, or nothing.
            Some(Value::Array(types)) => {
                let named: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
                let others: Vec<&&str> = named.iter().filter(|name| **name != "null").collect();
                match others.as_slice() {
                    [name] if self.never_null || !named.contains(&"null") => self.typed(name),
                    [_] => Kind::Optional(Node {
                        never_null: true,
                        ..*self
                    }),
                    _ => Kind::Any,
                }
            }
            Some(Value::String(name)) => self.typed(name),
            _ => Kind::Any,
        }
    }

    fn typed(&self, name: &str) -> Kind<'a> {
        let node = self.inner;
        match name {
            "boolean" => Kind::Bool,
            "integer" => {
                let (low, high) = match node.get("format").and_then(Value::as_str) {
                    Some("uint8") => (Some(0), Some(i64::from(u8::MAX))),
                    Some("uint16") => (Some(0), Some(i64::from(u16::MAX))),
                    Some("uint32") => (Some(0), Some(i64::from(u32::MAX))),
                    Some("uint" | "uint64") => (Some(0), None),
                    Some("int8") => (Some(i64::from(i8::MIN)), Some(i64::from(i8::MAX))),
                    Some("int16") => (Some(i64::from(i16::MIN)), Some(i64::from(i16::MAX))),
                    Some("int32") => (Some(i64::from(i32::MIN)), Some(i64::from(i32::MAX))),
                    _ => (None, None),
                };
                Kind::Integer {
                    min: node.get("minimum").and_then(Value::as_i64).or(low),
                    max: node.get("maximum").and_then(Value::as_i64).or(high),
                }
            }
            "number" => Kind::Number,
            "string" => match node.get("const").and_then(Value::as_str) {
                Some(value) => Kind::Choice(vec![Choice {
                    value,
                    description: self.description(),
                }]),
                None => Kind::Text,
            },
            "array" => {
                if let Some(parts) = node.get("prefixItems").and_then(Value::as_array) {
                    let bytes = parts
                        .iter()
                        .all(|part| part.get("format").and_then(Value::as_str) == Some("uint8"));
                    if parts.len() == 3 && bytes {
                        return Kind::Color;
                    }
                    return Kind::Tuple(
                        parts
                            .iter()
                            .map(|part| Node::new(self.schema, part))
                            .collect(),
                    );
                }
                match node.get("items") {
                    Some(item) if item.is_object() => Kind::List(Node::new(self.schema, item)),
                    _ => Kind::Any,
                }
            }
            "object" => {
                let fields = self.fields();
                if !fields.is_empty() {
                    return Kind::Object(fields);
                }
                match node.get("additionalProperties") {
                    Some(values) if values.is_object() => Kind::Map(Node::new(self.schema, values)),
                    _ => Kind::Any,
                }
            }
            _ => Kind::Any,
        }
    }

    fn fields(&self) -> Vec<Field<'a>> {
        let required: Vec<&str> = self
            .inner
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        self.inner
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| {
                properties
                    .iter()
                    .map(|(name, node)| Field {
                        name: name.as_str(),
                        node: Node::new(self.schema, node),
                        required: required.contains(&name.as_str()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `oneOf`: a choice of words, or a value that takes one of several
    /// forms.
    fn one_of(&self, options: &'a [Value]) -> Kind<'a> {
        // Words only (some with a description of their own): a choice.
        let mut words = Vec::new();
        let mut only_words = true;
        for option in options {
            let description = option
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(value) = option.get("const").and_then(Value::as_str) {
                words.push(Choice { value, description });
            } else if let Some(values) = option.get("enum").and_then(Value::as_array) {
                words.extend(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|value| Choice { value, description }),
                );
            } else {
                only_words = false;
            }
        }
        if only_words {
            return Kind::Choice(words);
        }
        // The key every object form names itself in, if there is one.
        let objects: Vec<&Value> = options
            .iter()
            .filter(|option| option.get("properties").is_some())
            .collect();
        let tag = objects.first().and_then(|first| {
            first
                .get("properties")?
                .as_object()?
                .iter()
                .find_map(|(key, property)| {
                    let shared = property.get("const").is_some()
                        && objects.iter().all(|object| {
                            object
                                .get("properties")
                                .and_then(|properties| properties.get(key))
                                .is_some_and(|property| property.get("const").is_some())
                        });
                    shared.then_some(key.as_str())
                })
        });
        let mut variants = Vec::new();
        for option in options {
            let description = option
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(name) = option.get("const").and_then(Value::as_str) {
                variants.push(Variant {
                    name,
                    description,
                    shape: VariantShape::Word,
                });
                continue;
            }
            if let Some(values) = option.get("enum").and_then(Value::as_array) {
                for name in values.iter().filter_map(Value::as_str) {
                    variants.push(Variant {
                        name,
                        description,
                        shape: VariantShape::Word,
                    });
                }
                continue;
            }
            let node = Node::new(self.schema, option);
            let fields = node.fields();
            match tag {
                Some(key) => {
                    let Some(name) = fields
                        .iter()
                        .find(|field| field.name == key)
                        .and_then(|field| field.node.inner.get("const"))
                        .and_then(Value::as_str)
                    else {
                        return Kind::Any;
                    };
                    variants.push(Variant {
                        name,
                        description,
                        shape: VariantShape::Tagged {
                            key,
                            fields: fields
                                .into_iter()
                                .filter(|field| field.name != key)
                                .collect(),
                        },
                    });
                }
                None => {
                    // `{"word": value}`: one key, which is the word.
                    let [field] = fields.as_slice() else {
                        return Kind::Any;
                    };
                    variants.push(Variant {
                        name: field.name,
                        description,
                        shape: VariantShape::Wrapped(field.node),
                    });
                }
            }
        }
        Kind::Union(variants)
    }

    /// The place one step further: a field, an item of a list, a value of
    /// a map, a part of a tuple, a field of any variant.
    pub fn child(&self, step: &str) -> Option<Node<'a>> {
        match self.kind() {
            Kind::Optional(inner) => inner.child(step),
            Kind::Object(fields) => fields
                .into_iter()
                .find(|field| field.name == step)
                .map(|field| field.node),
            Kind::List(item) | Kind::Map(item) => Some(item),
            Kind::Tuple(parts) => parts.get(step.parse::<usize>().ok()?).copied(),
            Kind::Union(variants) => variants
                .into_iter()
                .find_map(|variant| match variant.shape {
                    VariantShape::Tagged { fields, .. } => fields
                        .into_iter()
                        .find(|field| field.name == step)
                        .map(|field| field.node),
                    VariantShape::Wrapped(inner) if variant.name == step => Some(inner),
                    _ => None,
                }),
            _ => None,
        }
    }

    /// A value that is valid here and says as little as possible: the
    /// model's default when it has one, else the emptiest value of the
    /// kind. For a new item of a list, or a variant just chosen.
    pub fn blank(&self) -> Value {
        if let Some(default) = self
            .default()
            .filter(|default| !(self.never_null && default.is_null()))
        {
            return default.clone();
        }
        match self.kind() {
            Kind::Bool => Value::Bool(false),
            Kind::Integer { min, .. } => Value::from(min.unwrap_or(0).max(0)),
            Kind::Number => Value::from(0.0),
            Kind::Text => Value::String(String::new()),
            Kind::Choice(words) => words
                .first()
                .map_or(Value::Null, |word| Value::from(word.value)),
            Kind::Color => Value::from(vec![128, 128, 128]),
            Kind::Optional(_) | Kind::Any => Value::Null,
            Kind::Object(fields) => Value::Object(
                fields
                    .iter()
                    .filter(|field| field.required)
                    .map(|field| (field.name.to_string(), field.node.blank()))
                    .collect(),
            ),
            Kind::List(_) => Value::Array(Vec::new()),
            Kind::Map(_) => Value::Object(Default::default()),
            Kind::Tuple(parts) => Value::Array(parts.iter().map(Node::blank).collect()),
            Kind::Union(variants) => variants.first().map_or(Value::Null, Variant::blank),
        }
    }
}

fn is_null(node: &Value) -> bool {
    node.get("type").and_then(Value::as_str) == Some("null")
}

impl<'a> Variant<'a> {
    /// The emptiest value of this form.
    pub fn blank(&self) -> Value {
        match &self.shape {
            VariantShape::Word => Value::from(self.name),
            VariantShape::Tagged { key, fields } => {
                let mut object = serde_json::Map::new();
                object.insert((*key).to_string(), Value::from(self.name));
                for field in fields.iter().filter(|field| field.required) {
                    object.insert(field.name.to_string(), field.node.blank());
                }
                Value::Object(object)
            }
            VariantShape::Wrapped(inner) => {
                let mut object = serde_json::Map::new();
                object.insert(self.name.to_string(), inner.blank());
                Value::Object(object)
            }
        }
    }

    /// Whether a value has this form.
    pub fn matches(&self, value: &Value) -> bool {
        match &self.shape {
            VariantShape::Word => value.as_str() == Some(self.name),
            VariantShape::Tagged { key, .. } => {
                value.get(key).and_then(Value::as_str) == Some(self.name)
            }
            VariantShape::Wrapped(_) => value
                .as_object()
                .is_some_and(|object| object.len() == 1 && object.contains_key(self.name)),
        }
    }
}

/// A name as a person reads it: `corner_radius` is "Corner radius".
pub fn title(name: &str) -> String {
    let mut words: Vec<String> = name
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .map(|word| match word {
            "ms" => "(ms)".to_string(),
            "fps" => "FPS".to_string(),
            "x11" => "X11".to_string(),
            "xwayland" => "Xwayland".to_string(),
            "id" => "ID".to_string(),
            "led" | "leds" => word.to_ascii_uppercase().replace("LEDS", "LEDs"),
            other => other.to_string(),
        })
        .collect();
    if let Some(first) = words.first_mut() {
        let mut letters = first.chars();
        if let Some(letter) = letters.next() {
            *first = letter.to_uppercase().chain(letters).collect();
        }
    }
    words.join(" ")
}

/// One setting, for a list to search in.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// Where it is in the file: `/desktop/bar/height`.
    pub pointer: String,
    pub title: String,
    pub description: String,
    /// The titles of the objects it lies in, from the outside in.
    pub group: Vec<String>,
    pub kind: &'static str,
}

/// Every setting that has a fixed place in the file: the fields of the
/// configuration and of the objects in it. What lies in a list, a map or a
/// variant has no place of its own; the list itself is the entry.
pub fn catalogue() -> Vec<Setting> {
    let mut found = Vec::new();
    walk(
        schema().root(),
        String::new(),
        &mut Vec::new(),
        &mut found,
        0,
    );
    found
}

fn walk(
    node: Node<'_>,
    pointer: String,
    group: &mut Vec<String>,
    found: &mut Vec<Setting>,
    depth: usize,
) {
    let kind = node.kind();
    let inner = match &kind {
        Kind::Optional(inner) => inner.kind(),
        _ => kind.clone(),
    };
    if let Kind::Object(fields) = inner {
        if depth > 12 {
            return;
        }
        for field in fields {
            let name = title(field.name);
            let place = format!("{pointer}/{}", field.name);
            let is_object = matches!(field.node.kind(), Kind::Object(_))
                || matches!(field.node.kind(), Kind::Optional(inner) if matches!(inner.kind(), Kind::Object(_)));
            if is_object {
                group.push(name);
                walk(field.node, place, group, found, depth + 1);
                group.pop();
            } else {
                found.push(Setting {
                    pointer: place,
                    title: name,
                    description: field.node.description().to_string(),
                    group: group.clone(),
                    kind: kind_name(&field.node.kind()),
                });
            }
        }
    }
}

fn kind_name(kind: &Kind<'_>) -> &'static str {
    match kind {
        Kind::Bool => "switch",
        Kind::Integer { .. } | Kind::Number => "number",
        Kind::Text => "text",
        Kind::Choice(_) => "choice",
        Kind::Color => "colour",
        Kind::Optional(inner) => kind_name(&inner.kind()),
        Kind::Object(_) => "group",
        Kind::List(_) => "list",
        Kind::Map(_) => "map",
        Kind::Tuple(_) => "values",
        Kind::Union(_) => "variant",
        Kind::Any => "any",
    }
}

/// What in `value` no [`Kind`] of `node` explains, as `(pointer, what)`.
/// Empty for every configuration the model itself writes.
pub fn unexplained(node: Node<'_>, value: &Value) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    explain(node, value, String::new(), &mut problems);
    problems
}

fn explain(node: Node<'_>, value: &Value, pointer: String, problems: &mut Vec<(String, String)>) {
    let mut wrong = |what: &str| problems.push((pointer.clone(), format!("{what}, found {value}")));
    match node.kind() {
        Kind::Any => {}
        Kind::Bool => {
            if !value.is_boolean() {
                wrong("a switch");
            }
        }
        Kind::Integer { min, max } => match value.as_i64() {
            Some(number)
                if min.is_none_or(|min| number >= min) && max.is_none_or(|max| number <= max) => {}
            _ if value.is_u64() && max.is_none() => {}
            _ => wrong("a whole number in range"),
        },
        Kind::Number => {
            if !value.is_number() {
                wrong("a number");
            }
        }
        Kind::Text => {
            if !value.is_string() {
                wrong("text");
            }
        }
        Kind::Choice(words) => {
            if !value
                .as_str()
                .is_some_and(|text| words.iter().any(|word| word.value == text))
            {
                wrong("one of the words");
            }
        }
        Kind::Color => {
            let fine = value.as_array().is_some_and(|parts| {
                parts.len() == 3
                    && parts
                        .iter()
                        .all(|part| part.as_u64().is_some_and(|byte| byte <= 255))
            });
            if !fine {
                wrong("a colour");
            }
        }
        Kind::Optional(inner) => {
            if !value.is_null() {
                explain(inner, value, pointer, problems);
            }
        }
        Kind::Object(fields) => {
            let Some(object) = value.as_object() else {
                return wrong("an object");
            };
            for field in &fields {
                match object.get(field.name) {
                    Some(inner) => explain(
                        field.node,
                        inner,
                        format!("{pointer}/{}", field.name),
                        problems,
                    ),
                    None if field.required => {
                        problems.push((pointer.clone(), format!("`{}` is missing", field.name)))
                    }
                    None => {}
                }
            }
        }
        Kind::List(item) => {
            let Some(items) = value.as_array() else {
                return wrong("a list");
            };
            for (index, inner) in items.iter().enumerate() {
                explain(item, inner, format!("{pointer}/{index}"), problems);
            }
        }
        Kind::Map(item) => {
            let Some(object) = value.as_object() else {
                return wrong("a map");
            };
            for (key, inner) in object {
                explain(item, inner, format!("{pointer}/{key}"), problems);
            }
        }
        Kind::Tuple(parts) => {
            let Some(items) = value.as_array().filter(|items| items.len() == parts.len()) else {
                return wrong("values in a row");
            };
            for (index, (part, inner)) in parts.iter().zip(items).enumerate() {
                explain(*part, inner, format!("{pointer}/{index}"), problems);
            }
        }
        Kind::Union(variants) => {
            let Some(variant) = variants.iter().find(|variant| variant.matches(value)) else {
                return wrong("one of the forms");
            };
            match &variant.shape {
                VariantShape::Word => {}
                VariantShape::Tagged { fields, .. } => {
                    for field in fields {
                        match value.get(field.name) {
                            Some(inner) => explain(
                                field.node,
                                inner,
                                format!("{pointer}/{}", field.name),
                                problems,
                            ),
                            None if field.required => problems
                                .push((pointer.clone(), format!("`{}` is missing", field.name))),
                            None => {}
                        }
                    }
                }
                VariantShape::Wrapped(inner) => explain(
                    *inner,
                    &value[variant.name],
                    format!("{pointer}/{}", variant.name),
                    problems,
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests;
