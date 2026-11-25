//! gevrek-model: backend-neutral data shared by the engine, backends, IPC,
//! and the editor. Serde types only: no Windows, egui, or tokio.
//! See docs/PHASE2_PLAN.md (step 1).

/// Reading and writing for an enum with an `Unknown(serde_json::Value)`
/// variant: a value the enum's own reader does not accept becomes `Unknown`
/// holding the JSON as it was, and is written back as that JSON. The enum
/// derives its reader and writer with `#[serde(remote = "Self")]`, which
/// makes them plain functions these impls call.
#[macro_export]
macro_rules! tolerant_enum {
    ($ty:ident) => {
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    $ty::Unknown(raw) => serde::Serialize::serialize(raw, serializer),
                    known => $ty::serialize(known, serializer),
                }
            }
        }
        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
                Ok($ty::deserialize(&raw).unwrap_or($ty::Unknown(raw)))
            }
        }
    };
}

pub mod command;
pub mod config;
pub mod examples;
pub mod files;
pub mod focus;
pub mod geometry;
pub mod graph;
pub mod graph_layout;
pub mod ids;
pub mod input;
pub mod keys;
pub mod layer_rules;
pub mod layout;
pub mod migrate;
pub mod night_light;
pub mod outputs;
pub mod paths;
pub mod platform;
pub mod rules;
pub mod session;
pub mod settings;
pub mod shortcuts;
pub mod snap_zones;
pub mod theme;
pub mod wallpaper;
pub mod window;
pub mod zones;
// What a display says of itself, and the id made of it (std only).
pub mod edid;

pub use config::Config;

/// What a configuration section holds beyond the fields this version knows
/// (see the `extra` field of every section). A map of the unknown keys as
/// they were read.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Extra(pub std::collections::BTreeMap<String, serde_json::Value>);

// JSON values hold floats, so the map is not `Eq` by itself; unknown keys are
// only ever carried, never computed with.
impl Eq for Extra {}

// Written by hand: derived, the description of this type would replace the
// description of every section it is flattened into.
impl schemars::JsonSchema for Extra {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Extra".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "object", "additionalProperties": true })
    }
}

impl Extra {
    /// No unknown keys.
    pub const fn new() -> Self {
        Self(std::collections::BTreeMap::new())
    }
}

/// Write a hash map with its keys in order (see `NodeGraph::nodes`).
pub(crate) fn ordered_map<K, V, S>(
    map: &std::collections::HashMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    K: Ord + serde::Serialize,
    V: serde::Serialize,
    S: serde::Serializer,
{
    serializer.collect_map(map.iter().collect::<std::collections::BTreeMap<_, _>>())
}

/// Write a hash set with its values in order.
pub(crate) fn ordered_set<T, S>(
    set: &std::collections::HashSet<T>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    T: Ord + serde::Serialize,
    S: serde::Serializer,
{
    serializer.collect_seq(set.iter().collect::<std::collections::BTreeSet<_>>())
}
