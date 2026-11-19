//! `gevrek zones ...`: what the command line shows of the zones on the
//! desktop, made from what the window manager answers.
//!
//! The zones live in the shell; the window manager keeps what the shell
//! last said of them. So every answer here can also be "no shell runs the
//! zones" or "this is old", and each is said in words (and as fields in
//! the JSON) and never guessed around. The text is made by functions
//! that take the answer and give lines, so that it can be tested without
//! a window manager.

use gevrek_protocol::proto;
use gevrek_zones::ipc::{ZoneSnapshot, ZonesSnapshot, RESTORE_SYSTEM_ICONS};
use gevrek_zones::config::ZoneKind;

/// The command that puts the system's own desktop icons back.
pub fn restore_explorer_command() -> String {
    serde_json::json!({ "command": RESTORE_SYSTEM_ICONS }).to_string()
}

/// The snapshot of an answer, read. `Ok(None)`: there is none to read.
fn snapshot_of(state: &proto::ZonesStateResponse) -> Result<Option<ZonesSnapshot>, String> {
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
        width(|zone| zone.display.as_ref().map_or("-".to_string(), ToString::to_string), 7),
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
                zone.display.as_ref().map_or("-".to_string(), ToString::to_string),
                place(zone),
                zone.item_count,
                flags(zone)
            )
            .trim_end()
            .to_string(),
        );
    }
    lines.push(format!("Loose items: {}", snapshot.loose.len()));
    Ok(lines.join("\n"))
}

/// The zone of an answer by its id, or why it cannot be shown.
fn zone_of(state: &proto::ZonesStateResponse, id: &str) -> Result<(ZonesSnapshot, ZoneSnapshot), String> {
    let Some(snapshot) = snapshot_of(state)? else {
        return Err(state.message.clone());
    };
    match snapshot.zone(id).cloned() {
        Some(zone) => Ok((snapshot, zone)),
        None => {
            let known: Vec<String> = snapshot.zones.iter().map(|zone| zone.id.to_string()).collect();
            Err(if known.is_empty() {
                format!("There is no zone '{id}': there are no zones")
            } else {
                format!("There is no zone '{id}'. The zones are: {}", known.join(", "))
            })
        }
    }
}

/// What `gevrek zones show <id>` prints: the zone and its items.
pub fn show_text(state: &proto::ZonesStateResponse, id: &str) -> Result<String, String> {
    let (snapshot, zone) = zone_of(state, id)?;
    let mut lines = standing(state, &snapshot);
    let yes = |value: bool| if value { "yes" } else { "no" };
    lines.push(format!("{}  \"{}\"  {}", zone.id, zone.title, kind_word(&zone.kind)));
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
        zone.display.as_ref().map_or("-".to_string(), ToString::to_string)
    ));
    lines.push(format!("  rect       {}", place(&zone)));
    if let Some(geometry) = &zone.geometry {
        lines.push(format!(
            "  anchored   {} {:+},{:+} {}x{}",
            serde_json::to_value(geometry.anchor)
                .ok()
                .and_then(|anchor| anchor.as_str().map(str::to_string))
                .unwrap_or_default(),
            geometry.offset_x,
            geometry.offset_y,
            geometry.width,
            geometry.height
        ));
    }
    lines.push(format!("  visible    {}", yes(zone.visible)));
    lines.push(format!("  rolled up  {}", yes(zone.rolled_up)));
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
    lines.push(format!("  items ({}):", zone.item_count));
    let name = zone.items.iter().map(|item| item.name.chars().count()).max().unwrap_or(0);
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
        if let Some(sequence) = event.fields.get("sequence").and_then(|text| text.parse::<u64>().ok()) {
            line["sequence"] = sequence.into();
        }
        if let Some(session) = event.fields.get("session").and_then(|text| text.parse::<u64>().ok()) {
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
                    kind: ZoneKind::Portal { directory: "/home/ada/Downloads".to_string() },
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
            core.command(Command::RollUp { zone: ZoneId::new("work"), rolled_up: Some(true) }, NOW);
            core.command(
                Command::Lock { zone: ZoneId::new("work"), geometry: Some(true), members: None },
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
        assert_eq!(work[work.len() - 3..], ["1", "rolled-up", "locked(geometry)"]);
        assert!(lines[3].contains("portal"), "{text}");
        assert_eq!(*lines.last().unwrap(), "Loose items: 1");
    }

    #[test]
    fn what_is_old_switched_off_or_not_there_is_said() {
        // No shell, and never one: the one sentence, as the reason.
        assert!(list_text(&not_running()).unwrap_err().starts_with("Zones are not running"));
        assert!(show_text(&not_running(), "work").unwrap_err().starts_with("Zones are not running"));
        let json = state_json(&not_running());
        assert_eq!(json["running"], false);
        assert!(json["state"].is_null());

        // The shell is gone: what it last said, marked.
        let mut old = answer(&zones(true));
        old.running = false;
        old.stale = true;
        old.message = "Zones are not running. This is what the shell that ran them last reported".to_string();
        let text = list_text(&old).unwrap();
        assert!(text.starts_with("Stale: Zones are not running"), "{text}");
        assert!(text.contains("Zones: 2"));
        assert_eq!(state_json(&old)["stale"], true);
        assert_eq!(state_json(&old)["state"]["zones"][0]["id"], "work");

        let off = list_text(&answer(&zones(false))).unwrap();
        assert!(off.starts_with("Zones are switched off (desktop.zones.enabled)"), "{off}");

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
        assert!(text.contains("    file      report.pdf  /home/ada/Desktop/report.pdf"), "{text}");
        assert!(show_text(&state, "dl").unwrap().contains("  folder     /home/ada/Downloads"));

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
        assert_eq!(restore_explorer_command(), r#"{"command":"restore_system_icons"}"#);
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
            &[("json", r#"{"event":"zone_created","zone":"a"}"#), ("sequence", "3")],
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
}
