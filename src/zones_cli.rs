//! `gevrek zones ...`: what the command line shows of the zones on the
//! desktop, made from what the window manager answers.
//!
//! The zones live in the shell; the window manager keeps what the shell
//! last said of them. So every answer here can also be "no shell runs the
//! zones" or "this is old", and each is said in words (and as fields in
//! the JSON) and never guessed around. The text is made by functions
//! that take the answer and give lines, so that it can be tested without
//! a window manager.

pub mod ask;
pub mod auto;

use gevrek_protocol::proto;
use gevrek_zones::config::ZoneKind;
use gevrek_zones::ipc::{ZoneSnapshot, ZonesSnapshot, RESTORE_SYSTEM_ICONS};
use serde_json::Value;

/// The command that puts the system's own desktop icons back.
pub fn restore_explorer_command() -> String {
    serde_json::json!({ "command": RESTORE_SYSTEM_ICONS }).to_string()
}

/// How `gevrek zones ...` ends, for a script that asks `$?`. 0 is "done".
/// (2 is the command line's own: words it cannot read.)
pub mod exit {
    /// Not done: the zones refused (with their reason), or what was
    /// named is not there, or two things have the name.
    pub const REFUSED: i32 = 1;
    /// No shell runs the zones, so nothing was asked of them.
    pub const NOT_RUNNING: i32 = 3;
    /// The shell was asked and did not answer in time, or went away
    /// before it did: what was asked may or may not have been done.
    pub const NO_ANSWER: i32 = 4;
}

/// Why `gevrek zones ...` did not do what it was asked, and the exit
/// code that says which kind of reason it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: i32,
    pub message: String,
}

impl Failure {
    pub fn refused(message: impl Into<String>) -> Self {
        Self {
            code: exit::REFUSED,
            message: message.into(),
        }
    }

    /// As a reply would say it, for `--json`: a script reads one shape
    /// whether the zones answered or were never asked.
    pub fn json(&self) -> Value {
        serde_json::json!({
            "success": false,
            "delivered": false,
            "error": self.message,
            "events": [],
        })
    }
}

/// Why a reply is no success, and which kind of reason; None for one
/// that is. The window manager tells the three kinds apart by
/// `delivered` and by its own two sentences for a shell that was asked
/// and did not answer.
pub fn failure_of(reply: &proto::ZonesCommandReply) -> Option<Failure> {
    if reply.success {
        return None;
    }
    let message = reply
        .error
        .clone()
        .unwrap_or_else(|| "The zones refused, and did not say why".to_string());
    let code = if !reply.delivered {
        exit::NOT_RUNNING
    } else if message.starts_with("The shell did not answer within")
        || message.starts_with("The shell that runs the zones went away")
    {
        exit::NO_ANSWER
    } else {
        exit::REFUSED
    };
    Some(Failure { code, message })
}

/// The zones of an answer, for a command that is about to be sent to
/// them: read, or why nothing can be sent (no shell runs them, or what
/// the shell told cannot be read by this program).
pub fn zones_for_asking(state: &proto::ZonesStateResponse) -> Result<ZonesSnapshot, Failure> {
    if !state.running {
        // (What a shell that is gone last told is of no use to a command.)
        let sentence = state.message.split(". This is").next().unwrap_or_default();
        return Err(Failure {
            code: exit::NOT_RUNNING,
            message: sentence.to_string(),
        });
    }
    match snapshot_of(state) {
        Ok(Some(snapshot)) => Ok(snapshot),
        Ok(None) => Err(Failure {
            code: exit::NO_ANSWER,
            message: state.message.clone(),
        }),
        Err(why) => Err(Failure::refused(why)),
    }
}

/// The snapshot of an answer, read. `Ok(None)`: there is none to read.
pub fn snapshot_of(state: &proto::ZonesStateResponse) -> Result<Option<ZonesSnapshot>, String> {
    if state.json.is_empty() {
        return Ok(None);
    }
    ZonesSnapshot::from_json(&state.json).map(Some)
}

/// The answer as one JSON object: whether the zones run, whether what is
/// shown is old, the snapshot's number, and the snapshot itself as
/// `state` (null where there is none).
pub fn state_json(state: &proto::ZonesStateResponse) -> serde_json::Value {
    serde_json::json!({
        "running": state.running,
        "stale": state.stale,
        "sequence": state.sequence,
        "version": state.version,
        "published_ms": state.published_ms,
        "message": state.message,
        "state": serde_json::from_str::<serde_json::Value>(&state.json)
            .unwrap_or(serde_json::Value::Null),
    })
}

fn kind_word(kind: &ZoneKind) -> &'static str {
    match kind {
        ZoneKind::Manual => "manual",
        ZoneKind::Portal { .. } => "portal",
        ZoneKind::Smart { .. } => "smart",
    }
}

fn place(zone: &ZoneSnapshot) -> String {
    match zone.rect {
        Some(rect) => format!("{},{} {}x{}", rect.x, rect.y, rect.width, rect.height),
        None => "-".to_string(),
    }
}

fn flags(zone: &ZoneSnapshot) -> String {
    let mut flags = Vec::new();
    if !zone.visible {
        flags.push("hidden");
    }
    if zone.rolled_up {
        flags.push("rolled-up");
    }
    match (zone.locked.geometry, zone.locked.members) {
        (true, true) => flags.push("locked"),
        (true, false) => flags.push("locked(geometry)"),
        (false, true) => flags.push("locked(members)"),
        (false, false) => {}
    }
    if zone.always_visible {
        flags.push("always-visible");
    }
    if zone.tab_group.is_some() && !zone.front {
        flags.push("tab-behind");
    }
    if zone.masked {
        flags.push("covered");
    } else if zone.private {
        flags.push("private");
    }
    if zone.reserves_space {
        flags.push("reserves-space");
    }
    if zone.focused {
        flags.push("focused");
    }
    flags.join(" ")
}

/// The line that says what is to be known before the zones themselves:
/// that they are switched off, hidden, or that the answer is old.
fn standing(state: &proto::ZonesStateResponse, snapshot: &ZonesSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if state.stale {
        lines.push(format!("Stale: {}", state.message));
    }
    if !snapshot.enabled {
        lines.push("Zones are switched off (desktop.zones.enabled)".to_string());
    }
    if snapshot.peeking {
        lines.push("Zones are shown above the windows (peek)".to_string());
    } else if snapshot.quick_hidden {
        lines.push("Zones are hidden (quick hide)".to_string());
    }
    if snapshot.masked {
        lines.push("What the zones hold is covered (privacy)".to_string());
    }
    if snapshot.screen_shared {
        lines.push("The screen is shared or recorded".to_string());
    }
    lines
}

/// What `gevrek zones list` prints, or why there is nothing to print.
pub fn list_text(state: &proto::ZonesStateResponse) -> Result<String, String> {
    let Some(snapshot) = snapshot_of(state)? else {
        return Err(state.message.clone());
    };
    let mut lines = standing(state, &snapshot);
    // Switched on and not shown: why (the system's own icons could not
    // be put away safely, say).
    if let Some(why) = &snapshot.not_shown {
        lines.push(format!("Not shown: {why}"));
    }
    let profile = match &snapshot.profile {
        Some(key) => {
            let displays: Vec<String> = key
                .displays
                .iter()
                .map(|display| {
                    format!(
                        "{} {}x{}@{:.2}",
                        display.id,
                        display.width,
                        display.height,
                        f64::from(display.scale) / 1000.0
                    )
                })
                .collect();
            match &key.workspace {
                Some(workspace) => format!("{} · workspace {workspace}", displays.join(" + ")),
                None => displays.join(" + "),
            }
        }
        None => "no displays known yet".to_string(),
    };
    lines.push(format!(
        "Zones: {} · profile: {} · snapshot {}",
        snapshot.zones.len(),
        profile,
        state.sequence
    ));
    let width = |text: fn(&ZoneSnapshot) -> String, least: usize| {
        snapshot
            .zones
            .iter()
            .map(|zone| text(zone).chars().count())
            .max()
            .unwrap_or(0)
            .max(least)
    };
    let (id, title, display, rect) = (
        width(|zone| zone.id.to_string(), 2),
        width(|zone| zone.title.clone(), 5),
        width(
            |zone| {
                zone.display
                    .as_ref()
                    .map_or("-".to_string(), ToString::to_string)
            },
            7,
        ),
        width(place, 4),
    );
    if !snapshot.zones.is_empty() {
        lines.push(format!(
            "  {:<id$}  {:<title$}  {:<6}  {:<display$}  {:<rect$}  {:>5}  FLAGS",
            "ID", "TITLE", "KIND", "DISPLAY", "RECT", "ITEMS"
        ));
    }
    for zone in &snapshot.zones {
        lines.push(
            format!(
                "  {:<id$}  {:<title$}  {:<6}  {:<display$}  {:<rect$}  {:>5}  {}",
                zone.id.to_string(),
                zone.title,
                kind_word(&zone.kind),
                zone.display
                    .as_ref()
                    .map_or("-".to_string(), ToString::to_string),
                place(zone),
                zone.item_count,
                flags(zone)
            )
            .trim_end()
            .to_string(),
        );
    }
    lines.push(format!("Loose items: {}", loose_count(state, &snapshot)));
    Ok(lines.join("\n"))
}

/// How many items are in no zone. An answer that was asked for without
/// the items says the number (`loose_count`) in place of the list.
fn loose_count(state: &proto::ZonesStateResponse, snapshot: &ZonesSnapshot) -> usize {
    serde_json::from_str::<Value>(&state.json)
        .ok()
        .and_then(|value| value.get("loose_count").and_then(Value::as_u64))
        .map_or(snapshot.loose.len(), |count| count as usize)
}

/// Where a zone is as it is remembered: from which corner or side of
/// its display, how far, and how large.
fn anchored(geometry: &gevrek_zones::geometry::ZoneGeometry) -> String {
    format!(
        "{} {:+},{:+} {}x{}",
        serde_json::to_value(geometry.anchor)
            .ok()
            .and_then(|anchor| anchor.as_str().map(str::to_string))
            .unwrap_or_default(),
        geometry.offset_x,
        geometry.offset_y,
        geometry.width,
        geometry.height
    )
}

/// The zone of an answer by its id, or why it cannot be shown.
fn zone_of(
    state: &proto::ZonesStateResponse,
    id: &str,
) -> Result<(ZonesSnapshot, ZoneSnapshot), String> {
    let Some(snapshot) = snapshot_of(state)? else {
        return Err(state.message.clone());
    };
    match snapshot.zone(id).cloned() {
        Some(zone) => Ok((snapshot, zone)),
        None => {
            let known: Vec<String> = snapshot
                .zones
                .iter()
                .map(|zone| zone.id.to_string())
                .collect();
            Err(if known.is_empty() {
                format!("There is no zone '{id}': there are no zones")
            } else {
                format!(
                    "There is no zone '{id}'. The zones are: {}",
                    known.join(", ")
                )
            })
        }
    }
}

/// What `gevrek zones show <id>` prints: the zone and its items.
pub fn show_text(state: &proto::ZonesStateResponse, id: &str) -> Result<String, String> {
    let (snapshot, zone) = zone_of(state, id)?;
    let mut lines = standing(state, &snapshot);
    let yes = |value: bool| if value { "yes" } else { "no" };
    lines.push(format!(
        "{}  \"{}\"  {}",
        zone.id,
        zone.title,
        kind_word(&zone.kind)
    ));
    match &zone.kind {
        ZoneKind::Portal { directory } => lines.push(format!("  folder     {directory}")),
        ZoneKind::Smart { query } => lines.push(format!(
            "  query      {}",
            serde_json::to_string(query).unwrap_or_default()
        )),
        ZoneKind::Manual => {}
    }
    lines.push(format!(
        "  display    {}",
        zone.display
            .as_ref()
            .map_or("-".to_string(), ToString::to_string)
    ));
    lines.push(format!("  rect       {}", place(&zone)));
    if let Some(geometry) = &zone.geometry {
        lines.push(format!("  anchored   {}", anchored(geometry)));
    }
    lines.push(format!("  visible    {}", yes(zone.visible)));
    lines.push(format!(
        "  rolled up  {}{}",
        yes(zone.rolled_up),
        if zone.rolled_up && zone.expanded {
            " (open for now)"
        } else {
            ""
        }
    ));
    lines.push(format!(
        "  view       {}, icons {}",
        serde_json::to_value(zone.view)
            .ok()
            .and_then(|view| view.as_str().map(str::to_string))
            .unwrap_or_default(),
        zone.icon_size
    ));
    if zone.always_visible {
        lines.push("  stays      when the zones are hidden".to_string());
    }
    if zone.private {
        lines.push(format!(
            "  private    yes ({})",
            if zone.masked {
                "covered"
            } else {
                "uncovered for now"
            }
        ));
    } else if zone.masked {
        lines.push("  covered    yes".to_string());
    }
    if zone.reserves_space {
        lines.push("  reserves   tiled windows keep off its place".to_string());
    }
    if !zone.workspaces.is_empty() {
        lines.push(format!("  workspaces {}", zone.workspaces.join(", ")));
    }
    if zone.focused {
        lines.push("  keyboard   yes".to_string());
    }
    if let Some(typed) = zone.filter.as_ref().filter(|typed| !typed.is_empty()) {
        lines.push(format!("  filter     {typed}"));
    }
    // Where a portal stands, and whether its folder (or a smart zone's
    // source) could be listed: as the zones wrote it.
    let told = serde_json::to_value(&zone).unwrap_or_default();
    if let Some(directory) = told["portal"]["directory"].as_str() {
        lines.push(format!("  stands in  {directory}"));
    }
    if let Some(source) = told["source"]["from"].as_str() {
        lines.push(format!(
            "  source     {}",
            told["source"]["directory"].as_str().unwrap_or(source)
        ));
    }
    match told["listing"]["state"].as_str() {
        Some("loading") => lines.push("  listing    still being read".to_string()),
        Some("unavailable") => lines.push(format!(
            "  listing    unavailable: {}",
            told["listing"]["reason"].as_str().unwrap_or_default()
        )),
        _ => {}
    }
    lines.push(format!(
        "  locked     geometry: {}, members: {}",
        yes(zone.locked.geometry),
        yes(zone.locked.members)
    ));
    if let Some(group) = &zone.tab_group {
        lines.push(format!(
            "  tabs       {group} ({})",
            if zone.front { "in front" } else { "behind" }
        ));
    }
    lines.push(if zone.items.len() == zone.item_count {
        format!("  items ({}):", zone.item_count)
    } else {
        format!("  items ({} of {}):", zone.items.len(), zone.item_count)
    });
    let name = zone
        .items
        .iter()
        .map(|item| item.name.chars().count())
        .max()
        .unwrap_or(0);
    for item in &zone.items {
        let kind = serde_json::to_value(item.kind)
            .ok()
            .and_then(|kind| kind.as_str().map(str::to_string))
            .unwrap_or_default();
        lines.push(
            format!(
                "    {:<8}  {:<name$}  {}",
                kind,
                item.name,
                item.path.as_deref().unwrap_or(item.id.as_str())
            )
            .trim_end()
            .to_string(),
        );
    }
    Ok(lines.join("\n"))
}

/// The same as JSON: the zone as the snapshot has it, with whether the
/// answer is old and its number.
pub fn show_json(state: &proto::ZonesStateResponse, id: &str) -> Result<serde_json::Value, String> {
    let (_, zone) = zone_of(state, id)?;
    Ok(serde_json::json!({
        "running": state.running,
        "stale": state.stale,
        "sequence": state.sequence,
        "zone": zone,
    }))
}

/// The events of a reply, read (one that is not JSON is shown as text).
fn reply_events(reply: &proto::ZonesCommandReply) -> Vec<serde_json::Value> {
    reply
        .events
        .iter()
        .map(|event| {
            serde_json::from_str(event).unwrap_or_else(|_| serde_json::Value::String(event.clone()))
        })
        .collect()
}

pub fn reply_json(reply: &proto::ZonesCommandReply) -> serde_json::Value {
    serde_json::json!({
        "success": reply.success,
        "delivered": reply.delivered,
        "error": reply.error,
        "events": reply_events(reply),
    })
}

/// What `gevrek zones send` prints for a command that was carried out
/// (`Done`, then what happened because of it, one event a line), or the
/// reason it was not.
pub fn reply_text(reply: &proto::ZonesCommandReply) -> Result<String, String> {
    if !reply.success {
        return Err(reply
            .error
            .clone()
            .unwrap_or_else(|| "The zones refused, and did not say why".to_string()));
    }
    let mut lines = vec!["Done".to_string()];
    lines.extend(reply_events(reply).iter().map(|event| event.to_string()));
    Ok(lines.join("\n"))
}

/// The first event of a reply that has this name (`zone_created`).
pub fn event_of(reply: &proto::ZonesCommandReply, name: &str) -> Option<Value> {
    reply_events(reply)
        .into_iter()
        .find(|event| event.get("event").and_then(Value::as_str) == Some(name))
}

/// Rows as a table: every column as wide as its widest cell, two spaces
/// between, the last one as long as it is.
fn table(rows: &[Vec<String>]) -> Vec<String> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(column, cell)| format!("{:<width$}", cell, width = widths[column]))
                .collect();
            format!("  {}", cells.join("  ")).trim_end().to_string()
        })
        .collect()
}

/// A zone as a table names it: its id, and its title where that says
/// more.
fn zone_words(id: &str, title: &str) -> String {
    if title.is_empty() || title == id {
        format!("zone {id}")
    } else {
        format!("zone {id} ({title})")
    }
}

/// What a rule does with what it matches, in words (as the
/// configuration writes an action).
fn action_words(snapshot: &ZonesSnapshot, action: &Value) -> String {
    match action.get("action").and_then(Value::as_str) {
        Some("assign_to_zone") => {
            let id = action["zone"].as_str().unwrap_or_default();
            zone_words(id, snapshot.zone(id).map_or("", |zone| zone.title.as_str()))
        }
        Some("move_to_folder") => format!(
            "folder {} (the file is moved)",
            action["folder"].as_str().unwrap_or_default()
        ),
        _ => action.to_string(),
    }
}

/// Where a line of a plan sends its item, in words.
fn destination_words(destination: &Value) -> String {
    match destination.get("to").and_then(Value::as_str) {
        Some("zone") => zone_words(
            destination["zone"].as_str().unwrap_or_default(),
            destination["title"].as_str().unwrap_or_default(),
        ),
        Some("folder") => format!(
            "folder {} (the file is moved)",
            destination["folder"].as_str().unwrap_or_default()
        ),
        _ => destination.to_string(),
    }
}

/// Which rule decided, in words: `rule 2 "Documents"` (counted from 1,
/// as `rules list` counts them), or the catch-all.
fn decided_words(line: &Value) -> String {
    match line["decided"].get("rule").and_then(Value::as_u64) {
        Some(index) => match line["rule"].as_str().unwrap_or_default() {
            "" => format!("rule {}", index + 1),
            name => format!("rule {} \"{name}\"", index + 1),
        },
        None => "the catch-all".to_string(),
    }
}

fn held_words(reason: &str) -> &str {
    match reason {
        "no_such_zone" => "the rule names a zone that is not there",
        "zone_fills_itself" => "the zone it names fills itself (a portal or a smart zone)",
        "locked" => "its zone, or the one it would go to, is locked",
        "not_a_file" => "it is no file that could be moved",
        other => other,
    }
}

/// The event of a reply that tells of a plan of the rules
/// (`rules_previewed` or `rules_applied`); None where the rules found
/// nothing to say (the timed ones, when nothing has come of age).
pub fn plan_of(reply: &proto::ZonesCommandReply) -> Option<Value> {
    event_of(reply, "rules_previewed").or_else(|| event_of(reply, "rules_applied"))
}

/// What `gevrek zones rules dry-run` (and `apply`, `check`) prints from
/// the zones' own lines: every item the rules would move (or moved),
/// from where, where to, and by which rule; then the items a rule
/// matched and nothing is done with, and why. `asked_of` says which
/// items were looked at.
pub fn plan_text(event: Option<&Value>, applied: bool, asked_of: &str) -> String {
    let empty = Vec::new();
    let lines = event
        .and_then(|event| event["lines"].as_array())
        .unwrap_or(&empty);
    let (held, steps): (Vec<&Value>, Vec<&Value>) = lines
        .iter()
        .partition(|line| line.get("held").is_some_and(|held| !held.is_null()));
    let count = |count: usize| {
        if count == 1 {
            "1 item".to_string()
        } else {
            format!("{count} items")
        }
    };
    let from = |line: &Value| match line["from"].as_str() {
        Some(zone) => format!("zone {zone}"),
        None => "loose".to_string(),
    };
    let name = |line: &Value| line["name"].as_str().unwrap_or_default().to_string();
    let mut text = Vec::new();
    if steps.is_empty() {
        text.push(format!(
            "The rules {} nothing (asked of {asked_of})",
            if applied { "changed" } else { "would change" }
        ));
    } else {
        text.push(format!(
            "The rules {} {} (asked of {asked_of}):",
            if applied { "sorted" } else { "would sort" },
            count(steps.len())
        ));
        let mut rows = vec![vec!["ITEM", "FROM", "GOES TO", "BY"]
            .into_iter()
            .map(str::to_string)
            .collect()];
        rows.extend(steps.iter().map(|line| {
            vec![
                name(line),
                from(line),
                destination_words(&line["destination"]),
                decided_words(line),
            ]
        }));
        text.extend(table(&rows));
    }
    if !held.is_empty() {
        text.push(format!(
            "Left where {} ({}):",
            if held.len() == 1 { "it is" } else { "they are" },
            held.len()
        ));
        let mut rows = vec![vec!["ITEM", "FROM", "BECAUSE", "BY"]
            .into_iter()
            .map(str::to_string)
            .collect()];
        rows.extend(held.iter().map(|line| {
            vec![
                name(line),
                from(line),
                held_words(line["held"].as_str().unwrap_or_default()).to_string(),
                decided_words(line),
            ]
        }));
        text.extend(table(&rows));
    }
    if !applied && !steps.is_empty() {
        text.push("Nothing was changed. `gevrek zones rules apply` carries it out.".to_string());
    }
    if applied && !steps.is_empty() {
        text.push("`gevrek zones rules undo` takes it back.".to_string());
    }
    text.join("\n")
}

/// One thing a rule asks of an item, in words.
fn matcher_words(matcher: &Value) -> String {
    let list = |key: &str| -> String {
        matcher[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let text = |key: &str| matcher[key].as_str().unwrap_or_default();
    match matcher.get("match").and_then(Value::as_str) {
        Some("extension") => format!("extension {}", list("any_of")),
        Some("mime") => format!("type {}", text("pattern")),
        Some("name") => format!("name {}", text("pattern")),
        Some("kind") => format!("kind {}", list("any_of")),
        Some("shortcut_target") => format!("leads to {}", text("pattern")),
        Some("untouched") => format!("untouched for {} days", matcher["days"]),
        Some("size") => {
            let mut bounds = Vec::new();
            if let Some(least) = matcher["at_least"].as_u64() {
                bounds.push(format!("at least {least} bytes"));
            }
            if let Some(most) = matcher["at_most"].as_u64() {
                bounds.push(format!("at most {most} bytes"));
            }
            format!("size {}", bounds.join(", "))
        }
        Some("date") => {
            let mut bounds = Vec::new();
            for (key, words) in [
                ("older_than_days", "more than {} days ago"),
                ("newer_than_days", "in the last {} days"),
                ("after", "after {} (seconds since 1970)"),
                ("before", "before {} (seconds since 1970)"),
            ] {
                if let Some(number) = matcher[key].as_u64() {
                    bounds.push(words.replace("{}", &number.to_string()));
                }
            }
            format!(
                "{} {}",
                matcher["field"].as_str().unwrap_or("modified"),
                bounds.join(", ")
            )
        }
        _ => matcher.to_string(),
    }
}

/// What `gevrek zones rules list` prints, from the section
/// `desktop.zones` of the configuration: the rules in the order they are
/// tried, what each asks and does, and what happens to what none
/// matches.
pub fn rules_text(zones: &Value, snapshot: Option<&ZonesSnapshot>) -> String {
    let none = ZonesSnapshot::default();
    let snapshot = snapshot.unwrap_or(&none);
    let empty = Vec::new();
    let rules = zones["rules"].as_array().unwrap_or(&empty);
    let yes = |value: &Value, otherwise: bool| {
        if value.as_bool().unwrap_or(otherwise) {
            "yes"
        } else {
            "no"
        }
    };
    let mut head = format!(
        "Rules: {} · on arrival: {}",
        rules.len(),
        yes(&zones["rules_on_arrival"], true)
    );
    if let Some(timed) = zones.get("timed_rules") {
        match timed["every_minutes"].as_u64().unwrap_or(60) {
            0 => head.push_str(" · by age: never by themselves"),
            minutes => head.push_str(&format!(
                " · by age: every {minutes} min{}",
                if timed["move_files"].as_bool().unwrap_or(false) {
                    ", files moved too"
                } else {
                    ""
                }
            )),
        }
    }
    let mut lines = vec![head];
    if !rules.is_empty() {
        let mut rows = vec![vec!["#", "NAME", "ON", "WHEN", "THEN"]
            .into_iter()
            .map(str::to_string)
            .collect()];
        rows.extend(rules.iter().enumerate().map(|(index, rule)| {
            let asks: Vec<String> = rule["matchers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(matcher_words)
                .collect();
            vec![
                (index + 1).to_string(),
                rule["name"].as_str().unwrap_or_default().to_string(),
                yes(&rule["enabled"], true).to_string(),
                if asks.is_empty() {
                    "(asks nothing: matches nothing)".to_string()
                } else {
                    asks.join(" and ")
                },
                action_words(snapshot, &rule["action"]),
            ]
        }));
        lines.extend(table(&rows));
    }
    lines.push(
        match zones.get("catch_all").filter(|action| !action.is_null()) {
            Some(action) => format!("What no rule matches: {}", action_words(snapshot, action)),
            None => "What no rule matches stays where it is".to_string(),
        },
    );
    lines.join("\n")
}

/// What `gevrek zones search` prints from the `found` event: the items
/// that fit, the best first, and where each is.
pub fn search_text(found: &Value, snapshot: &ZonesSnapshot) -> String {
    let typed = found["typed"].as_str().unwrap_or_default();
    let empty = Vec::new();
    let hits = found["hits"].as_array().unwrap_or(&empty);
    if hits.is_empty() {
        return format!("Nothing fits '{typed}'");
    }
    let zone_title = |id: &str| snapshot.zone(id).map_or("", |zone| zone.title.as_str());
    let mut lines = vec![format!(
        "{} fit{} '{typed}':",
        if hits.len() == 1 {
            "1 item".to_string()
        } else {
            format!("{} items", hits.len())
        },
        if hits.len() == 1 { "s" } else { "" }
    )];
    let mut rows = vec![vec![
        "NAME".to_string(),
        "WHERE".to_string(),
        "ID".to_string(),
    ]];
    rows.extend(hits.iter().map(|hit| {
        vec![
            hit["name"].as_str().unwrap_or_default().to_string(),
            match hit["zone"].as_str() {
                Some(zone) => zone_words(zone, zone_title(zone)),
                None => "loose".to_string(),
            },
            hit["item"].as_str().unwrap_or_default().to_string(),
        ]
    }));
    lines.extend(table(&rows));
    lines.join("\n")
}

/// What an answer's snapshot is as plain JSON; null where there is none.
fn state_value(state: &proto::ZonesStateResponse) -> Value {
    serde_json::from_str(&state.json).unwrap_or(Value::Null)
}

/// A time as the zones keep it (seconds since 1970), written as a day
/// and an hour of the world's clock. (No calendar is at hand here; the
/// days are counted by the rule that every reckoning of dates uses.)
fn time_words(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // From the days since 1970 to a date: the civil calendar's cycle of
    // 400 years, begun in March so that the leap day is the last.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// The saved copies of the layout an answer tells of, as JSON:
/// `{running, stale, sequence, snapshots}`.
pub fn snapshots_json(state: &proto::ZonesStateResponse) -> Value {
    serde_json::json!({
        "running": state.running,
        "stale": state.stale,
        "sequence": state.sequence,
        "snapshots": state_value(state).get("snapshots").cloned().unwrap_or_else(|| serde_json::json!([])),
    })
}

/// What `gevrek zones snapshot list` prints.
pub fn snapshots_text(state: &proto::ZonesStateResponse) -> Result<String, String> {
    let Some(snapshot) = snapshot_of(state)? else {
        return Err(state.message.clone());
    };
    let mut lines = standing(state, &snapshot);
    let value = state_value(state);
    let empty = Vec::new();
    let copies = value["snapshots"].as_array().unwrap_or(&empty);
    if copies.is_empty() {
        lines.push(
            "No copies of the layout are saved (`gevrek zones snapshot save <name>`)".to_string(),
        );
        return Ok(lines.join("\n"));
    }
    lines.push(format!("Saved copies of the layout: {}", copies.len()));
    let mut rows = vec![vec![
        "NAME".to_string(),
        "TAKEN".to_string(),
        "ZONES".to_string(),
        String::new(),
    ]];
    rows.extend(copies.iter().map(|copy| {
        vec![
            copy["name"].as_str().unwrap_or_default().to_string(),
            copy["taken_at"]
                .as_u64()
                .map_or("-".to_string(), time_words),
            copy["zones"]
                .as_u64()
                .map_or("-".to_string(), |zones| zones.to_string()),
            // Taken by the zones themselves before a layout was replaced.
            if copy["automatic"].as_bool().unwrap_or(false) {
                "(automatic)".to_string()
            } else {
                String::new()
            },
        ]
    }));
    lines.extend(table(&rows));
    Ok(lines.join("\n"))
}

/// A set of displays as a line names it: `DEL-A1B2-77 1920x1080@1.00 +
/// ...`, and the workspace where the arrangement is one workspace's.
fn profile_words(key: &Value) -> String {
    let displays: Vec<String> = key["displays"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|display| {
            format!(
                "{} {}x{}@{:.2}",
                display["id"].as_str().unwrap_or_default(),
                display["width"],
                display["height"],
                display["scale"].as_f64().unwrap_or(1000.0) / 1000.0
            )
        })
        .collect();
    let displays = if displays.is_empty() {
        "no displays".to_string()
    } else {
        displays.join(" + ")
    };
    match key["workspace"].as_str() {
        Some(workspace) => format!("{displays} · workspace {workspace}"),
        None => displays,
    }
}

/// The arrangements an answer tells of, as JSON: `{running, stale,
/// sequence, current, profiles}` (`all`), or `{..., current, zones}` with
/// where each zone is in the one in use.
pub fn profiles_json(state: &proto::ZonesStateResponse, all: bool) -> Value {
    let value = state_value(state);
    let mut answer = serde_json::json!({
        "running": state.running,
        "stale": state.stale,
        "sequence": state.sequence,
        "current": value.get("profile").cloned().unwrap_or(Value::Null),
    });
    if all {
        answer["profiles"] = value
            .get("profiles")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
    } else {
        let zones: Vec<Value> = value["zones"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|zone| {
                serde_json::json!({
                    "id": zone["id"], "display": zone["display"], "rect": zone["rect"],
                    "geometry": zone["geometry"], "visible": zone["visible"],
                })
            })
            .collect();
        answer["zones"] = zones.into();
    }
    answer
}

/// What `gevrek zones profile list` (`all`) and `profile show` print.
pub fn profiles_text(state: &proto::ZonesStateResponse, all: bool) -> Result<String, String> {
    let Some(snapshot) = snapshot_of(state)? else {
        return Err(state.message.clone());
    };
    let value = state_value(state);
    let mut lines = standing(state, &snapshot);
    let current = value.get("profile").filter(|key| !key.is_null());
    if all {
        let empty = Vec::new();
        let profiles = value["profiles"].as_array().unwrap_or(&empty);
        lines.push(format!(
            "Arrangements remembered: {}",
            profiles.len().max(usize::from(current.is_some()))
        ));
        if profiles.is_empty() {
            // (A shell that tells only of the one in use.)
            if let Some(key) = current {
                lines.push(format!("  * {}", profile_words(key)));
            }
        }
        for (index, profile) in profiles.iter().enumerate() {
            let in_use = profile["current"].as_bool().unwrap_or(false);
            let mut line = format!(
                "  {} {}  {}",
                if in_use { "*" } else { " " },
                index + 1,
                profile_words(&profile["key"])
            );
            if let Some(zones) = profile["zones"].as_u64() {
                line.push_str(&format!(
                    " · {zones} zone{} placed",
                    if zones == 1 { "" } else { "s" }
                ));
            }
            lines.push(line);
        }
        lines.push(
            "(* is the one in use; `gevrek zones profile copy <number>` takes another over)"
                .to_string(),
        );
        return Ok(lines.join("\n"));
    }
    match current {
        Some(key) => lines.push(format!("In use: {}", profile_words(key))),
        None => lines.push("No arrangement is in use: no displays are known yet".to_string()),
    }
    let mut rows = vec![vec![
        "ZONE".to_string(),
        "DISPLAY".to_string(),
        "RECT".to_string(),
        "ANCHORED".to_string(),
    ]];
    rows.extend(snapshot.zones.iter().map(|zone| {
        vec![
            zone.id.to_string(),
            zone.display
                .as_ref()
                .map_or("-".to_string(), ToString::to_string),
            place(zone) + if zone.visible { "" } else { " (hidden)" },
            zone.geometry.as_ref().map_or("-".to_string(), anchored),
        ]
    }));
    if rows.len() > 1 {
        lines.extend(table(&rows));
    }
    Ok(lines.join("\n"))
}

/// The copies and moves that are under way, as an answer's snapshot
/// tells of them (`operations`): each with the number it is called off
/// by. None where the shell tells of none.
pub fn operations_of(state: &proto::ZonesStateResponse) -> Vec<Value> {
    state_value(state)
        .get("operations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What `gevrek zones cancel` prints where there is not exactly one
/// operation to call off: that there is none, or which there are.
pub fn operations_text(operations: &[Value]) -> String {
    if operations.is_empty() {
        return "Nothing is being copied or moved (a number is told with the event \
                `zones.operation_begun`, and called off with `gevrek zones cancel <number>`)"
            .to_string();
    }
    let mut lines = vec![format!(
        "Under way: {}. Say which: `gevrek zones cancel <number>`",
        operations.len()
    )];
    let mut rows = vec![vec!["NUMBER", "WHAT", "OF", "DONE"]
        .into_iter()
        .map(str::to_string)
        .collect()];
    rows.extend(operations.iter().map(|operation| {
        let what = &operation["operation"];
        // Whatever the operation names of files and folders, in the
        // order it names them.
        let of: Vec<String> = what
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| key.as_str() != "operation")
            .filter_map(|(_, value)| value.as_str().map(str::to_string))
            .collect();
        let (done, total) = (
            operation["done"].as_u64().unwrap_or(0),
            operation["total"].as_u64().unwrap_or(0),
        );
        vec![
            operation["request"].to_string(),
            what["operation"].as_str().unwrap_or_default().to_string(),
            of.join(" -> "),
            // (A total of 0 is "not known".)
            done.saturating_mul(100)
                .checked_div(total)
                .map_or("-".to_string(), |part| format!("{part}%")),
        ]
    }));
    lines.extend(table(&rows));
    lines.join("\n")
}

/// The layout a reply hands over (the event `layout_exported`), as it
/// is written to a file.
pub fn exported(reply: &proto::ZonesCommandReply) -> Option<Value> {
    event_of(reply, "layout_exported").and_then(|event| event.get("document").cloned())
}

/// The command that takes a layout in, from the text of a file `snapshot
/// export` wrote. Only that it is JSON is looked at here: the zones look
/// the document over themselves before anything is touched, and say what
/// is wrong with it.
pub fn import_command(text: &str, apply: bool) -> Result<Value, String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("it is not JSON: {error}"))?;
    Ok(serde_json::json!({ "command": "import_layout", "apply": apply, "document": document }))
}

/// The line `gevrek zones events` prints for something the window
/// manager tells; None for what is not about the zones.
pub fn event_line(event: &proto::RuntimeEvent, json: bool) -> Option<String> {
    let name = event.kind.strip_prefix("zones.")?;
    let data = event
        .fields
        .get("json")
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok());
    if json {
        let mut line = serde_json::json!({
            "event": event.kind,
            "timestamp_ms": event.timestamp_ms,
        });
        if let Some(sequence) = event
            .fields
            .get("sequence")
            .and_then(|text| text.parse::<u64>().ok())
        {
            line["sequence"] = sequence.into();
        }
        if let Some(session) = event
            .fields
            .get("session")
            .and_then(|text| text.parse::<u64>().ok())
        {
            line["session"] = session.into();
        }
        if let Some(data) = data {
            line["data"] = data;
        }
        return Some(line.to_string());
    }
    Some(match (name, data) {
        ("attached", _) => "[zones] a shell runs the zones".to_string(),
        ("detached", _) => "[zones] the shell that ran the zones is gone".to_string(),
        (_, Some(data)) => format!("[zones] {data}"),
        (name, None) => format!("[zones] {name}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::backend::{BackendEvent, Location};
    use gevrek_zones::config::{ZoneDefinition, ZonesConfig};
    use gevrek_zones::geometry::Display;
    use gevrek_zones::{Command, ItemId, Rect, ZoneId, ZonesCore, ZonesState};

    const NOW: u64 = 1_800_000_000;

    /// Two zones on one display, one with a file in it and rolled up.
    fn zones(enabled: bool) -> ZonesSnapshot {
        let config = ZonesConfig {
            enabled,
            zones: vec![
                ZoneDefinition::manual("work", "Work"),
                ZoneDefinition {
                    kind: ZoneKind::Portal {
                        directory: "/home/ada/Downloads".to_string(),
                    },
                    ..ZoneDefinition::manual("dl", "Downloads")
                },
            ],
            ..ZonesConfig::default()
        };
        let mut core = ZonesCore::new(config, ZonesState::default());
        core.event(
            BackendEvent::Displays {
                displays: vec![Display {
                    primary: true,
                    ..Display::new("DEL-A1B2-77", Rect::new(0, 0, 1920, 1080))
                }],
                reserved: Vec::new(),
            },
            NOW,
        );
        core.event(
            BackendEvent::Listed {
                location: Location::Desktop,
                items: vec![
                    gevrek_zones::Item::file("/home/ada/Desktop/report.pdf"),
                    gevrek_zones::Item::file("/home/ada/Desktop/notes.txt"),
                ],
            },
            NOW,
        );
        if enabled {
            core.command(
                Command::AddItem {
                    zone: ZoneId::new("work"),
                    item: ItemId::new("/home/ada/Desktop/report.pdf"),
                    index: None,
                    cell: None,
                },
                NOW,
            );
            core.command(
                Command::RollUp {
                    zone: ZoneId::new("work"),
                    rolled_up: Some(true),
                },
                NOW,
            );
            core.command(
                Command::Lock {
                    zone: ZoneId::new("work"),
                    geometry: Some(true),
                    members: None,
                },
                NOW,
            );
        }
        ZonesSnapshot::of(&core, NOW)
    }

    fn answer(snapshot: &ZonesSnapshot) -> proto::ZonesStateResponse {
        proto::ZonesStateResponse {
            running: true,
            stale: false,
            version: snapshot.version,
            sequence: 14,
            json: serde_json::to_string(snapshot).unwrap(),
            published_ms: 1,
            message: String::new(),
        }
    }

    fn not_running() -> proto::ZonesStateResponse {
        proto::ZonesStateResponse {
            message: "Zones are not running: no gevrek-shell that runs them is attached to this window manager"
                .to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_list_names_every_zone_with_where_it_is_and_how() {
        let text = list_text(&answer(&zones(true))).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "Zones: 2 · profile: DEL-A1B2-77 1920x1080@1.00 · snapshot 14"
        );
        assert!(lines[1].trim_start().starts_with("ID"), "{text}");
        let work: Vec<&str> = lines[2].split_whitespace().collect();
        assert_eq!(&work[..4], ["work", "Work", "manual", "DEL-A1B2-77"]);
        assert_eq!(
            work[work.len() - 3..],
            ["1", "rolled-up", "locked(geometry)"]
        );
        assert!(lines[3].contains("portal"), "{text}");
        assert_eq!(*lines.last().unwrap(), "Loose items: 1");
    }

    #[test]
    fn what_is_old_switched_off_or_not_there_is_said() {
        // No shell, and never one: the one sentence, as the reason.
        assert!(list_text(&not_running())
            .unwrap_err()
            .starts_with("Zones are not running"));
        assert!(show_text(&not_running(), "work")
            .unwrap_err()
            .starts_with("Zones are not running"));
        let json = state_json(&not_running());
        assert_eq!(json["running"], false);
        assert!(json["state"].is_null());

        // The shell is gone: what it last said, marked.
        let mut old = answer(&zones(true));
        old.running = false;
        old.stale = true;
        old.message =
            "Zones are not running. This is what the shell that ran them last reported".to_string();
        let text = list_text(&old).unwrap();
        assert!(text.starts_with("Stale: Zones are not running"), "{text}");
        assert!(text.contains("Zones: 2"));
        assert_eq!(state_json(&old)["stale"], true);
        assert_eq!(state_json(&old)["state"]["zones"][0]["id"], "work");

        let off = list_text(&answer(&zones(false))).unwrap();
        assert!(
            off.starts_with("Zones are switched off (desktop.zones.enabled)"),
            "{off}"
        );

        // A snapshot of a newer shape is not shown as if it were read.
        let mut newer = answer(&zones(true));
        newer.json = newer.json.replacen("\"version\":1", "\"version\":7", 1);
        assert!(list_text(&newer).unwrap_err().contains("version 7"));
    }

    #[test]
    fn a_zone_is_shown_with_its_items() {
        let state = answer(&zones(true));
        let text = show_text(&state, "work").unwrap();
        assert!(text.starts_with("work  \"Work\"  manual"), "{text}");
        assert!(text.contains("  rolled up  yes"));
        assert!(text.contains("  locked     geometry: yes, members: no"));
        assert!(text.contains("  anchored   top_left"), "{text}");
        assert!(text.contains("  items (1):"));
        assert!(
            text.contains("    file      report.pdf  /home/ada/Desktop/report.pdf"),
            "{text}"
        );
        assert!(show_text(&state, "dl")
            .unwrap()
            .contains("  folder     /home/ada/Downloads"));

        let json = show_json(&state, "work").unwrap();
        assert_eq!(json["zone"]["items"][0]["name"], "report.pdf");
        assert_eq!(json["sequence"], 14);

        let missing = show_text(&state, "play").unwrap_err();
        assert_eq!(missing, "There is no zone 'play'. The zones are: work, dl");
    }

    #[test]
    fn a_reply_is_done_with_what_happened_or_the_reason_it_was_not() {
        let done = proto::ZonesCommandReply {
            success: true,
            error: None,
            events: vec![r#"{"event":"zone_created","zone":"zone-3"}"#.to_string()],
            delivered: true,
        };
        assert_eq!(
            reply_text(&done).unwrap(),
            "Done\n{\"event\":\"zone_created\",\"zone\":\"zone-3\"}"
        );
        assert_eq!(reply_json(&done)["events"][0]["zone"], "zone-3");

        let refused = proto::ZonesCommandReply {
            success: false,
            error: Some("zones are switched off (desktop.zones.enabled)".to_string()),
            events: Vec::new(),
            delivered: true,
        };
        assert_eq!(
            reply_text(&refused).unwrap_err(),
            "zones are switched off (desktop.zones.enabled)"
        );
        assert_eq!(reply_json(&refused)["success"], false);
        assert_eq!(
            restore_explorer_command(),
            r#"{"command":"restore_system_icons"}"#
        );
    }

    #[test]
    fn only_what_is_about_the_zones_is_a_line_of_the_events() {
        let event = |kind: &str, fields: &[(&str, &str)]| proto::RuntimeEvent {
            kind: kind.to_string(),
            timestamp_ms: 5,
            window_id: None,
            fields: fields
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        };
        assert_eq!(event_line(&event("window.created", &[]), false), None);
        let created = event(
            "zones.zone_created",
            &[
                ("json", r#"{"event":"zone_created","zone":"a"}"#),
                ("sequence", "3"),
            ],
        );
        assert_eq!(
            event_line(&created, false).unwrap(),
            r#"[zones] {"event":"zone_created","zone":"a"}"#
        );
        let line: serde_json::Value =
            serde_json::from_str(&event_line(&created, true).unwrap()).unwrap();
        assert_eq!(line["event"], "zones.zone_created");
        assert_eq!(line["sequence"], 3);
        assert_eq!(line["data"]["zone"], "a");
        assert_eq!(
            event_line(&event("zones.detached", &[("session", "2")]), false).unwrap(),
            "[zones] the shell that ran the zones is gone"
        );
    }

    fn reply(
        success: bool,
        delivered: bool,
        error: Option<&str>,
        events: &[&str],
    ) -> proto::ZonesCommandReply {
        proto::ZonesCommandReply {
            success,
            delivered,
            error: error.map(str::to_string),
            events: events.iter().map(|event| event.to_string()).collect(),
        }
    }

    #[test]
    fn how_it_ended_is_told_by_a_code_a_script_can_ask() {
        assert_eq!(failure_of(&reply(true, true, None, &[])), None);
        for (reply, code) in [
            (reply(false, true, Some("no zone is called nowhere"), &[]), exit::REFUSED),
            (reply(false, true, None, &[]), exit::REFUSED),
            (
                reply(false, false, Some("Zones are not running: no gevrek-shell that runs them is attached to this window manager"), &[]),
                exit::NOT_RUNNING,
            ),
            (
                reply(false, true, Some("The shell did not answer within 5000 ms; what was asked may still be carried out"), &[]),
                exit::NO_ANSWER,
            ),
            (
                reply(false, true, Some("The shell that runs the zones went away before it answered"), &[]),
                exit::NO_ANSWER,
            ),
        ] {
            assert_eq!(failure_of(&reply).unwrap().code, code, "{:?}", reply.error);
        }
        assert_eq!(
            (exit::REFUSED, exit::NOT_RUNNING, exit::NO_ANSWER),
            (1, 3, 4)
        );

        // Before a command: no shell is "not running", also where an
        // old shell's zones are still told.
        let none = zones_for_asking(&not_running()).unwrap_err();
        assert_eq!(none.code, exit::NOT_RUNNING);
        let mut old = answer(&zones(true));
        old.running = false;
        old.stale = true;
        old.message = format!(
            "{}. This is what the shell that ran them last reported",
            not_running().message
        );
        let gone = zones_for_asking(&old).unwrap_err();
        assert_eq!(
            (gone.code, gone.message),
            (exit::NOT_RUNNING, not_running().message)
        );
        assert_eq!(
            zones_for_asking(&answer(&zones(true))).unwrap().zones.len(),
            2
        );
        // Said to a script as an answer is.
        assert_eq!(
            Failure::refused("There is no zone 'x'").json().to_string(),
            r#"{"success":false,"delivered":false,"error":"There is no zone 'x'","events":[]}"#
        );
    }

    #[test]
    fn a_list_without_the_items_still_counts_them() {
        let snapshot = zones(true);
        let mut state = answer(&snapshot);
        // As the window manager trims it: no items, their numbers.
        let mut value: Value = serde_json::from_str(&state.json).unwrap();
        value.as_object_mut().unwrap().remove("loose");
        value["loose_count"] = 4000.into();
        for zone in value["zones"].as_array_mut().unwrap() {
            zone.as_object_mut().unwrap().remove("items");
        }
        state.json = value.to_string();
        let text = list_text(&state).unwrap();
        assert!(text.ends_with("Loose items: 4000"), "{text}");
        let work: Vec<&str> = text.lines().nth(2).unwrap().split_whitespace().collect();
        assert_eq!(
            work[work.len() - 3..],
            ["1", "rolled-up", "locked(geometry)"]
        );
    }

    #[test]
    fn what_is_covered_focused_or_private_is_said() {
        let mut snapshot = zones(true);
        snapshot.masked = true;
        snapshot.screen_shared = true;
        snapshot.zones[0].private = true;
        snapshot.zones[0].masked = true;
        snapshot.zones[0].focused = true;
        snapshot.zones[1].private = true;
        snapshot.zones[1].reserves_space = true;
        snapshot.zones[1].workspaces = vec!["2".to_string(), "dev".to_string()];
        let state = answer(&snapshot);
        let text = list_text(&state).unwrap();
        assert!(
            text.starts_with(
                "What the zones hold is covered (privacy)\nThe screen is shared or recorded\n"
            ),
            "{text}"
        );
        assert!(
            text.lines()
                .nth(4)
                .unwrap()
                .ends_with("rolled-up locked(geometry) covered focused"),
            "{text}"
        );
        assert!(
            text.lines()
                .nth(5)
                .unwrap()
                .ends_with("private reserves-space"),
            "{text}"
        );
        let shown = show_text(&state, "dl").unwrap();
        assert!(
            shown.contains("  private    yes (uncovered for now)"),
            "{shown}"
        );
        assert!(
            shown.contains("  reserves   tiled windows keep off its place"),
            "{shown}"
        );
        assert!(shown.contains("  workspaces 2, dev"), "{shown}");
        assert!(shown.contains("  view       grid, icons 48"), "{shown}");
        let shown = show_text(&state, "work").unwrap();
        assert!(shown.contains("  private    yes (covered)"), "{shown}");
        assert!(shown.contains("  keyboard   yes"), "{shown}");
    }

    #[test]
    fn a_dry_run_is_a_table_of_what_goes_where_and_by_which_rule() {
        let event: Value = serde_json::from_str(
            r#"{"event": "rules_previewed",
                "plan": {"steps": [], "held": []},
                "lines": [
                  {"item": "/desk/cat.png", "name": "cat.png", "from": "inbox",
                   "destination": {"to": "zone", "zone": "pictures", "title": "Pictures"},
                   "decided": {"rule": 0}, "rule": "Pictures"},
                  {"item": "/desk/old.zip", "name": "old.zip",
                   "destination": {"to": "folder", "folder": "/home/ada/Archive"},
                   "decided": {"rule": 2}, "rule": ""},
                  {"item": "/desk/thing.bin", "name": "thing.bin",
                   "destination": {"to": "zone", "zone": "misc", "title": "misc"},
                   "decided": "catch_all", "rule": ""},
                  {"item": "/desk/setup.exe", "name": "setup.exe", "from": "tools",
                   "destination": {"to": "zone", "zone": "programs", "title": "Programs"},
                   "decided": {"rule": 1}, "rule": "Programs", "held": "locked"}]}"#,
        )
        .unwrap();
        let text = plan_text(Some(&event), false, "every item of the desktop");
        println!("{text}");
        assert_eq!(
            text,
            "The rules would sort 3 items (asked of every item of the desktop):\n\
             \x20 ITEM       FROM        GOES TO                                       BY\n\
             \x20 cat.png    zone inbox  zone pictures (Pictures)                      rule 1 \"Pictures\"\n\
             \x20 old.zip    loose       folder /home/ada/Archive (the file is moved)  rule 3\n\
             \x20 thing.bin  loose       zone misc                                     the catch-all\n\
             Left where it is (1):\n\
             \x20 ITEM       FROM        BECAUSE                                         BY\n\
             \x20 setup.exe  zone tools  its zone, or the one it would go to, is locked  rule 2 \"Programs\"\n\
             Nothing was changed. `gevrek zones rules apply` carries it out."
        );
        let applied = plan_text(Some(&event), true, "the items that are in no zone");
        assert!(applied
            .starts_with("The rules sorted 3 items (asked of the items that are in no zone):"));
        assert!(applied.ends_with("`gevrek zones rules undo` takes it back."));
        // Nothing to do, and nothing said at all (the timed rules).
        let nothing: Value =
            serde_json::from_str(r#"{"event":"rules_previewed","plan":{},"lines":[]}"#).unwrap();
        assert_eq!(
            plan_text(Some(&nothing), false, "every item of the desktop"),
            "The rules would change nothing (asked of every item of the desktop)"
        );
        assert_eq!(
            plan_text(None, true, "every item, by the rules that go by age"),
            "The rules changed nothing (asked of every item, by the rules that go by age)"
        );
        // The event is found in a reply under either of its names.
        let told = reply(
            true,
            true,
            None,
            &[r#"{"event":"layout_changed"}"#, &nothing.to_string()],
        );
        assert_eq!(plan_of(&told).unwrap()["event"], "rules_previewed");
        assert_eq!(plan_of(&reply(true, true, None, &[])), None);
    }

    #[test]
    fn the_rules_are_listed_in_the_order_they_are_tried() {
        let section: Value = serde_json::from_str(
            r#"{"rules_on_arrival": true,
                "timed_rules": {"every_minutes": 60, "move_files": false},
                "rules": [
                  {"name": "Documents", "enabled": true,
                   "matchers": [{"match": "extension", "any_of": ["pdf", "docx"]}],
                   "action": {"action": "assign_to_zone", "zone": "work"}},
                  {"name": "Stale", "enabled": false,
                   "matchers": [{"match": "untouched", "days": 30}, {"match": "size", "at_least": 1000000}],
                   "action": {"action": "move_to_folder", "folder": "/home/ada/Archive"}},
                  {"name": "Recent pictures",
                   "matchers": [{"match": "mime", "pattern": "image/*"},
                                {"match": "date", "field": "modified", "newer_than_days": 7}],
                   "action": {"action": "assign_to_zone", "zone": "gone"}}],
                "catch_all": {"action": "assign_to_zone", "zone": "dl"}}"#,
        )
        .unwrap();
        let text = rules_text(&section, Some(&zones(true)));
        println!("{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "Rules: 3 · on arrival: yes · by age: every 60 min"
        );
        assert_eq!(
            lines[1].split_whitespace().collect::<Vec<_>>(),
            ["#", "NAME", "ON", "WHEN", "THEN"]
        );
        assert!(
            lines[2].contains("1  Documents        yes  extension pdf, docx"),
            "{text}"
        );
        assert!(lines[2].ends_with("zone work (Work)"), "{text}");
        assert!(
            lines[3].contains("no   untouched for 30 days and size at least 1000000 bytes"),
            "{text}"
        );
        assert!(
            lines[3].ends_with("folder /home/ada/Archive (the file is moved)"),
            "{text}"
        );
        assert!(
            lines[4].contains("type image/* and modified in the last 7 days"),
            "{text}"
        );
        assert!(lines[4].ends_with("zone gone"), "{text}");
        assert_eq!(lines[5], "What no rule matches: zone dl (Downloads)");
        // Without rules, and without a shell to ask for the titles.
        let none: Value = serde_json::from_str(r#"{"timed_rules": {"every_minutes": 0}}"#).unwrap();
        assert_eq!(
            rules_text(&none, None),
            "Rules: 0 · on arrival: yes · by age: never by themselves\nWhat no rule matches stays where it is"
        );
    }

    #[test]
    fn what_was_found_is_listed_with_where_it_is() {
        let found: Value = serde_json::from_str(
            r#"{"event": "found", "typed": "rep", "hits": [
                 {"item": "/home/ada/Desktop/report.pdf", "name": "report.pdf", "zone": "work", "score": 890},
                 {"item": "/home/ada/Desktop/Old report.txt", "name": "Old report.txt", "score": 700}]}"#,
        )
        .unwrap();
        let text = search_text(&found, &zones(true));
        println!("{text}");
        assert_eq!(
            text,
            "2 items fit 'rep':\n\
             \x20 NAME            WHERE             ID\n\
             \x20 report.pdf      zone work (Work)  /home/ada/Desktop/report.pdf\n\
             \x20 Old report.txt  loose             /home/ada/Desktop/Old report.txt"
        );
        let nothing: Value = serde_json::from_str(r#"{"typed": "xyz", "hits": []}"#).unwrap();
        assert_eq!(search_text(&nothing, &zones(true)), "Nothing fits 'xyz'");
    }

    #[test]
    fn saved_copies_and_arrangements_are_listed() {
        let mut snapshot = zones(true);
        snapshot.snapshots = serde_json::from_str(
            r#"[{"name": "Work", "taken_at": 1799990000, "zones": 4},
                {"name": "Before restoring", "taken_at": 1800000000, "zones": 2, "automatic": true}]"#,
        )
        .unwrap();
        snapshot.profiles = serde_json::from_str(
            r#"[{"key": {"displays": [{"id": "DEL-A1B2-77", "width": 1920, "height": 1080, "scale": 1000}]},
                 "zones": 2, "current": true},
                {"key": {"displays": [{"id": "DEL-A1B2-77", "width": 1920, "height": 1080, "scale": 1000},
                                      {"id": "LG-0042", "width": 2560, "height": 1440, "scale": 1250}],
                         "workspace": "dev"},
                 "zones": 1, "current": false}]"#,
        )
        .unwrap();
        let state = answer(&snapshot);
        let text = snapshots_text(&state).unwrap();
        println!("{text}");
        assert_eq!(
            text,
            "Saved copies of the layout: 2\n\
             \x20 NAME              TAKEN                 ZONES\n\
             \x20 Work              2027-01-15 05:13 UTC  4\n\
             \x20 Before restoring  2027-01-15 08:00 UTC  2      (automatic)"
        );
        assert_eq!(snapshots_json(&state)["snapshots"][1]["automatic"], true);
        assert!(snapshots_text(&answer(&zones(true)))
            .unwrap()
            .starts_with("No copies of the layout are saved"));
        assert!(snapshots_text(&not_running())
            .unwrap_err()
            .starts_with("Zones are not running"));

        let text = profiles_text(&state, true).unwrap();
        println!("{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Arrangements remembered: 2");
        assert_eq!(
            lines[1],
            "  * 1  DEL-A1B2-77 1920x1080@1.00 · 2 zones placed"
        );
        assert_eq!(
            lines[2],
            "    2  DEL-A1B2-77 1920x1080@1.00 + LG-0042 2560x1440@1.25 · workspace dev · 1 zone placed"
        );
        assert_eq!(
            profiles_json(&state, true)["profiles"][1]["key"]["workspace"],
            "dev"
        );

        let text = profiles_text(&state, false).unwrap();
        println!("{text}");
        assert!(
            text.starts_with("In use: DEL-A1B2-77 1920x1080@1.00\n"),
            "{text}"
        );
        assert!(text.contains("  work  DEL-A1B2-77  "), "{text}");
        assert!(text.contains("top_left"), "{text}");
        assert_eq!(profiles_json(&state, false)["zones"][0]["id"], "work");
    }

    #[test]
    fn what_is_under_way_is_listed_with_the_number_that_calls_it_off() {
        let mut state = answer(&zones(true));
        assert!(operations_of(&state).is_empty());
        assert!(operations_text(&[]).starts_with("Nothing is being copied or moved"));
        let mut value: Value = serde_json::from_str(&state.json).unwrap();
        value["operations"] = serde_json::json!([
            {"request": 7, "done": 314572800u64, "total": 1073741824u64,
             "operation": {"operation": "copy", "from": "/media/usb/film.mkv", "to_folder": "/home/ada/Desktop"}},
            {"request": 9, "done": 0, "total": 0,
             "operation": {"operation": "move", "from": "/home/ada/a.iso", "to_folder": "/mnt/nas"}}]);
        state.json = value.to_string();
        let operations = operations_of(&state);
        assert_eq!(operations.len(), 2);
        let text = operations_text(&operations);
        println!("{text}");
        assert_eq!(
            text,
            "Under way: 2. Say which: `gevrek zones cancel <number>`\n\
             \x20 NUMBER  WHAT  OF                                        DONE\n\
             \x20 7       copy  /media/usb/film.mkv -> /home/ada/Desktop  29%\n\
             \x20 9       move  /home/ada/a.iso -> /mnt/nas               -"
        );
    }

    #[test]
    fn a_time_is_written_as_a_day_and_an_hour() {
        for (seconds, written) in [
            (0, "1970-01-01 00:00 UTC"),
            (951_782_400, "2000-02-29 00:00 UTC"),
            (1_800_000_000, "2027-01-15 08:00 UTC"),
            (4_102_444_799, "2099-12-31 23:59 UTC"),
        ] {
            assert_eq!(time_words(seconds), written);
        }
    }

    #[test]
    fn a_layout_goes_to_a_file_and_comes_from_one() {
        let document =
            r#"{"format":"gevrek-zones-layout","version":1,"layout":{"name":"Work","zones":[]}}"#;
        let told = reply(
            true,
            true,
            None,
            &[&format!(
                r#"{{"event":"layout_exported","document":{document}}}"#
            )],
        );
        assert_eq!(exported(&told).unwrap().to_string(), document);
        assert_eq!(exported(&reply(true, true, None, &[])), None);
        let command = import_command(document, false).unwrap();
        assert_eq!(
            command.to_string(),
            format!(r#"{{"command":"import_layout","apply":false,"document":{document}}}"#)
        );
        // The zones read it as theirs.
        gevrek_zones::ipc::Request::from_value(import_command(document, true).unwrap()).unwrap();
        assert!(import_command("not a layout", false)
            .unwrap_err()
            .starts_with("it is not JSON"));
    }
}
