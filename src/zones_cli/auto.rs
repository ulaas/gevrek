//! What `gevrek zones auto-create` and `auto-undo` print: the plan of the
//! zones that would be made for each type of item as a table, and what
//! was made or taken back as a sentence. Made from the zones' own events
//! (`auto_planned`, `auto_created`, `auto_undone`), so that it can be
//! tested without a window manager.

use super::table;
use gevrek_zones::ipc::ZonesSnapshot;
use serde_json::Value;

fn counted(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// What an item is called: as the zones tell it, else the end of its id.
fn item_name(snapshot: &ZonesSnapshot, id: &str) -> String {
    let told = snapshot
        .loose
        .iter()
        .chain(snapshot.zones.iter().flat_map(|zone| &zone.items))
        .find(|item| item.id.as_str() == id)
        .map(|item| item.name.clone());
    told.unwrap_or_else(|| {
        let id = id.trim_end_matches(['/', '\\']);
        id.rsplit(['/', '\\']).next().unwrap_or(id).to_string()
    })
}

/// What `gevrek zones auto-create --dry-run` prints from the zones' plan
/// (the event `auto_planned`): each zone with its title, whether it is
/// new or one that is there, how many items would go into it, where it
/// is or would be, and whether a rule would be added; then the items
/// that are left where they are. `asked_of` says which items were looked
/// at.
pub fn plan_text(event: Option<&Value>, snapshot: &ZonesSnapshot, asked_of: &str) -> String {
    let Some(plan) = event.map(|event| &event["plan"]) else {
        return "The zones answered, but did not say what they would make (this shell may be older)".to_string();
    };
    let empty = Vec::new();
    let zones = plan["zones"].as_array().unwrap_or(&empty);
    let left = plan["left"].as_array().unwrap_or(&empty);
    let mut text = Vec::new();
    if zones.is_empty() {
        text.push(format!("There are no zones to make by type (asked of {asked_of})"));
    } else {
        let new = zones.iter().filter(|zone| zone["zone"].is_null()).count() as u64;
        let items: u64 = zones.iter().map(|zone| zone["items"].as_array().map_or(0, Vec::len) as u64).sum();
        let on = match plan["display"].as_str() {
            Some(display) if new > 0 => format!(" on display {display}"),
            _ => String::new(),
        };
        text.push(format!(
            "{} would be made{on}, and {} sorted (asked of {asked_of}):",
            counted(new, "zone", "zones"),
            counted(items, "item", "items"),
        ));
        let mut rows = vec![vec!["ZONE", "IS", "ITEMS", "AT", "SIZE", "RULE"].into_iter().map(str::to_string).collect()];
        rows.extend(zones.iter().map(|zone| {
            let rect = &zone["rect"];
            let number = |key: &str| rect[key].as_i64().unwrap_or(0);
            vec![
                zone["title"].as_str().unwrap_or_default().to_string(),
                match zone["zone"].as_str() {
                    Some(id) => format!("zone {id}"),
                    None => "new".to_string(),
                },
                zone["items"].as_array().map_or(0, Vec::len).to_string(),
                format!("{},{}", number("x"), number("y")),
                format!("{}x{}", number("width"), number("height")),
                if zone["rule"].as_bool().unwrap_or(false) { "a rule for what comes later" } else { "" }.to_string(),
            ]
        }));
        text.extend(table(&rows));
    }
    if !left.is_empty() {
        const NAMED: usize = 8;
        let mut names: Vec<String> =
            left.iter().take(NAMED).map(|item| item_name(snapshot, item.as_str().unwrap_or_default())).collect();
        if left.len() > NAMED {
            names.push(format!("and {} more", left.len() - NAMED));
        }
        text.push(format!(
            "Left where {} ({}; too few of a type for a zone of its own, or its zone is locked): {}",
            if left.len() == 1 { "it is" } else { "they are" },
            left.len(),
            names.join(", ")
        ));
    }
    if !zones.is_empty() {
        text.push("Nothing was changed. `gevrek zones auto-create` carries it out.".to_string());
    }
    text.join("\n")
}

/// What `gevrek zones auto-create` prints once it is done, from the
/// event `auto_created`.
pub fn created_text(event: Option<&Value>) -> String {
    let Some(event) = event else {
        return "Done".to_string();
    };
    let zones: Vec<&str> = event["zones"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let items = event["items"].as_u64().unwrap_or(0);
    let rules = event["rules"].as_u64().unwrap_or(0);
    if zones.is_empty() && items == 0 {
        return "There was nothing to do: no type has enough loose items for a zone, and the zones made by type have theirs"
            .to_string();
    }
    let mut said = match zones.len() {
        0 => format!("{} sorted into the zones made by type before", counted(items, "item was", "items were")),
        count => format!(
            "{} made ({}), and {} sorted by type",
            counted(count as u64, "zone was", "zones were"),
            zones.join(", "),
            counted(items, "item", "items"),
        ),
    };
    if rules > 0 {
        said.push_str(&format!("; {} added for what comes later", counted(rules, "rule was", "rules were")));
    }
    said.push_str(". `gevrek zones auto-undo` takes it back.");
    said
}

/// What `gevrek zones auto-undo` prints, from the event `auto_undone`.
pub fn undone_text(event: Option<&Value>) -> String {
    let Some(event) = event else {
        return "The zones made by type were taken back".to_string();
    };
    let count = |key: &str| event[key].as_u64().unwrap_or(0);
    format!(
        "Taken back: {} taken away, {} put back where {}, {} taken out",
        counted(count("zones"), "zone", "zones"),
        counted(count("items"), "item", "items"),
        if count("items") == 1 { "it was" } else { "they were" },
        counted(count("rules"), "rule", "rules"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::ipc::ItemSnapshot;
    use gevrek_zones::ItemId;

    fn desk() -> ZonesSnapshot {
        ZonesSnapshot {
            enabled: true,
            loose: vec![ItemSnapshot {
                id: ItemId::new("shell:42"),
                name: "Holiday.lnk".to_string(),
                ..ItemSnapshot::default()
            }],
            ..ZonesSnapshot::default()
        }
    }

    #[test]
    fn a_plan_of_zones_by_type_is_a_table_of_what_would_be_made_and_where() {
        let event: Value = serde_json::from_str(
            r#"{"event": "auto_planned", "plan": {
                "display": "DEL-A1B2-77",
                "zones": [
                  {"category": "folders", "title": "Folders", "rect": {"x": 8, "y": 48, "width": 172, "height": 126},
                   "items": ["/desk/Archive", "/desk/Projects"], "rule": true},
                  {"category": "pictures", "title": "Photos", "zone": "zone-4",
                   "rect": {"x": 900, "y": 600, "width": 412, "height": 384},
                   "items": ["/desk/one.gif"], "rule": false},
                  {"category": "other", "title": "Other", "rect": {"x": 188, "y": 48, "width": 252, "height": 212},
                   "items": ["/desk/a", "/desk/b", "/desk/c", "/desk/d", "/desk/e"], "rule": false}],
                "left": ["/desk/song.mp3", "shell:42"]}}"#,
        )
        .unwrap();
        let text = plan_text(Some(&event), &desk(), "the items that are in no zone");
        println!("{text}");
        assert_eq!(
            text,
            "2 zones would be made on display DEL-A1B2-77, and 8 items sorted (asked of the items that are in no zone):\n\
             \x20 ZONE     IS           ITEMS  AT       SIZE     RULE\n\
             \x20 Folders  new          2      8,48     172x126  a rule for what comes later\n\
             \x20 Photos   zone zone-4  1      900,600  412x384\n\
             \x20 Other    new          5      188,48   252x212\n\
             Left where they are (2; too few of a type for a zone of its own, or its zone is locked): song.mp3, Holiday.lnk\n\
             Nothing was changed. `gevrek zones auto-create` carries it out."
        );
        // Nothing to make is said so, with what is left and why.
        let nothing: Value =
            serde_json::from_str(r#"{"event": "auto_planned", "plan": {"zones": [], "left": ["/desk/song.mp3"]}}"#)
                .unwrap();
        assert_eq!(
            plan_text(Some(&nothing), &desk(), "every item of the desktop"),
            "There are no zones to make by type (asked of every item of the desktop)\n\
             Left where it is (1; too few of a type for a zone of its own, or its zone is locked): song.mp3"
        );
        let bare: Value = serde_json::from_str(r#"{"event": "auto_planned", "plan": {"zones": []}}"#).unwrap();
        assert_eq!(
            plan_text(Some(&bare), &desk(), "the items that are in no zone"),
            "There are no zones to make by type (asked of the items that are in no zone)"
        );
        // Many that are left are counted, not all named.
        let many: Vec<String> = (0..12).map(|number| format!("/desk/f{number}.bin")).collect();
        let event = serde_json::json!({"event": "auto_planned", "plan": {"zones": [], "left": many}});
        let text = plan_text(Some(&event), &desk(), "x");
        assert!(text.ends_with("f6.bin, f7.bin, and 4 more"), "{text}");
        // A shell that does not say (an older one) is not guessed around.
        assert!(plan_text(None, &desk(), "x").contains("did not say"));
    }

    #[test]
    fn what_was_made_by_type_and_what_was_taken_back_is_said_in_a_sentence() {
        let event = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        assert_eq!(
            created_text(Some(&event(r#"{"event":"auto_created","zones":["zone-4","zone-5"],"items":7,"rules":2}"#))),
            "2 zones were made (zone-4, zone-5), and 7 items sorted by type; \
             2 rules were added for what comes later. `gevrek zones auto-undo` takes it back."
        );
        assert_eq!(
            created_text(Some(&event(r#"{"event":"auto_created","zones":["zone-4"],"items":2,"rules":0}"#))),
            "1 zone was made (zone-4), and 2 items sorted by type. `gevrek zones auto-undo` takes it back."
        );
        assert_eq!(
            created_text(Some(&event(r#"{"event":"auto_created","zones":[],"items":1,"rules":0}"#))),
            "1 item was sorted into the zones made by type before. `gevrek zones auto-undo` takes it back."
        );
        assert!(created_text(Some(&event(r#"{"event":"auto_created","zones":[],"items":0,"rules":0}"#)))
            .starts_with("There was nothing to do"));
        assert_eq!(created_text(None), "Done");
        assert_eq!(
            undone_text(Some(&event(r#"{"event":"auto_undone","zones":3,"items":1,"rules":2}"#))),
            "Taken back: 3 zones taken away, 1 item put back where it was, 2 rules taken out"
        );
        assert_eq!(undone_text(None), "The zones made by type were taken back");
    }
}
