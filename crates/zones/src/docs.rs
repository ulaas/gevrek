//! The contract other parts are built on (docs/zones/core-milestones-3-5.md)
//! gives an example in JSON for every command, event and key. Each of
//! them is read here with the types it is an example of, and must come
//! back out as it went in: an example that names a field the code does
//! not have, or writes a value the code writes otherwise, fails.

use crate::backend::{BackendEvent, BackendRequest};
use crate::config::ZonesConfig;
use crate::core::ZonesEvent;
use crate::ipc::Request;
use serde_json::Value;

const CONTRACT: &str = include_str!("../../../docs/zones/core-milestones-3-5.md");

/// The blocks fenced as `json` (and not `jsonc`: those are parts of
/// larger things).
fn examples(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match &mut current {
            None if line.trim() == "```json" => current = Some(String::new()),
            None => {}
            Some(block) if line.trim() == "```" => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    blocks
}

/// What of `example` is not in `written` as it is there: the first place
/// they differ, if any. Keys that `written` has beyond the example are
/// defaults the example had no need to say.
fn missing(example: &Value, written: &Value, at: &str) -> Option<String> {
    match (example, written) {
        (Value::Object(example), Value::Object(written)) => example.iter().find_map(|(key, value)| {
            let place = format!("{at}.{key}");
            match written.get(key) {
                Some(other) => missing(value, other, &place),
                None => Some(format!("{place} is not a key that is written")),
            }
        }),
        (Value::Array(example), Value::Array(written)) if example.len() == written.len() => example
            .iter()
            .zip(written)
            .enumerate()
            .find_map(|(index, (value, other))| missing(value, other, &format!("{at}[{index}]"))),
        (example, written) if example == written => None,
        (example, written) => Some(format!("{at} is written {written}, and the example says {example}")),
    }
}

/// Read an example as what it is an example of, and write it again.
fn there_and_back(example: &Value) -> Result<Value, String> {
    let fail = |error: serde_json::Error| error.to_string();
    if example.get("command").is_some() {
        return Request::from_value(example.clone()).map(|request| request.to_value());
    }
    if example.get("ask").is_some() {
        let request: BackendRequest = serde_json::from_value(example.clone()).map_err(fail)?;
        return serde_json::to_value(request).map_err(fail);
    }
    if example.get("event").is_some() {
        if let Ok(event) = serde_json::from_value::<ZonesEvent>(example.clone()) {
            return serde_json::to_value(event).map_err(fail);
        }
        let event: BackendEvent = serde_json::from_value(example.clone()).map_err(fail)?;
        return serde_json::to_value(event).map_err(fail);
    }
    // Everything else is a part of `desktop.zones`.
    let config: ZonesConfig = serde_json::from_value(example.clone()).map_err(fail)?;
    if let Some((path, message)) = config.problems().into_iter().next() {
        return Err(format!("{path}: {message}"));
    }
    // (A key the configuration does not know is kept and written back,
    // for the sake of newer versions; in an example it is a mistake.)
    let unknown = [&config.extra, &config.look.extra, &config.snap.extra, &config.roll_up.extra]
        .into_iter()
        .chain([&config.privacy.extra, &config.timed_rules.extra])
        .chain([&config.gestures.extra, &config.auto.extra])
        .chain(config.auto.categories.iter().map(|category| &category.extra))
        .chain(config.zones.iter().flat_map(|zone| [&zone.extra, &zone.look.extra]))
        .chain(config.rules.iter().map(|rule| &rule.extra))
        .find_map(|extra| extra.0.keys().next());
    if let Some(key) = unknown {
        return Err(format!("{key} is no key of the configuration"));
    }
    serde_json::to_value(config).map_err(fail)
}

#[test]
fn every_example_in_the_contract_is_read() {
    let examples = examples(CONTRACT);
    assert!(examples.len() >= 50, "only {} examples were found", examples.len());
    let (mut commands, mut events, mut configurations) = (0, 0, 0);
    for text in examples {
        let example: Value = serde_json::from_str(&text).unwrap_or_else(|error| panic!("not JSON ({error}):\n{text}"));
        let written = there_and_back(&example).unwrap_or_else(|error| panic!("not read ({error}):\n{text}"));
        assert_eq!(missing(&example, &written, ""), None, "\n{text}");
        if example.get("command").is_some() {
            commands += 1;
        } else if example.get("event").is_some() {
            events += 1;
        } else if example.get("ask").is_none() {
            configurations += 1;
        }
    }
    assert!(commands >= 25 && events >= 12 && configurations >= 8, "{commands} {events} {configurations}");
}

#[test]
fn an_example_that_does_not_fit_is_noticed() {
    let read = |text: &str| {
        let example: Value = serde_json::from_str(text).unwrap();
        there_and_back(&example).map(|written| missing(&example, &written, ""))
    };
    assert_eq!(read(r#"{"command": "expand", "zone": "a"}"#), Ok(None));
    // A word the code does not know, a field it does not have, a value
    // it would write otherwise, a configuration that cannot be right.
    assert!(read(r#"{"command": "unroll", "zone": "a"}"#).is_err());
    assert!(read(r#"{"command": "expand", "zone": "a", "open": true}"#).unwrap().is_some());
    assert!(read(r#"{"roll_up": {"expand": "click"}}"#).is_err());
    assert!(read(r#"{"look": {"background": "blue"}}"#).is_err());
    assert!(read(r#"{"event": "nothing_at_all"}"#).is_err());
}
