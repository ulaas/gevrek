//! A whole layout as one document, to carry to another machine or to
//! keep beside a backup: the zones and rules as they were configured, and
//! which item is where on which displays. It is JSON that says what it is
//! (`format`) and of which shape (`version`).
//!
//! Reading one changes nothing by itself. It is looked at first (`read`):
//! a document of another kind, of a newer shape, or one whose zones could
//! not be right is refused with the reason, and the layout there is stays.

use crate::config::ZonesConfig;
use crate::state::Snapshot;
use gevrek_model::Extra;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a layout document says it is.
pub const LAYOUT_FORMAT: &str = "gevrek-zones-layout";

/// The shape of a layout document. One of a higher number is from a
/// newer gevrek and is refused; a field that is merely new does not
/// change the number.
pub const LAYOUT_VERSION: u32 = 1;

/// A layout, on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutDocument {
    /// Always `LAYOUT_FORMAT`.
    pub format: String,
    pub version: u32,
    /// When it was written, in seconds since 1970.
    #[serde(default)]
    pub exported_at: u64,
    /// The zones, the rules, the placements and the arrangements. Its
    /// name is the layout's.
    pub layout: Snapshot,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: Extra,
}

impl LayoutDocument {
    /// A layout as a document, written at `now`.
    pub fn of(layout: Snapshot, now: u64) -> Self {
        Self {
            format: LAYOUT_FORMAT.to_string(),
            version: LAYOUT_VERSION,
            exported_at: now,
            layout: Snapshot {
                automatic: false,
                ..layout
            },
            extra: Extra::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// Read a layout document and look it over. The reason it cannot be used
/// is said in words a person can act on.
pub fn read(document: &Value) -> Result<LayoutDocument, String> {
    let format = document.get("format").and_then(Value::as_str);
    if format != Some(LAYOUT_FORMAT) {
        return Err(format!(
            "this is no layout of the zones (its \"format\" is not \"{LAYOUT_FORMAT}\")"
        ));
    }
    let version = document
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("the layout does not say which version it is")?;
    if version == 0 || version > u64::from(LAYOUT_VERSION) {
        return Err(format!(
            "the layout is of version {version}, and this gevrek reads version {LAYOUT_VERSION}"
        ));
    }
    let read: LayoutDocument = serde_json::from_value(document.clone())
        .map_err(|error| format!("the layout cannot be read: {error}"))?;
    if read.layout.name.trim().is_empty() {
        return Err("the layout has no name".to_string());
    }
    // The zones and rules are held to what the configuration is held to.
    let as_configured = ZonesConfig {
        zones: read.layout.zones.clone(),
        rules: read.layout.rules.clone(),
        catch_all: read.layout.catch_all.clone(),
        ..ZonesConfig::default()
    };
    if let Some((path, message)) = as_configured.problems().into_iter().next() {
        let path = path
            .strip_prefix("desktop.zones.")
            .unwrap_or(&path)
            .to_string();
        return Err(format!("the layout cannot be right at {path}: {message}"));
    }
    if let Some(broken) = read.layout.placements.broken() {
        return Err(format!("the layout cannot be right: {broken}"));
    }
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZoneDefinition;
    use crate::ids::{ItemId, ZoneId};

    fn layout() -> Snapshot {
        let mut layout = Snapshot {
            name: "Work".to_string(),
            taken_at: 1_800_000_000,
            zones: vec![ZoneDefinition::manual("papers", "Papers")],
            automatic: true,
            ..Snapshot::default()
        };
        layout.placements.assign(
            &ItemId::new("/desk/a.pdf"),
            &ZoneId::new("papers"),
            None,
            None,
        );
        layout
    }

    #[test]
    fn a_layout_survives_being_written_and_read() {
        let document = LayoutDocument::of(layout(), 1_800_000_100);
        let value = document.to_value();
        assert_eq!(value["format"], LAYOUT_FORMAT);
        assert_eq!(value["version"], LAYOUT_VERSION);
        assert_eq!(value["layout"]["name"], "Work");
        // What the zones took themselves is, once carried, a layout like any other.
        assert!(value["layout"].get("automatic").is_none());
        let read = read(&value).unwrap();
        assert_eq!(read, document);
        assert!(read.layout.same_layout(&layout()));
    }

    #[test]
    fn what_is_no_layout_or_cannot_be_right_is_refused_with_the_reason() {
        let good = LayoutDocument::of(layout(), 0).to_value();
        let with = |change: &dyn Fn(&mut Value)| {
            let mut value = good.clone();
            change(&mut value);
            read(&value).unwrap_err()
        };
        assert!(read(&serde_json::json!({"zones": []}))
            .unwrap_err()
            .contains("no layout"));
        assert!(with(&|value| value["format"] = "something-else".into()).contains("no layout"));
        assert!(with(&|value| value["version"] = 2.into()).contains("version 2"));
        assert!(with(&|value| value["version"] = 0.into()).contains("version 0"));
        assert!(with(&|value| value["version"] = "one".into()).contains("which version"));
        assert!(with(&|value| value["layout"] = 3.into()).contains("cannot be read"));
        assert!(with(&|value| value["layout"]["name"] = " ".into()).contains("no name"));
        let twice = with(&|value| {
            let zone = value["layout"]["zones"][0].clone();
            value["layout"]["zones"].as_array_mut().unwrap().push(zone);
        });
        assert!(twice.contains("zones[1].id"), "{twice}");
        let two_places = with(&|value| {
            value["layout"]["placements"]["zones"]["other"] =
                serde_json::json!({"items": ["/desk/a.pdf"]});
        });
        assert!(two_places.contains("/desk/a.pdf"), "{two_places}");
        // A key of a newer gevrek is carried along, not refused.
        let mut newer = good.clone();
        newer["remark"] = "from the future".into();
        assert_eq!(
            read(&newer).unwrap().to_value()["remark"],
            "from the future"
        );
    }
}
