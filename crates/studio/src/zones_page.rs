//! The page of the zones on the desktop: the containers that hold its
//! icons (`desktop.zones`). Not the places windows snap to, which have a
//! page of their own (`snap_zones_edit`).
//!
//! What the page holds is what the configuration holds: whether zones are
//! shown, the zones there are (what each is called, where its items come
//! from, how it shows them), the rules that sort items into them, and how
//! zones look and behave. Where a zone *is* on the desktop is not here:
//! that is remembered for each arrangement of displays by the zones
//! themselves, and a zone is placed by dragging it on the desktop.
//!
//! The lists are drawn by hand (a line an entry, the one that is open
//! with its settings below it); what stands in an entry is asked of the
//! schema, so a field the model gains has its control here without a line
//! written for it. Only what reads better than a form is special: a list
//! of extensions is one text, a colour is a colour, a folder is chosen.
//!
//! The rules can be tried: the zones that run are asked, through the
//! window manager, what their rules would do, and the answer is shown as
//! a table before anything is moved. The window manager is also asked,
//! now and then while the page is shown, whether zones run at all: where
//! none do, that is said instead of offering what cannot be done.
//!
//! Zones can be made by type the same way: the zones that run are asked
//! what they would make for the desktop as it is (a zone for the folders,
//! one for the pictures...), the answer is shown, and only then are they
//! made; and what was made so can be taken back.

use crate::document::Document;
use crate::form::Form;
use crate::pages::{Card, Page, Show};
use egui::{RichText, Ui};
use gevrek_model::settings::{ConfigIssue, ConfigIssueSeverity};
use gevrek_model::zones::ZoneLook;
use gevrek_schema::{title, Field, Kind, Node, Variant, VariantShape};
use gevrek_ui::theme::{self, icons, Palette};
use gevrek_ui::widgets;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const ZONES: &str = "/desktop/zones";
const LIST: &str = "/desktop/zones/zones";
const RULES: &str = "/desktop/zones/rules";
const CATCH_ALL: &str = "/desktop/zones/catch_all";
/// Whether Explorer's own icons are put away while zones are shown. It is
/// a setting of Windows and stands with those; this page names it.
pub const HIDE_ICONS: &str = "/desktop/platform/windows/hide_desktop_icons";

/// Below this width an entry's line has no room for its buttons beside
/// its fields: they stand on a line of their own.
const NARROW: f32 = 640.0;

/// How long the zones may take to answer (the window manager's own wait,
/// and the line's, are shorter: their answer is the one shown).
const ANSWER_ARRIVES: Duration = Duration::from_secs(20);
/// How often the window manager is asked how the zones are while the
/// page is shown.
const STATE_AGAIN: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------
// The lists: entries taken out and put elsewhere.

/// What is done to a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Remove(usize),
    /// The entry at `from` is put down before the one at `to` (the list's
    /// length: at its end).
    Move {
        from: usize,
        to: usize,
    },
}

impl Change {
    /// One place towards the beginning.
    pub fn up(index: usize) -> Option<Change> {
        (index > 0).then(|| Change::Move {
            from: index,
            to: index - 1,
        })
    }

    /// One place towards the end.
    pub fn down(index: usize, count: usize) -> Option<Change> {
        (index + 1 < count).then_some(Change::Move {
            from: index,
            to: index + 2,
        })
    }
}

/// Carry a change out. Says whether the list is another now.
pub fn changed<T>(list: &mut Vec<T>, change: Change) -> bool {
    match change {
        Change::Remove(index) if index < list.len() => {
            list.remove(index);
            true
        }
        Change::Remove(_) => false,
        Change::Move { from, to } => crate::reorder::moved(list, from, to),
    }
}

/// Where the entry that was at `open` is after a change to a list of
/// `count` entries: the settings that are open stay with their entry when
/// it is moved, and close when it is taken out.
pub fn follows(open: Option<usize>, count: usize, change: Change) -> Option<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    changed(&mut order, change);
    open.and_then(|open| order.iter().position(|was| *was == open))
}

// ---------------------------------------------------------------------
// The zones.

/// A name no zone of these has: `zone-1`, `zone-2`, as the zones
/// themselves name one that is made on the desktop.
pub fn free_id(zones: &[Value]) -> String {
    (1u32..)
        .map(|number| format!("zone-{number}"))
        .find(|id| !zones.iter().any(|zone| zone["id"].as_str() == Some(id)))
        .unwrap_or_default()
}

/// A zone to add to these: filled by hand, with a name of its own.
pub fn another_zone(zones: &[Value]) -> Value {
    let id = free_id(zones);
    let number = id.trim_start_matches("zone-").to_string();
    let zone = gevrek_model::zones::ZoneDefinition::manual(&id, &format!("Zone {number}"));
    serde_json::to_value(zone).unwrap_or(Value::Null)
}

/// Give the zone at `index` of the section (`desktop.zones`) another id,
/// and have every rule that names it name it still. The rules follow only
/// where the old id was this zone's alone: of two zones with one id
/// nobody can say which a rule meant.
pub fn rename_zone(section: &mut Value, index: usize, new: &str) {
    let old = section["zones"][index]["id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if section["zones"].get(index).is_none() || old == new {
        return;
    }
    let shared = section["zones"].as_array().map_or(0, |zones| {
        zones
            .iter()
            .filter(|zone| zone["id"] == old.as_str())
            .count()
    }) > 1;
    section["zones"][index]["id"] = Value::from(new);
    if old.trim().is_empty() || shared {
        return;
    }
    let follow = |action: &mut Value| {
        if action["action"] == "assign_to_zone" && action["zone"] == old.as_str() {
            action["zone"] = Value::from(new);
        }
    };
    if let Some(rules) = section["rules"].as_array_mut() {
        for rule in rules {
            follow(&mut rule["action"]);
        }
    }
    if section["catch_all"].is_object() {
        follow(&mut section["catch_all"]);
    }
}

/// What a person calls the zone with this id: its title, its id where it
/// has none, and the id alone where no zone has it.
pub fn zone_called(zones: &[Value], id: &str) -> String {
    zones
        .iter()
        .find(|zone| zone["id"].as_str() == Some(id))
        .and_then(|zone| zone["title"].as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(id)
        .to_string()
}

/// What a zone's line says of it beside its title: only what is other
/// than usual.
pub fn zone_said(zone: &Value) -> String {
    let mut said = Vec::new();
    match zone["kind"]["type"].as_str() {
        Some("portal") => {
            let folder = zone["kind"]["directory"].as_str().unwrap_or("").trim();
            said.push(if folder.is_empty() {
                "no folder yet".to_string()
            } else {
                folder.to_string()
            });
        }
        Some("smart") => {
            let count = zone["kind"]["query"].as_array().map_or(0, Vec::len);
            said.push(counted(count, "condition"));
        }
        _ => {}
    }
    if zone["view"] == "list" {
        said.push("as a list".to_string());
    }
    if zone["rolled_up"] == true {
        said.push("rolled up".to_string());
    }
    if zone["always_visible"] == true {
        said.push("always visible".to_string());
    }
    let locked = |what: &str| zone["lock"][what] == true;
    match (locked("geometry"), locked("members")) {
        (true, true) => said.push("locked".to_string()),
        (true, false) => said.push("place locked".to_string()),
        (false, true) => said.push("items locked".to_string()),
        (false, false) => {}
    }
    if let Some(group) = zone["tab_group"]
        .as_str()
        .filter(|group| !group.trim().is_empty())
    {
        said.push(format!("a tab of \u{201c}{group}\u{201d}"));
    }
    said.join("  ·  ")
}

fn counted(count: usize, word: &str) -> String {
    match count {
        1 => format!("1 {word}"),
        count => format!("{count} {word}s"),
    }
}

// ---------------------------------------------------------------------
// The rules, in words.

/// What is typed as a list of extensions (or of anything short): parted
/// at commas, semicolons and blanks, without the dot or the `*.` a person
/// may write before an extension, each once.
pub fn listed(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split([',', ';', ' ', '\t']) {
        let word = word.trim().trim_start_matches("*.").trim_start_matches('.');
        if !word.is_empty() && !found.iter().any(|known| known.eq_ignore_ascii_case(word)) {
            found.push(word.to_string());
        }
    }
    found
}

fn words_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|word| word.as_str().map(str::to_string))
        .collect()
}

fn days(count: u64) -> String {
    match count {
        1 => "1 day".to_string(),
        count => format!("{count} days"),
    }
}

/// One condition of a rule, as a person says it.
pub fn matcher_said(matcher: &Value) -> String {
    let text = |key: &str| matcher[key].as_str().unwrap_or("").trim().to_string();
    let number = |key: &str| matcher[key].as_u64();
    let or_none = |text: String| {
        if text.is_empty() {
            "(nothing yet)".to_string()
        } else {
            text
        }
    };
    match matcher["match"].as_str().unwrap_or("") {
        "extension" => format!(
            "extension is {}",
            or_none(words_of(&matcher["any_of"]).join(", "))
        ),
        "mime" => format!("type fits {}", or_none(text("pattern"))),
        "name" => format!("name fits {}", or_none(text("pattern"))),
        "shortcut_target" => format!("leads to {}", or_none(text("pattern"))),
        "kind" => {
            let kinds: Vec<String> = words_of(&matcher["any_of"])
                .iter()
                .map(|kind| title(kind).to_lowercase())
                .collect();
            format!("is a {}", or_none(kinds.join(" or a ")))
        }
        "untouched" => format!("untouched for {}", days(number("days").unwrap_or(0))),
        "size" => {
            let said = |bytes: u64| gevrek_ui::browser::size_said(bytes);
            match (number("at_least"), number("at_most")) {
                (Some(least), Some(most)) => format!("between {} and {}", said(least), said(most)),
                (Some(least), None) => format!("at least {}", said(least)),
                (None, Some(most)) => format!("at most {}", said(most)),
                (None, None) => "any size".to_string(),
            }
        }
        "date" => {
            let field = matcher["field"].as_str().unwrap_or("modified");
            let mut bounds = Vec::new();
            if let Some(newer) = number("newer_than_days") {
                bounds.push(format!("in the last {}", days(newer)));
            }
            if let Some(older) = number("older_than_days") {
                bounds.push(format!("more than {} ago", days(older)));
            }
            if number("after").is_some() {
                bounds.push("after a day".to_string());
            }
            if number("before").is_some() {
                bounds.push("before a day".to_string());
            }
            if bounds.is_empty() {
                format!("{field} at any time")
            } else {
                format!("{field} {}", bounds.join(" and "))
            }
        }
        // A condition this page does not know by name: its own name.
        other => title(other).to_lowercase(),
    }
}

/// What a rule does with an item, as a person says it.
pub fn action_said(action: &Value, zones: &[Value]) -> String {
    match action["action"].as_str().unwrap_or("") {
        "assign_to_zone" => {
            let id = action["zone"].as_str().unwrap_or("").trim();
            if id.is_empty() {
                "into a zone (none chosen)".to_string()
            } else {
                format!("into \u{201c}{}\u{201d}", zone_called(zones, id))
            }
        }
        "move_to_folder" => {
            let folder = action["folder"].as_str().unwrap_or("").trim();
            if folder.is_empty() {
                "the file is moved (no folder yet)".to_string()
            } else {
                format!("the file is moved to {folder}")
            }
        }
        other => title(other).to_lowercase(),
    }
}

/// The sign between what a rule asks and what it does: an arrow the
/// program's own letters have.
const THEN: &str = icons::ARROW_RIGHT;

/// A rule on one line: what must hold, and what is done then.
pub fn rule_said(rule: &Value, zones: &[Value]) -> String {
    let matchers: Vec<String> = rule["matchers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(matcher_said)
        .collect();
    let holds = if matchers.is_empty() {
        "matches nothing yet".to_string()
    } else {
        matchers.join(" and ")
    };
    format!("{holds}  {THEN}  {}", action_said(&rule["action"], zones))
}

// ---------------------------------------------------------------------
// What the rules would do: the plan the zones answer with, as a table.

/// One line of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub item: String,
    /// What is done with it; for an item left alone, why.
    pub does: String,
    /// The rule that decided.
    pub by: String,
}

/// The plan as it is shown: what would be done, and what a rule matched
/// and is left alone all the same.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub steps: Vec<Row>,
    pub held: Vec<Row>,
}

/// What an item is called in the table: the name of its file. An item
/// that is no file (This PC, the bin) is known by a longer name of the
/// system's, which is shown as it is.
pub fn item_name(id: &str) -> String {
    let id = id.trim().trim_end_matches(['/', '\\']);
    let id = id.strip_prefix("file://").unwrap_or(id);
    if id.contains("::{") {
        return id.to_string();
    }
    id.rsplit(['/', '\\']).next().unwrap_or(id).to_string()
}

/// Which rule decided, by its place and its name.
pub fn decided_said(decided: &Value, rules: &[Value]) -> String {
    if let Some(index) = decided["rule"].as_u64() {
        let name = rules
            .get(index as usize)
            .and_then(|rule| rule["name"].as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty());
        return match name {
            Some(name) => format!("rule {}, \u{201c}{name}\u{201d}", index + 1),
            None => format!("rule {}", index + 1),
        };
    }
    if decided == "catch_all" {
        return "no rule matched: what is done with the rest".to_string();
    }
    String::new()
}

/// Why a rule that matched does nothing.
pub fn reason_said(reason: &str) -> String {
    match reason {
        "no_such_zone" => "the rule names a zone that is not there".to_string(),
        "zone_fills_itself" => "that zone fills itself (a folder, or a query)".to_string(),
        "locked" => "the zone is locked".to_string(),
        "not_a_file" => "it is no file that could be moved".to_string(),
        other => other.replace('_', " "),
    }
}

/// The table of a plan (`{"steps": [...], "held": [...]}`), with the
/// rules and the zones it speaks of named as the configuration names
/// them, and the items as the zones that run call them (`names`, by id).
pub fn table(
    plan: &Value,
    rules: &[Value],
    zones: &[Value],
    names: &std::collections::BTreeMap<String, String>,
) -> Table {
    // (A plan names an item by its id. What it is called is told with
    // it, or known from the zones' state, or else read off the id.)
    let item = |entry: &Value| {
        let id = entry["item"].as_str().unwrap_or("");
        entry["name"]
            .as_str()
            .or_else(|| names.get(id).map(String::as_str))
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| item_name(id))
    };
    let rows = |key: &str, does: &dyn Fn(&Value) -> String| -> Vec<Row> {
        plan[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| Row {
                item: item(entry),
                does: does(entry),
                by: decided_said(&entry["decided"], rules),
            })
            .collect()
    };
    Table {
        steps: rows("steps", &|step| action_said(&step["action"], zones)),
        held: rows("held", &|held| {
            reason_said(held["reason"].as_str().unwrap_or(""))
        }),
    }
}

/// The table of what the zones say of a plan line by line (`lines` of
/// their answer: each item with its name, where it would go, the rule
/// that decided and, where it is left alone, why). The zones know what an
/// item is called and what a zone's title is; this is preferred to
/// reading the plan itself wherever the answer has it.
pub fn table_of_lines(lines: &[Value], rules: &[Value]) -> Table {
    let mut table = Table::default();
    for line in lines {
        let item = line["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| item_name(line["item"].as_str().unwrap_or("")));
        // (The rule's name as the zones have it: theirs are the rules
        // that decided, also where the ones here were changed since.)
        let by = match (
            line["decided"]["rule"].as_u64(),
            line["rule"].as_str().map(str::trim),
        ) {
            (Some(index), Some(name)) if !name.is_empty() => {
                format!("rule {}, \u{201c}{name}\u{201d}", index + 1)
            }
            _ => decided_said(&line["decided"], rules),
        };
        if let Some(reason) = line["held"].as_str() {
            table.held.push(Row {
                item,
                does: reason_said(reason),
                by,
            });
            continue;
        }
        let destination = &line["destination"];
        let does = match destination["to"].as_str().unwrap_or("") {
            "zone" => {
                let title = destination["title"]
                    .as_str()
                    .map(str::trim)
                    .filter(|title| !title.is_empty());
                let called = title.or(destination["zone"].as_str()).unwrap_or("");
                format!("into \u{201c}{called}\u{201d}")
            }
            "folder" => format!(
                "the file is moved to {}",
                destination["folder"].as_str().unwrap_or("")
            ),
            other => title(other).to_lowercase(),
        };
        table.steps.push(Row { item, does, by });
    }
    table
}

/// The table in one sentence. `done`: it was carried out, not only asked.
pub fn summed(table: &Table, done: bool) -> String {
    let items = |count: usize| match count {
        1 => "1 item".to_string(),
        count => format!("{count} items"),
    };
    let steps = match (table.steps.len(), done) {
        (0, false) => "Nothing would be moved: every item is where its rule wants it.".to_string(),
        (0, true) => "Nothing was moved: every item was where its rule wants it.".to_string(),
        (1, false) => "1 item would be sorted.".to_string(),
        (1, true) => "1 item was sorted.".to_string(),
        (count, false) => format!("{} would be sorted.", items(count)),
        (count, true) => format!("{} were sorted.", items(count)),
    };
    match table.held.len() {
        0 => steps,
        1 => format!("{steps} 1 item a rule matched is left alone."),
        count => format!("{steps} {} a rule matched are left alone.", items(count)),
    }
}

// ---------------------------------------------------------------------
// Zones made by type: the plan the zones answer with, as a table.

/// One zone of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    pub title: String,
    /// It would be made; else it is there, and takes more.
    pub new: bool,
    /// How many items would go into it.
    pub items: usize,
    /// Where it would be made, and how large.
    pub place: String,
    /// A rule would be added for what comes to the desktop later.
    pub rule: bool,
}

/// The plan as it is shown: the zones, and what is left where it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByType {
    pub rows: Vec<TypeRow>,
    /// The display new zones would be made on.
    pub display: String,
    /// The items nothing would be done with, by their names.
    pub left: Vec<String>,
}

/// The table of a plan of zones by type (`auto_planned.plan`), with the
/// items called as the zones that run call them (`names`, by id).
pub fn by_type(plan: &Value, names: &std::collections::BTreeMap<String, String>) -> ByType {
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let rows = list(&plan["zones"])
        .iter()
        .map(|zone| {
            let rect = &zone["rect"];
            let number = |key: &str| rect[key].as_i64().unwrap_or(0);
            TypeRow {
                title: zone["title"].as_str().unwrap_or("").to_string(),
                new: zone["zone"].is_null(),
                items: list(&zone["items"]).len(),
                place: format!(
                    "{} \u{d7} {} at {}, {}",
                    number("width"),
                    number("height"),
                    number("x"),
                    number("y")
                ),
                rule: zone["rule"].as_bool().unwrap_or(false),
            }
        })
        .collect();
    let left = list(&plan["left"])
        .iter()
        .filter_map(Value::as_str)
        .map(|id| names.get(id).cloned().unwrap_or_else(|| item_name(id)))
        .collect();
    ByType {
        rows,
        display: plan["display"].as_str().unwrap_or("").to_string(),
        left,
    }
}

/// The plan of zones by type in one sentence.
pub fn by_type_summed(table: &ByType) -> String {
    let new = table.rows.iter().filter(|row| row.new).count();
    let items: usize = table.rows.iter().map(|row| row.items).sum();
    let sorted = match items {
        1 => "1 item would be put into its type\u{2019}s zone".to_string(),
        count => format!("{count} items would be put into the zones of their types"),
    };
    let said = match (table.rows.len(), new) {
        (0, _) => "There is nothing to make: no type has enough items for a zone of its own, \
                   and the zones made by type have theirs."
            .to_string(),
        (_, 0) => format!("No zone would be made; {sorted}."),
        (_, 1) => format!("1 zone would be made, and {sorted}."),
        (_, new) => format!("{new} zones would be made, and {sorted}."),
    };
    match table.left.len() {
        0 => said,
        1 => format!("{said} 1 item is left where it is."),
        count => format!("{said} {count} items are left where they are."),
    }
}

/// What is said of zones that were made by type (`auto_created`).
pub fn made_said(event: &Value) -> String {
    let count = |key: &str| event[key].as_u64().unwrap_or(0) as usize;
    let zones = event["zones"].as_array().map_or(0, Vec::len);
    if zones == 0 && count("items") == 0 {
        return "There was nothing to do.".to_string();
    }
    let mut said = match zones {
        0 => format!(
            "{} sorted into the zones made by type before.",
            counted(count("items"), "item")
        ),
        zones => format!(
            "{} made, {} sorted.",
            counted(zones, "zone"),
            counted(count("items"), "item")
        ),
    };
    if count("rules") > 0 {
        said.push_str(&format!(
            " {} added for what comes later.",
            counted(count("rules"), "rule")
        ));
    }
    said
}

/// What is said of zones made by type that were taken back
/// (`auto_undone`).
pub fn unmade_said(event: &Value) -> String {
    let count = |key: &str| event[key].as_u64().unwrap_or(0) as usize;
    format!(
        "Taken back: {} taken away, {} put back, {} taken out.",
        counted(count("zones"), "zone"),
        counted(count("items"), "item"),
        counted(count("rules"), "rule"),
    )
}

// ---------------------------------------------------------------------
// Asking the zones that run.

/// What the zones were asked from this page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    Preview,
    Apply,
    Undo,
    /// An arrangement is kept, put back, or forgotten.
    Snapshot,
    /// An arrangement is asked for as a document, to be written to this
    /// file.
    Export(String),
    Import,
    /// What zones would be made by type.
    TypesPreview,
    TypesMake,
    TypesUndo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    what: Ask,
    command: String,
    at: Instant,
}

/// What came of asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The plan: what would be done, or (`done`) what was.
    Plan {
        table: Table,
        every_item: bool,
        done: bool,
    },
    /// The zones that would be made by type.
    ByType { table: ByType, every_item: bool },
    /// Something to say; `bad`: it did not work.
    Said { bad: bool, text: String },
    /// No zones run that could be asked.
    NotRunning(String),
}

/// Why asking failed, as it is shown. That no zones run is no failure of
/// anything: it is said plainly, with what to do about it. `delivered`:
/// the question reached a shell that runs zones.
pub fn refusal(error: &str, delivered: bool) -> Outcome {
    let lower = error.to_lowercase();
    if lower.contains("switched off") {
        return Outcome::NotRunning(
            "Zones are switched off. Switch them on at the top of this page and apply, then ask again."
                .to_string(),
        );
    }
    if lower.contains("zones are not running") || lower.contains("no gevrek-shell") {
        return Outcome::NotRunning(
            "Zones are not running: no gevrek-shell that shows them is attached to the window manager."
                .to_string(),
        );
    }
    if lower.contains("unimplemented")
        || (lower.contains("unknown variant") && lower.contains("zones"))
    {
        return Outcome::NotRunning(
            "This window manager does not know zones yet (it is an older one).".to_string(),
        );
    }
    if !delivered {
        return Outcome::NotRunning(format!("The zones could not be asked: {error}"));
    }
    Outcome::Said {
        bad: true,
        text: error.to_string(),
    }
}

/// Write an arrangement the zones gave as a document to a file.
pub fn written(document: &Value, file: &str) -> Result<(), String> {
    if !document.is_object() {
        return Err("the zones gave no document".to_string());
    }
    let text = serde_json::to_string_pretty(document).map_err(|error| error.to_string())?;
    let path = widgets::whole_path(file, dirs::home_dir().as_deref());
    let written = std::fs::write(&path, text + "\n");
    match &written {
        Ok(()) => log::debug!("the zones' arrangement was written to {}", path.display()),
        Err(error) => log::debug!(
            "the zones' arrangement could not be written to {}: {error}",
            path.display()
        ),
    }
    written.map_err(|error| error.to_string())
}

/// What is said of an arrangement that was read in (`layout_imported`).
pub fn imported_said(event: &Value) -> String {
    let name = event["name"].as_str().unwrap_or("");
    let mut said = if event["applied"].as_bool().unwrap_or(false) {
        format!("Read in as \u{201c}{name}\u{201d}, and the zones are as it says.")
    } else {
        format!("Read in and kept as \u{201c}{name}\u{201d}.")
    };
    if let Some(before) = event["kept_as"]
        .as_str()
        .filter(|before| !before.is_empty())
    {
        said.push_str(&format!(
            " What there was is kept as \u{201c}{before}\u{201d}."
        ));
    }
    said
}

/// The zones as the window manager last heard of them from the shell
/// that runs them: whether there is anybody to ask, and what the items a
/// plan speaks of are called.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Running {
    /// A shell runs the zones now.
    pub running: bool,
    /// `desktop.zones.enabled`, as the zones that run have it.
    pub enabled: bool,
    /// Why there is nobody to ask, or why zones are not shown although
    /// they are switched on, in the words of whoever said it.
    pub message: String,
    pub zones: usize,
    /// The items of the desktop that are in no zone.
    pub loose: usize,
    /// What each item is called, by its id.
    pub names: std::collections::BTreeMap<String, String>,
    /// The arrangements that are kept by name.
    pub kept: Vec<Kept>,
    /// There is a sorting by the rules that can be taken back.
    pub can_undo: bool,
    /// Zones were made by type, and can be taken back.
    pub can_undo_types: bool,
}

/// An arrangement of the zones that is kept under a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub name: String,
    /// When, in seconds since 1970.
    pub taken_at: u64,
    /// How many zones it holds.
    pub zones: usize,
    /// The zones took it themselves, before something replaced the whole
    /// arrangement.
    pub automatic: bool,
}

impl Kept {
    /// What its line says beside its name.
    pub fn said(&self) -> String {
        let when = std::time::UNIX_EPOCH + Duration::from_secs(self.taken_at);
        let mut said = vec![counted(self.zones, "zone")];
        if self.taken_at > 0 {
            said.push(format!("{} UTC", gevrek_ui::browser::moment_said(when)));
        }
        if self.automatic {
            said.push("kept by the zones themselves".to_string());
        }
        said.join("  \u{b7}  ")
    }
}

impl Running {
    /// From the answer to a question for the zones' state. `json`: the
    /// zones' own account of themselves, empty where there is none.
    pub fn of(running: bool, stale: bool, json: &str, message: &str) -> Self {
        let told: Value = serde_json::from_str(json).unwrap_or(Value::Null);
        // (What a shell that has gone last told is nobody's state now.)
        let told = if stale { &Value::Null } else { &told };
        let mut names = std::collections::BTreeMap::new();
        let mut name = |item: &Value| {
            if let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) {
                names.insert(id.to_string(), name.to_string());
            }
        };
        let zones = told["zones"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let loose = told["loose"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        zones
            .iter()
            .flat_map(|zone| zone["items"].as_array().into_iter().flatten())
            .chain(loose)
            .for_each(&mut name);
        let not_shown = told["not_shown"].as_str().unwrap_or("");
        let kept = told["snapshots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|kept| {
                Some(Kept {
                    name: kept["name"].as_str()?.to_string(),
                    taken_at: kept["taken_at"].as_u64().unwrap_or(0),
                    // (Told as how many, or as the zones themselves.)
                    zones: kept["zones"].as_u64().map_or_else(
                        || kept["zones"].as_array().map_or(0, Vec::len),
                        |count| count as usize,
                    ),
                    automatic: kept["automatic"].as_bool().unwrap_or(false),
                })
            })
            .collect();
        Self {
            running,
            enabled: told["enabled"].as_bool().unwrap_or(false),
            message: if message.trim().is_empty() {
                not_shown.to_string()
            } else {
                message.trim().to_string()
            },
            zones: zones.len(),
            loose: loose.len(),
            names,
            kept,
            // (Zones that do not say are asked, and answer for themselves.)
            can_undo: told["can_undo_rules"].as_bool().unwrap_or(true),
            // (Said only while there is something to take back.)
            can_undo_types: told["can_undo_auto"].as_bool().unwrap_or(false),
        }
    }

    /// Whether there are zones to ask about their rules.
    pub fn can_be_asked(&self) -> bool {
        self.running && self.enabled
    }

    /// What is to be said of them above the buttons, and whether it is
    /// the reason nothing can be asked.
    pub fn said(&self) -> (bool, String) {
        if !self.running {
            let why = if self.message.is_empty() {
                "Zones are not running: no gevrek-shell that shows them is attached to the window manager."
            } else {
                self.message.as_str()
            };
            return (true, why.to_string());
        }
        if !self.enabled {
            return (
                true,
                "Zones are switched off. Switch them on at the top of this page and apply: then their rules can be tried."
                    .to_string(),
            );
        }
        let counted = format!(
            "The zones that run: {}, {} in none.",
            counted(self.zones, "zone"),
            counted(self.loose, "item")
        );
        if self.message.is_empty() {
            (false, counted)
        } else {
            (false, format!("{counted} {}", self.message))
        }
    }
}

/// What stands around the page: what the window says of itself.
pub struct Around<'a> {
    /// A window manager is on the line.
    pub connected: bool,
    /// Something is changed here and not applied yet.
    pub dirty: bool,
    /// The desktop is Windows' (the window manager's, or this program's
    /// where none is connected).
    pub windows: bool,
    /// A setting to draw a ring around (found by search).
    pub highlight: Option<&'a str>,
}

/// What the page asks of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wish {
    /// Ask this of the zones (a zones command, as JSON), through the
    /// window manager.
    Ask(String),
    /// Ask the window manager how the zones are.
    AskState,
    /// Go to the page that has this setting, and show it.
    Show(&'static str),
}

/// The page, with what it remembers between frames.
#[derive(Default)]
pub struct ZonesPage {
    /// The zone whose settings are open, by its place in the list.
    open_zone: Option<usize>,
    open_rule: Option<usize>,
    /// The rules are asked about every item, not only the loose ones.
    every_item: bool,
    asked: Option<Asked>,
    outcome: Option<Outcome>,
    /// The zones that run, once the window manager has said how they are.
    running: Option<Running>,
    state_asked: Option<Instant>,
    /// What came of asking is about the arrangements, and is said in
    /// their card, not in the rules'.
    below: bool,
    /// What came of asking is about zones by type, and is said in their
    /// card.
    of_types: bool,
    /// Zones by type are asked for every item, not only the loose ones.
    types_every_item: bool,
    /// The name typed for an arrangement to keep.
    snapshot: String,
    /// The file an arrangement is written to or read from.
    file: String,
}

impl ZonesPage {
    fn ask(&mut self, what: Ask, command: Value, wishes: &mut Vec<Wish>) {
        // (Only what is asked: an arrangement read in is a whole document.)
        log::debug!(
            "the zones are asked from the studio: {what:?} ({})",
            command["command"].as_str().unwrap_or("?")
        );
        let command = command.to_string();
        self.below = matches!(what, Ask::Snapshot | Ask::Export(_) | Ask::Import);
        self.of_types = matches!(what, Ask::TypesPreview | Ask::TypesMake | Ask::TypesUndo);
        self.outcome = None;
        self.asked = Some(Asked {
            what,
            command: command.clone(),
            at: Instant::now(),
        });
        wishes.push(Wish::Ask(command));
    }

    /// The window manager said how the zones are.
    pub fn state(&mut self, running: bool, stale: bool, json: &str, message: &str) {
        self.running = Some(Running::of(running, stale, json, message));
    }

    /// The zones answered a command (or nobody did). Says whether it was
    /// one this page asked, which shows the answer itself. `events`: what
    /// happened because of it, each the zones' own event as JSON; the plan
    /// of a dry run is among them. `document` names the rules and the
    /// zones a plan speaks of.
    pub fn answered(
        &mut self,
        command: &str,
        success: bool,
        delivered: bool,
        error: Option<&str>,
        events: &[String],
        document: &Document,
    ) -> bool {
        let Some(asked) = self.asked.take_if(|asked| asked.command == command) else {
            return false;
        };
        // (Whatever was done, the zones are other than they were told.)
        self.state_asked = None;
        if !success {
            log::debug!(
                "the zones did not do {:?}{}: {}",
                asked.what,
                if delivered {
                    ""
                } else {
                    " (it did not reach them)"
                },
                error.unwrap_or("refused")
            );
            self.outcome = Some(refusal(error.unwrap_or("refused"), delivered));
            return true;
        }
        log::debug!(
            "the zones did {:?}, after {} ms, with {} events",
            asked.what,
            asked.at.elapsed().as_millis(),
            events.len()
        );
        let events: Vec<Value> = events
            .iter()
            .filter_map(|event| {
                serde_json::from_str(event)
                    .inspect_err(|error| {
                        log::debug!("an event of the zones' answer is not understood: {error}")
                    })
                    .ok()
            })
            .collect();
        let event = |name: &str| events.iter().find(|event| event["event"] == name);
        let list = |pointer: &str| list_at(document, pointer);
        let names = self
            .running
            .as_ref()
            .map(|running| running.names.clone())
            .unwrap_or_default();
        let every_item = self.every_item;
        let types_every_item = self.types_every_item;
        let plan = |event: &Value, done: bool| Outcome::Plan {
            table: match event["lines"].as_array() {
                Some(lines) => table_of_lines(lines, &list(RULES)),
                None => table(&event["plan"], &list(RULES), &list(LIST), &names),
            },
            every_item,
            done,
        };
        let said = |text: String| Outcome::Said { bad: false, text };
        let missing = || Outcome::Said {
            bad: true,
            text: "The zones answered, but did not say what their rules would do.".to_string(),
        };
        self.outcome = Some(match asked.what {
            Ask::Preview => {
                event("rules_previewed").map_or_else(missing, |event| plan(event, false))
            }
            Ask::Apply => event("rules_applied").map_or_else(missing, |event| plan(event, true)),
            Ask::Undo => Outcome::Said {
                bad: false,
                text: "The last sorting was taken back.".to_string(),
            },
            Ask::Snapshot => {
                let called = |event: &Value| event["name"].as_str().unwrap_or("").to_string();
                if let Some(event) = event("snapshot_saved") {
                    said(format!(
                        "The arrangement \u{201c}{}\u{201d} is kept.",
                        called(event)
                    ))
                } else if let Some(event) = event("snapshot_restored") {
                    said(format!(
                        "The arrangement \u{201c}{}\u{201d} is back.",
                        called(event)
                    ))
                } else if let Some(event) = event("snapshot_deleted") {
                    said(format!(
                        "The arrangement \u{201c}{}\u{201d} is gone.",
                        called(event)
                    ))
                } else {
                    said("Done.".to_string())
                }
            }
            Ask::Export(file) => match event("layout_exported") {
                Some(event) => match written(&event["document"], &file) {
                    Ok(()) => said(format!("Written to {file}.")),
                    Err(error) => Outcome::Said {
                        bad: true,
                        text: format!("{file} could not be written: {error}"),
                    },
                },
                None => Outcome::Said {
                    bad: true,
                    text: "The zones answered, but gave nothing to write.".to_string(),
                },
            },
            Ask::Import => match event("layout_imported") {
                Some(event) => said(imported_said(event)),
                None => said("Done.".to_string()),
            },
            Ask::TypesPreview => match event("auto_planned") {
                Some(event) => Outcome::ByType {
                    table: by_type(&event["plan"], &names),
                    every_item: types_every_item,
                },
                None => Outcome::Said {
                    bad: true,
                    text: "The zones answered, but did not say what they would make.".to_string(),
                },
            },
            Ask::TypesMake => event("auto_created")
                .map_or_else(|| said("Done.".to_string()), |event| said(made_said(event))),
            Ask::TypesUndo => event("auto_undone").map_or_else(
                || said("The zones made by type were taken back.".to_string()),
                |event| said(unmade_said(event)),
            ),
        });
        true
    }

    /// Something happened in the window manager. Where it is something
    /// of the zones (`zones.<name>`, whoever asked for it), how they are
    /// is asked again: the arrangements kept by a key or by the command
    /// line are listed here too.
    pub fn heard(&mut self, kind: &str) {
        if kind.starts_with("zones.") {
            self.state_asked = None;
        }
    }

    /// What is asked by itself while the page is shown (how the zones
    /// are, now and then), and what was asked and never answered, said
    /// once it is late.
    fn follow(&mut self, context: &egui::Context, around: &Around<'_>, wishes: &mut Vec<Wish>) {
        if !around.connected {
            self.running = None;
            self.state_asked = None;
        } else if self
            .state_asked
            .is_none_or(|asked| asked.elapsed() > STATE_AGAIN)
        {
            self.state_asked = Some(Instant::now());
            wishes.push(Wish::AskState);
            context.request_repaint_after(STATE_AGAIN);
        }
        if let Some(asked) = &self.asked {
            if asked.at.elapsed() > ANSWER_ARRIVES {
                log::debug!(
                    "the zones did not answer {:?} within {} ms",
                    asked.what,
                    ANSWER_ARRIVES.as_millis()
                );
                self.outcome = Some(Outcome::Said {
                    bad: true,
                    text: "No answer from the window manager.".to_string(),
                });
                self.asked = None;
            } else {
                context.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    /// The page. What it wants of the window is given back.
    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, around: &Around<'_>) -> Vec<Wish> {
        let mut wishes = Vec::new();
        self.follow(ui.ctx(), around, &mut wishes);
        let palette = theme::pal(ui.ctx());
        for card in Page::Zones.cards() {
            let names: &[&str] = match card.show {
                Show::Only(names) => names,
                _ => &[],
            };
            if card.pointer == ZONES && names.contains(&"enabled") {
                widgets::card(ui, card.title, |ui| {
                    switch_card(ui, document, around, &palette, &mut wishes);
                });
            } else if card.pointer == LIST {
                widgets::card(ui, card.title, |ui| {
                    self.zones_card(ui, document, around, &palette)
                });
                widgets::card(ui, "Zones by type", |ui| {
                    self.types_card(ui, around, &palette, &mut wishes);
                });
            } else if card.pointer == ZONES && names.contains(&"rules") {
                widgets::card(ui, card.title, |ui| {
                    self.rules_card(ui, document, around, &palette)
                });
                widgets::card(ui, "What the rules would do", |ui| {
                    self.dry_run_card(ui, around, &palette, &mut wishes);
                });
            } else if card.pointer == "/desktop/zones/look" {
                widgets::card(ui, card.title, |ui| {
                    let look = document
                        .config()
                        .map(|config| config.desktop.zones.look.clone())
                        .unwrap_or_default();
                    look_preview(ui, &look, "Every zone", &palette);
                    ui.add_space(6.0);
                    look_fields(ui, document, card.pointer, around.highlight, &palette);
                });
            } else {
                widgets::card(ui, card.title, |ui| {
                    generic(ui, document, card, around.highlight)
                });
            }
        }
        widgets::card(ui, "Arrangements kept by name", |ui| {
            self.snapshots_card(ui, around, &palette, &mut wishes);
        });
        wishes
    }

    fn zones_card(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        around: &Around<'_>,
        palette: &Palette,
    ) {
        let zones = list_at(document, LIST);
        ui.label(
            RichText::new(
                "What each zone is called, where its items come from and how it shows them. \
                 Where a zone is, is not set here: zones are placed on the desktop itself, \
                 by dragging one by its title and sizing it by its edges.",
            )
            .color(palette.muted),
        );
        said_of(ui, document.issues(), "desktop.zones.zones", palette);
        ui.add_space(6.0);
        if zones.is_empty() {
            ui.label(RichText::new("No zone yet.").color(palette.muted));
        }
        if self.open_zone.is_some_and(|open| open >= zones.len()) {
            self.open_zone = None;
        }
        let kinds = variants_at(&format!("{LIST}/0/kind"));
        let mut change = None;
        for (index, zone) in zones.iter().enumerate() {
            let pointer = format!("{LIST}/{index}");
            let open = self.open_zone == Some(index);
            let frame = entry_frame(ui, palette, |ui| {
                let narrow = ui.available_width() < NARROW;
                // What stands at the end of the line: what there is to
                // say of the zone, and its buttons.
                let mut tail = |ui: &mut Ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut toggled = false;
                        if let Some(asked) =
                            line_buttons(ui, index, zones.len(), open, &mut toggled)
                        {
                            change = Some(asked);
                        }
                        if toggled {
                            self.open_zone = (!open).then_some(index);
                        }
                        let said = zone_said(zone);
                        if !said.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(said).size(12.0).color(palette.muted),
                                )
                                .truncate(),
                            );
                        }
                    });
                };
                ui.horizontal(|ui| {
                    handle(ui, palette, "zones", index);
                    let mut text = zone["title"].as_str().unwrap_or("").to_string();
                    if widgets::text_field(ui, &mut text, 170.0)
                        .on_hover_text("What stands on the zone")
                        .changed()
                    {
                        document.set(&format!("{pointer}/title"), Value::String(text));
                    }
                    kind_picker(ui, document, &pointer, zone, &kinds);
                    if !narrow {
                        tail(ui);
                    }
                });
                // (Where the line has no room for them, below it.)
                if narrow {
                    ui.horizontal(|ui| tail(ui));
                }
                if open {
                    ui.add_space(6.0);
                    ui.separator();
                    zone_body(ui, document, index, zone, around, palette);
                }
            });
            if let Some(dropped) = put_down(ui, &frame, "zones", index, palette) {
                change = Some(dropped);
            }
            ui.add_space(3.0);
        }
        ui.add_space(4.0);
        if widgets::text_button(ui, icons::PLUS, "Add a zone")
            .on_hover_text(
                "It appears on the desktop beside the ones there are, to be put where it belongs.",
            )
            .clicked()
        {
            let zone = another_zone(&zones);
            document.edit(LIST, |list| {
                if let Some(list) = list.as_array_mut() {
                    list.push(zone);
                }
            });
            self.open_zone = Some(zones.len());
        }
        if let Some(change) = change {
            document.edit(LIST, |list| {
                if let Some(list) = list.as_array_mut() {
                    changed(list, change);
                }
            });
            self.open_zone = follows(self.open_zone, zones.len(), change);
        }
    }

    fn rules_card(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        around: &Around<'_>,
        palette: &Palette,
    ) {
        let rules = list_at(document, RULES);
        let zones = list_at(document, LIST);
        ui.label(
            RichText::new(
                "Tried from the first to the last for each item of the desktop: the first rule \
                 whose conditions all hold decides where the item goes.",
            )
            .color(palette.muted),
        );
        said_of(ui, document.issues(), "desktop.zones.rules", palette);
        ui.add_space(6.0);
        if rules.is_empty() {
            ui.label(RichText::new("No rule yet.").color(palette.muted));
        }
        if self.open_rule.is_some_and(|open| open >= rules.len()) {
            self.open_rule = None;
        }
        let mut change = None;
        for (index, rule) in rules.iter().enumerate() {
            let pointer = format!("{RULES}/{index}");
            let open = self.open_rule == Some(index);
            let enabled = rule["enabled"].as_bool().unwrap_or(true);
            let frame = entry_frame(ui, palette, |ui| {
                let narrow = ui.available_width() < NARROW;
                let mut tail = |ui: &mut Ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut toggled = false;
                        if let Some(asked) =
                            line_buttons(ui, index, rules.len(), open, &mut toggled)
                        {
                            change = Some(asked);
                        }
                        if toggled {
                            self.open_rule = (!open).then_some(index);
                        }
                        let ink = if enabled {
                            palette.muted
                        } else {
                            palette.faint
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(rule_said(rule, &zones)).size(12.0).color(ink),
                            )
                            .truncate(),
                        );
                    });
                };
                ui.horizontal(|ui| {
                    handle(ui, palette, "rules", index);
                    let mut on = enabled;
                    if widgets::switch(ui, &mut on)
                        .on_hover_text("A rule that is off is passed over")
                        .changed()
                    {
                        document.set(&format!("{pointer}/enabled"), Value::Bool(on));
                    }
                    let mut text = rule["name"].as_str().unwrap_or("").to_string();
                    if widgets::text_field(ui, &mut text, 170.0)
                        .on_hover_text("What you call the rule")
                        .changed()
                    {
                        document.set(&format!("{pointer}/name"), Value::String(text));
                    }
                    if !narrow {
                        tail(ui);
                    }
                });
                if narrow {
                    ui.horizontal(|ui| tail(ui));
                }
                if open {
                    ui.add_space(6.0);
                    ui.separator();
                    rule_body(ui, document, &pointer, rule, &zones, around, palette);
                }
            });
            if let Some(dropped) = put_down(ui, &frame, "rules", index, palette) {
                change = Some(dropped);
            }
            ui.add_space(3.0);
        }
        ui.add_space(4.0);
        if widgets::text_button(ui, icons::PLUS, "Add a rule").clicked() {
            let rule = serde_json::to_value(gevrek_model::zones::Rule {
                name: format!("Rule {}", rules.len() + 1),
                action: gevrek_model::zones::RuleAction::AssignToZone {
                    zone: gevrek_model::zones::ZoneId::new(first_id(&zones)),
                },
                ..Default::default()
            })
            .unwrap_or(Value::Null);
            document.edit(RULES, |list| {
                if let Some(list) = list.as_array_mut() {
                    list.push(rule);
                }
            });
            self.open_rule = Some(rules.len());
        }
        if let Some(change) = change {
            document.edit(RULES, |list| {
                if let Some(list) = list.as_array_mut() {
                    changed(list, change);
                }
            });
            self.open_rule = follows(self.open_rule, rules.len(), change);
        }

        // What no rule matches.
        ui.add_space(10.0);
        widgets::caption(ui, "What no rule matches");
        said_of(ui, document.issues(), "desktop.zones.catch_all", palette);
        let present = document
            .get(CATCH_ALL)
            .is_some_and(|action| !action.is_null());
        let mut wanted = present;
        ui.horizontal(|ui| {
            widgets::switch(ui, &mut wanted);
            ui.label(if present {
                "goes where this says:"
            } else {
                "stays where it is"
            });
        });
        if wanted != present {
            let action = if wanted {
                json!({"action": "assign_to_zone", "zone": first_id(&zones)})
            } else {
                Value::Null
            };
            document.set(CATCH_ALL, action);
        }
        if wanted && present {
            let action = document.get(CATCH_ALL).cloned().unwrap_or(Value::Null);
            ui.indent("what no rule matches", |ui| {
                action_editor(ui, document, CATCH_ALL, &action, &zones, around.highlight);
            });
        }
        ui.add_space(4.0);
        let mut form = Form::new(document);
        form.highlight = around.highlight;
        // When the rules are tried by themselves.
        form.only(ui, ZONES, &["rules_on_arrival", "timed_rules"]);
    }

    fn dry_run_card(
        &mut self,
        ui: &mut Ui,
        around: &Around<'_>,
        palette: &Palette,
        wishes: &mut Vec<Wish>,
    ) {
        ui.label(
            RichText::new(
                "Ask the zones that run now what their rules would do with what is on the \
                 desktop. Nothing is moved by asking.",
            )
            .color(palette.muted),
        );
        ui.add_space(6.0);
        if around.dirty {
            widgets::banner(ui, palette.warn, icons::INFO, |ui| {
                ui.label(
                    "The zones that run have the rules as they were last applied. \
                     Apply what is changed here first, then ask.",
                );
            });
            ui.add_space(6.0);
        }
        // Whether there is anybody to ask, in the window manager's words.
        let said = self.running.as_ref().map(Running::said);
        if let Some((hinders, text)) = &said {
            if *hinders {
                widgets::banner(ui, palette.muted, icons::INFO, |ui| {
                    ui.label(text);
                });
            } else {
                ui.label(RichText::new(text).size(12.0).color(palette.faint));
            }
            ui.add_space(6.0);
        }
        let there = self.running.as_ref().is_none_or(Running::can_be_asked);
        let scope = |every_item: bool| if every_item { "all" } else { "loose" };
        let waiting = self.asked.is_some();
        // What is shown is the answer for the items that are asked about
        // now, and something would be done: only then can it be carried out.
        let ready = matches!(
            &self.outcome,
            Some(Outcome::Plan { table, every_item, done: false })
                if *every_item == self.every_item && !table.steps.is_empty()
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Ask about");
            let options = [(false, "items in no zone"), (true, "every item")];
            if let Some(picked) = widgets::segmented(
                ui,
                ui.scope_id().with("which items the rules are asked about"),
                Some(&self.every_item),
                &options,
                false,
            ) {
                self.every_item = picked;
            }
        });
        ui.label(
            RichText::new(if self.every_item {
                "Every item: also what was sorted by hand is sorted again."
            } else {
                "Items in no zone: what was sorted by hand stays where it was put."
            })
            .size(12.0)
            .color(palette.faint),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let can = around.connected && there && !waiting;
            if widgets::primary_button(ui, can, format!("{}  Dry run", icons::PLAY)).clicked() {
                self.outcome = None;
                self.ask(
                    Ask::Preview,
                    json!({"command": "preview_rules", "scope": scope(self.every_item)}),
                    wishes,
                );
            }
            if ui
                .add_enabled(can && ready, egui::Button::new("Apply the rules"))
                .on_hover_text("Do what the table says.")
                .on_disabled_hover_text("After a dry run that has something to do: first see what would happen.")
                .clicked()
            {
                self.ask(
                    Ask::Apply,
                    json!({"command": "apply_rules", "scope": scope(self.every_item)}),
                    wishes,
                );
            }
            if ui
                .add_enabled(
                    can && self.running.as_ref().is_none_or(|running| running.can_undo),
                    egui::Button::new("Undo the last sorting"),
                )
                .on_hover_text("Everything goes back where it was before the rules were last applied, files that were moved too.")
                .clicked()
            {
                self.ask(Ask::Undo, json!({"command": "undo_rules"}), wishes);
            }
            if waiting {
                widgets::busy(ui);
            }
            if !around.connected {
                ui.label(RichText::new("when a window manager is connected").color(palette.muted));
            }
        });
        let Some(outcome) = self
            .outcome
            .as_ref()
            .filter(|_| !self.below && !self.of_types)
        else {
            return;
        };
        ui.add_space(8.0);
        outcome_said(ui, outcome, palette);
    }

    /// Zones made by type: asked for, shown, made, and taken back.
    fn types_card(
        &mut self,
        ui: &mut Ui,
        around: &Around<'_>,
        palette: &Palette,
        wishes: &mut Vec<Wish>,
    ) {
        ui.label(
            RichText::new(
                "A zone for each type of item on the desktop (folders, programs, documents, \
                 pictures and so on), made by the zones that run and filled with what is of its \
                 type. First they say what they would make: nothing is made by asking. Which \
                 types there are, and how many items a type needs for a zone of its own, is set \
                 under \u{201c}How they behave\u{201d}.",
            )
            .color(palette.muted),
        );
        ui.add_space(6.0);
        if around.dirty {
            widgets::banner(ui, palette.warn, icons::INFO, |ui| {
                ui.label(
                    "The zones that run have the types as they were last applied. \
                     Apply what is changed here first, then ask.",
                );
            });
            ui.add_space(6.0);
        }
        if let Some((true, text)) = self.running.as_ref().map(Running::said) {
            widgets::banner(ui, palette.muted, icons::INFO, |ui| {
                ui.label(text);
            });
            ui.add_space(6.0);
        }
        let there = self.running.as_ref().is_none_or(Running::can_be_asked);
        let scope = |every_item: bool| if every_item { "all" } else { "loose" };
        let waiting = self.asked.is_some();
        // What is shown is the answer for the items that are asked about
        // now, and something would be done: only then can it be made.
        let ready = self.of_types
            && matches!(
                &self.outcome,
                Some(Outcome::ByType { table, every_item })
                    if *every_item == self.types_every_item && !table.rows.is_empty()
            );
        ui.horizontal_wrapped(|ui| {
            ui.label("Sort");
            let options = [
                (false, "items in no zone"),
                (true, "also what zones made by hand hold"),
            ];
            if let Some(picked) = widgets::segmented(
                ui,
                ui.scope_id().with("which items are sorted by type"),
                Some(&self.types_every_item),
                &options,
                false,
            ) {
                self.types_every_item = picked;
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let can = around.connected && there && !waiting;
            if widgets::primary_button(ui, can, format!("{}  Make zones by type\u{2026}", icons::PLAY))
                .on_hover_text("Shows what would be made, and where. Nothing is made yet.")
                .clicked()
            {
                self.ask(
                    Ask::TypesPreview,
                    json!({"command": "auto_create", "scope": scope(self.types_every_item), "dry_run": true}),
                    wishes,
                );
            }
            if ui
                .add_enabled(can && ready, egui::Button::new("Make them"))
                .on_hover_text("Make the zones the table names, and put the items into them.")
                .on_disabled_hover_text("First see what would be made.")
                .clicked()
            {
                self.ask(
                    Ask::TypesMake,
                    json!({"command": "auto_create", "scope": scope(self.types_every_item)}),
                    wishes,
                );
            }
            if self.running.as_ref().is_some_and(|running| running.can_undo_types)
                && ui
                    .add_enabled(can, egui::Button::new("Take the automatic zones back"))
                    .on_hover_text(
                        "The zones that were last made by type go, their items go back where they \
                         were, and the rules that were added with them are taken out.",
                    )
                    .clicked()
            {
                self.ask(Ask::TypesUndo, json!({"command": "undo_auto_create"}), wishes);
            }
            if waiting && self.of_types {
                widgets::busy(ui);
            }
            if !around.connected {
                ui.label(RichText::new("when a window manager is connected").color(palette.muted));
            }
        });
        let Some(outcome) = self.outcome.as_ref().filter(|_| self.of_types) else {
            return;
        };
        ui.add_space(8.0);
        outcome_said(ui, outcome, palette);
    }

    fn snapshots_card(
        &mut self,
        ui: &mut Ui,
        around: &Around<'_>,
        palette: &Palette,
        wishes: &mut Vec<Wish>,
    ) {
        ui.label(
            RichText::new(
                "The zones, what lies in each and where each is, kept under a name and put \
                 back by it. Kept by the zones that run, beside their own state: not in the \
                 configuration.",
            )
            .color(palette.muted),
        );
        ui.add_space(6.0);
        let can = around.connected
            && self.asked.is_none()
            && self.running.as_ref().is_none_or(|running| running.running);
        let kept: Vec<Kept> = self
            .running
            .as_ref()
            .map(|running| running.kept.clone())
            .unwrap_or_default();
        let file = self.file.trim().to_string();
        let mut asked: Option<(Ask, Value)> = None;
        // The ones there are: each a line.
        for one in &kept {
            entry_frame(ui, palette, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&one.name).strong());
                    ui.label(RichText::new(one.said()).size(12.0).color(palette.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(can, |ui| {
                            if widgets::small_icon_button(ui, icons::TRASH, "Forget it").clicked() {
                                asked = Some((Ask::Snapshot, json!({"command": "delete_snapshot", "name": one.name})));
                            }
                            if ui
                                .add_enabled(!file.is_empty(), egui::Button::new("Write to the file"))
                                .on_hover_text("Write this arrangement to the file named below.")
                                .on_disabled_hover_text("Name a file below first.")
                                .clicked()
                            {
                                asked = Some((
                                    Ask::Export(file.clone()),
                                    json!({"command": "export_layout", "name": one.name}),
                                ));
                            }
                            if ui
                                .button("Put back")
                                .on_hover_text(
                                    "Make the zones as they were when this was kept. What there is now is kept first.",
                                )
                                .clicked()
                            {
                                asked = Some((Ask::Snapshot, json!({"command": "restore_snapshot", "name": one.name})));
                            }
                        });
                    });
                });
            });
            ui.add_space(3.0);
        }
        if kept.is_empty() && self.running.as_ref().is_some_and(|running| running.running) {
            ui.label(RichText::new("None is kept yet.").color(palette.muted));
        }
        ui.add_space(6.0);
        // One more, of the zones as they are now.
        let name = self.snapshot.trim().to_string();
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.snapshot)
                    .hint_text("a name")
                    .desired_width(190.0),
            );
            if ui
                .add_enabled(
                    can && !name.is_empty(),
                    egui::Button::new("Keep the zones as they are now"),
                )
                .on_hover_text("Under this name. One that is kept under it already is replaced.")
                .clicked()
            {
                asked = Some((
                    Ask::Snapshot,
                    json!({"command": "save_snapshot", "name": name}),
                ));
            }
            if !around.connected {
                ui.label(RichText::new("when a window manager is connected").color(palette.muted));
            }
        });
        // To a file and from one: for another machine, or for safekeeping.
        ui.add_space(10.0);
        widgets::caption(ui, "As a file");
        ui.horizontal_wrapped(|ui| {
            let home = dirs::home_dir();
            widgets::path_field_of(
                ui,
                &mut self.file,
                widgets::PathKind::File,
                home.as_deref(),
                280.0,
                "Arrangements",
                &["json"],
            );
            let named = can && !file.is_empty();
            if ui
                .add_enabled(named, egui::Button::new("Write the zones as they are now"))
                .on_hover_text("The file is written anew: what it held is replaced.")
                .clicked()
            {
                asked = Some((Ask::Export(file.clone()), json!({"command": "export_layout"})));
            }
            if ui
                .add_enabled(named, egui::Button::new("Read it in"))
                .on_hover_text(
                    "The zones become what the file says. What there is now is kept first, by the zones themselves.",
                )
                .clicked()
            {
                let whole = widgets::whole_path(&file, home.as_deref());
                match std::fs::read_to_string(&whole)
                    .map_err(|error| error.to_string())
                    .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()))
                {
                    Ok(document) => {
                        asked = Some((
                            Ask::Import,
                            json!({"command": "import_layout", "apply": true, "document": document}),
                        ));
                    }
                    Err(error) => {
                        log::debug!("an arrangement could not be read from {}: {error}", whole.display());
                        self.below = true;
                        self.outcome = Some(Outcome::Said {
                            bad: true,
                            text: format!("{file} could not be read: {error}"),
                        });
                    }
                }
            }
        });
        if let Some((what, command)) = asked {
            self.outcome = None;
            self.ask(what, command, wishes);
        }
        // What came of it, where it was asked.
        match self.outcome.as_ref().filter(|_| self.below) {
            Some(Outcome::Said { bad, text }) => {
                ui.add_space(6.0);
                let (colour, icon) = if *bad {
                    (palette.error, icons::X_CIRCLE)
                } else {
                    (palette.ok, icons::CHECK_CIRCLE)
                };
                widgets::banner(ui, colour, icon, |ui| {
                    ui.label(text);
                });
            }
            Some(Outcome::NotRunning(text)) => {
                ui.add_space(6.0);
                widgets::banner(ui, palette.muted, icons::INFO, |ui| {
                    ui.label(text);
                });
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// The cards' parts.

fn list_at(document: &Document, pointer: &str) -> Vec<Value> {
    document
        .get(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The id of the first zone: what a new rule names until it is told.
fn first_id(zones: &[Value]) -> String {
    zones
        .first()
        .and_then(|zone| zone["id"].as_str())
        .unwrap_or("")
        .to_string()
}

/// The forms the value at `pointer` can have, where it is one of several.
fn variants_at(pointer: &str) -> Vec<Variant<'static>> {
    match gevrek_schema::schema().at(pointer).map(|node| node.kind()) {
        Some(Kind::Union(variants)) => variants,
        Some(Kind::Optional(inner)) => match inner.kind() {
            Kind::Union(variants) => variants,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn fields_of<'a>(variant: &'a Variant<'static>) -> &'a [Field<'static>] {
    match &variant.shape {
        VariantShape::Tagged { fields, .. } => fields,
        _ => &[],
    }
}

/// A card that is the form and nothing else, as every page draws one.
fn generic(ui: &mut Ui, document: &mut Document, card: &Card, highlight: Option<&str>) {
    let mut form = Form::new(document);
    form.highlight = highlight;
    match card.show {
        Show::Place => {
            if let Some(node) = gevrek_schema::schema().at(card.pointer) {
                form.place(ui, card.pointer, node, card.title, 0);
            }
        }
        Show::AllBut(skip) | Show::Rest(skip) => form.fields(ui, card.pointer, skip),
        Show::Only(names) => form.only(ui, card.pointer, names),
    }
}

/// The page's part of the problems the configuration's own check finds,
/// in the card they are about: errors, and what is only worth knowing.
fn said_of(ui: &mut Ui, issues: &[ConfigIssue], under: &str, palette: &Palette) {
    for (bad, text) in issues_under(issues, under) {
        ui.add_space(4.0);
        let (colour, icon) = if bad {
            (palette.error, icons::X_CIRCLE)
        } else {
            (palette.warn, icons::WARNING)
        };
        widgets::banner(ui, colour, icon, |ui| {
            ui.label(text);
        });
    }
}

/// The issues at or under a place (`desktop.zones.rules`), each as
/// whether it is an error and what to say: where it is, in the words of
/// the page, and the check's own message.
pub fn issues_under(issues: &[ConfigIssue], under: &str) -> Vec<(bool, String)> {
    issues
        .iter()
        .filter(|issue| {
            issue.path == under
                || issue
                    .path
                    .strip_prefix(under)
                    .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
        })
        .map(|issue| {
            let bad = issue.severity != ConfigIssueSeverity::Warning;
            match place_said(&issue.path) {
                place if place.is_empty() => (bad, issue.message.clone()),
                place => (bad, format!("{place}: {}", issue.message)),
            }
        })
        .collect()
}

/// Where in the section a path of the check points, as the page numbers
/// its lines: `desktop.zones.rules[1].action.zone` is "Rule 2".
pub fn place_said(path: &str) -> String {
    let rest = path.strip_prefix("desktop.zones.").unwrap_or(path);
    let numbered = |list: &str, word: &str| -> Option<String> {
        let rest = rest.strip_prefix(list)?.strip_prefix('[')?;
        let index: usize = rest.split(']').next()?.parse().ok()?;
        Some(format!("{word} {}", index + 1))
    };
    numbered("zones", "Zone")
        .or_else(|| numbered("rules", "Rule"))
        .or_else(|| {
            rest.starts_with("catch_all")
                .then(|| "What no rule matches".to_string())
        })
        .unwrap_or_default()
}

/// The frame of one entry of a list.
fn entry_frame(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui)) -> egui::Response {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 4.0).max(200.0));
            add(ui);
        })
        .response
}

/// An entry that is being dragged: of which list, and which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dragged {
    list: &'static str,
    index: usize,
}

/// What an entry is taken up by, and its number.
fn handle(ui: &mut Ui, palette: &Palette, list: &'static str, index: usize) {
    let id = egui::Id::unique(("an entry of the zones' page to put in order", list, index));
    ui.dnd_drag_source(id, Dragged { list, index }, |ui| {
        ui.label(RichText::new(icons::DOTS_SIX_VERTICAL).color(palette.faint));
    })
    .response
    .on_hover_text("Drag to put it in another place");
    ui.label(
        RichText::new(format!("{}.", index + 1))
            .monospace()
            .color(palette.faint),
    );
}

/// An entry of the same list put down on this one: before it, or behind
/// it when it is let go over its lower half.
fn put_down(
    ui: &Ui,
    frame: &egui::Response,
    list: &'static str,
    index: usize,
    palette: &Palette,
) -> Option<Change> {
    let behind = ui
        .ctx()
        .pointer_interact_pos()
        .is_some_and(|pointer| pointer.y > frame.rect.center().y);
    if frame
        .dnd_hover_payload::<Dragged>()
        .is_some_and(|dragged| dragged.list == list)
    {
        let y = if behind {
            frame.rect.bottom() + 1.0
        } else {
            frame.rect.top() - 1.0
        };
        ui.painter().hline(
            frame.rect.x_range(),
            y,
            egui::Stroke::new(2.0, palette.accent),
        );
    }
    let dragged = frame.dnd_release_payload::<Dragged>()?;
    (dragged.list == list).then_some(Change::Move {
        from: dragged.index,
        to: index + usize::from(behind),
    })
}

/// The buttons at the end of an entry's line (drawn from the right).
fn line_buttons(
    ui: &mut Ui,
    index: usize,
    count: usize,
    open: bool,
    toggled: &mut bool,
) -> Option<Change> {
    let mut change = None;
    if widgets::small_icon_button(ui, icons::TRASH, "Take it out").clicked() {
        change = Some(Change::Remove(index));
    }
    let caret = if open {
        icons::CARET_UP
    } else {
        icons::CARET_DOWN
    };
    if widgets::small_icon_button(ui, caret, "Its settings").clicked() {
        *toggled = true;
    }
    if index + 1 < count && widgets::small_icon_button(ui, icons::ARROW_DOWN, "Move down").clicked()
    {
        change = Change::down(index, count);
    }
    if index > 0 && widgets::small_icon_button(ui, icons::ARROW_UP, "Move up").clicked() {
        change = Change::up(index);
    }
    change
}

/// The switch, and what switching it on does to the desktop.
fn switch_card(
    ui: &mut Ui,
    document: &mut Document,
    around: &Around<'_>,
    palette: &Palette,
    wishes: &mut Vec<Wish>,
) {
    let pointer = format!("{ZONES}/enabled");
    // (Zones are shown unless the configuration says no.)
    let on = document
        .get(&pointer)
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let is_set = document
        .default_at(&pointer)
        .is_some_and(|default| default != &Value::Bool(on));
    let mut wanted = on;
    let ((), reset) = widgets::row(
        ui,
        "Show zones on the desktop",
        "Containers on the desktop that hold its icons. They are shown unless this is switched off. Off: the desktop is as the system shows it.",
        is_set,
        around.highlight == Some(pointer.as_str()),
        |ui| {
            widgets::switch(ui, &mut wanted);
        },
    );
    if reset {
        document.reset(&pointer);
    } else if wanted != on {
        document.set(&pointer, Value::Bool(wanted));
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "These are the zones that hold the desktop's icons, not the places windows snap \
             to (those are on the page Snap zones).",
        )
        .color(palette.muted),
    );
    ui.add_space(6.0);
    let hides = document
        .get(HIDE_ICONS)
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let words = if hides {
        "On Windows, while zones are shown, Explorer's own desktop icons are put away: the \
         zones draw every item of the desktop instead. They are put back when zones are \
         switched off and when gevrek ends, and at any time by the command \
         \u{201c}gevrek zones restore-explorer\u{201d}."
    } else {
        "On Windows, Explorer's own desktop icons are left as they are (the setting \
         \u{201c}Hide desktop icons\u{201d} is off): they lie under the zones and cannot be \
         pressed. That is for trying zones, not for living with them."
    };
    let mut link = |ui: &mut Ui| {
        if ui
            .link("The setting is with Windows' own, on \u{201c}This system\u{201d}")
            .clicked()
        {
            wishes.push(Wish::Show(HIDE_ICONS));
        }
    };
    if around.windows {
        widgets::banner(ui, palette.warn, icons::WARNING, |ui| {
            ui.label(words);
            link(ui);
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(words).size(12.5).color(palette.muted));
            link(ui);
        });
    }
}

/// What a kind of zone is called where one is chosen.
fn kind_called(name: &str) -> String {
    match name {
        "manual" => "Filled by hand".to_string(),
        "portal" => "Shows a folder".to_string(),
        "smart" => "Shows what matches".to_string(),
        other => title(other),
    }
}

/// Where a zone's items come from: the one that is chosen, the others on
/// a press.
fn kind_picker(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    zone: &Value,
    kinds: &[Variant<'static>],
) {
    let chosen = zone["kind"]["type"].as_str().unwrap_or("").to_string();
    let mut options: Vec<(String, String, String)> = kinds
        .iter()
        .map(|kind| {
            (
                kind.name.to_string(),
                kind_called(kind.name),
                kind.description.to_string(),
            )
        })
        .collect();
    if !options.iter().any(|(name, _, _)| *name == chosen) {
        options.insert(0, (chosen.clone(), kind_called(&chosen), String::new()));
    }
    if let Some(picked) = widgets::dropdown(
        ui,
        &format!("the kind of {pointer}"),
        &chosen,
        &options,
        170.0,
    ) {
        if let Some(kind) = kinds.iter().find(|kind| kind.name == picked) {
            document.set(&format!("{pointer}/kind"), kind.blank());
        }
    }
}

/// A text that is kept to itself while it is typed and given back when
/// the field is left: a name other things follow is not followed letter
/// by letter.
fn held_text(ui: &mut Ui, id: egui::Id, current: &str, width: f32) -> Option<String> {
    let typed: Option<String> = ui.data(|data| data.get_temp(id));
    let mut text = typed.clone().unwrap_or_else(|| current.to_string());
    let response = widgets::text_field(ui, &mut text, width);
    if response.changed() {
        ui.data_mut(|data| data.insert_temp(id, text.clone()));
    }
    if (typed.is_some() || response.changed()) && !response.has_focus() {
        ui.data_mut(|data| data.remove::<String>(id));
        return (text != current).then_some(text);
    }
    None
}

/// The settings of the zone that is open.
fn zone_body(
    ui: &mut Ui,
    document: &mut Document,
    index: usize,
    zone: &Value,
    around: &Around<'_>,
    palette: &Palette,
) {
    let pointer = format!("{LIST}/{index}");
    let id = zone["id"].as_str().unwrap_or("").to_string();
    let mut renamed = None;
    widgets::row(
        ui,
        "Name for rules and commands",
        "What rules, keys and scripts know the zone by. Rules that name it follow when it is changed.",
        false,
        false,
        |ui| {
            renamed = held_text(ui, ui.scope_id().with(("the id of a zone", index)), &id, 200.0);
        },
    );
    if let Some(new) = renamed {
        document.edit(ZONES, |section| rename_zone(section, index, new.trim()));
    }
    // What its kind needs: the folder it shows, what its items must match.
    let kinds = variants_at(&format!("{pointer}/kind"));
    if let Some(kind) = kinds.iter().find(|kind| kind.matches(&zone["kind"])) {
        for field in fields_of(kind) {
            let place = format!("{pointer}/kind/{}", field.name);
            match field.node.kind() {
                Kind::Text if field.name == "directory" => folder_row(
                    ui,
                    document,
                    &place,
                    "The zone shows what is in this folder, and what is dropped on it is moved there.",
                ),
                Kind::List(item) if matches!(item.kind(), Kind::Union(_)) => {
                    // What it looks through, then what it asks of it.
                    source_row(ui, document, &format!("{pointer}/source"), &zone["source"], around);
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Of what is there it shows whatever all of these hold for:")
                            .color(palette.muted),
                    );
                    matchers(ui, document, &place, around.highlight);
                    let mut form = Form::new(document);
                    form.highlight = around.highlight;
                    form.only(ui, &pointer, &["limit"]);
                    ui.add_space(4.0);
                }
                _ => {
                    let mut form = Form::new(document);
                    form.highlight = around.highlight;
                    form.place(ui, &place, field.node, &title(field.name), 1);
                }
            }
        }
    }
    let mut form = Form::new(document);
    form.highlight = around.highlight;
    // (What a zone that fills itself looks through, and how much of it
    // it shows, stand with its query above; other zones have no use for
    // them.)
    form.fields(
        ui,
        &pointer,
        &["id", "title", "kind", "look", "source", "limit"],
    );
    egui::CollapsingHeader::new("A look of its own")
        .id_salt(("the look of a zone", index))
        .show(ui, |ui| {
            ui.label(
                RichText::new("What is not set here is as every zone has it.")
                    .size(12.0)
                    .color(palette.muted),
            );
            if let Some(config) = document.config() {
                let zones = &config.desktop.zones;
                if let Some(zone) = zones.zones.get(index) {
                    let look = zone.look.over(&zones.look);
                    let title = zone.title.clone();
                    look_preview(ui, &look, &title, palette);
                    ui.add_space(6.0);
                }
            }
            look_fields(
                ui,
                document,
                &format!("{pointer}/look"),
                around.highlight,
                palette,
            );
        });
}

/// A folder, chosen as a folder, on a row of its own.
fn folder_row(ui: &mut Ui, document: &mut Document, place: &str, help: &str) {
    let mut folder = document
        .get(place)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut changed = false;
    widgets::row(ui, "Folder", help, false, false, |ui| {
        let home = dirs::home_dir();
        changed = widgets::path_field(
            ui,
            &mut folder,
            widgets::PathKind::Folder,
            home.as_deref(),
            260.0,
        )
        .changed();
    });
    if changed {
        document.set(place, Value::String(folder));
    }
}

/// What a source of a zone that fills itself is called where one is
/// chosen.
fn source_called(name: &str) -> String {
    match name {
        "desktop" => "The desktop".to_string(),
        "recent" => "The files used last".to_string(),
        "downloads" => "The downloads".to_string(),
        "running_apps" => "The programs that run".to_string(),
        "pinned" => "The pinned launchers".to_string(),
        "folder" => "A folder".to_string(),
        other => title(other),
    }
}

/// What a zone that fills itself looks through: the one that is chosen,
/// the others on a press; and the folder, where it is one.
fn source_row(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    source: &Value,
    around: &Around<'_>,
) {
    let kinds = variants_at(pointer);
    // (A model that has no sources: the desktop is the only one.)
    if kinds.is_empty() {
        return;
    }
    let chosen = source["from"].as_str().unwrap_or("").to_string();
    let mut options: Vec<(String, String, String)> = kinds
        .iter()
        .map(|kind| {
            (
                kind.name.to_string(),
                source_called(kind.name),
                kind.description.to_string(),
            )
        })
        .collect();
    if !options.iter().any(|(name, _, _)| *name == chosen) {
        options.insert(0, (chosen.clone(), source_called(&chosen), String::new()));
    }
    let mut picked = None;
    widgets::row(
        ui,
        "Looks through",
        "What the zone's conditions are asked of.",
        false,
        around.highlight == Some(pointer),
        |ui| {
            picked = widgets::dropdown(
                ui,
                &format!("the source at {pointer}"),
                &chosen,
                &options,
                220.0,
            );
        },
    );
    if let Some(kind) = picked.and_then(|picked| kinds.iter().find(|kind| kind.name == picked)) {
        document.set(pointer, kind.blank());
        return;
    }
    for field in kinds
        .iter()
        .find(|kind| kind.matches(source))
        .map(fields_of)
        .unwrap_or_default()
    {
        let place = format!("{pointer}/{}", field.name);
        if field.name == "directory" {
            folder_row(ui, document, &place, "The zone looks into this folder.");
        } else {
            let mut form = Form::new(document);
            form.highlight = around.highlight;
            form.place(ui, &place, field.node, &title(field.name), 1);
        }
    }
}

/// The settings of the rule that is open.
fn rule_body(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    rule: &Value,
    zones: &[Value],
    around: &Around<'_>,
    palette: &Palette,
) {
    widgets::caption(ui, "When all of these hold");
    if rule["matchers"].as_array().is_none_or(Vec::is_empty) {
        ui.label(
            RichText::new(
                "A rule without a condition matches nothing. What is to happen to everything \
                 the rules leave is said below them, on purpose.",
            )
            .size(12.0)
            .color(palette.muted),
        );
    }
    matchers(
        ui,
        document,
        &format!("{pointer}/matchers"),
        around.highlight,
    );
    ui.add_space(6.0);
    widgets::caption(ui, "The item");
    action_editor(
        ui,
        document,
        &format!("{pointer}/action"),
        &rule["action"],
        zones,
        around.highlight,
    );
    // What a rule has beside these (a field of a newer model): as a form.
    let mut form = Form::new(document);
    form.highlight = around.highlight;
    form.fields(ui, pointer, &["name", "enabled", "matchers", "action"]);
}

/// What a condition is called where one is chosen.
fn matcher_called(name: &str) -> String {
    match name {
        "extension" => "Extension is one of".to_string(),
        "mime" => "Type (MIME) fits".to_string(),
        "name" => "Name fits".to_string(),
        "kind" => "Is a".to_string(),
        "date" => "Date".to_string(),
        "size" => "Size (bytes)".to_string(),
        "shortcut_target" => "Shortcut leads to".to_string(),
        "untouched" => "Untouched for (days)".to_string(),
        other => title(other),
    }
}

/// A list of conditions (a rule's, a zone's query): each a line with
/// what is asked and what it is asked against.
fn matchers(ui: &mut Ui, document: &mut Document, pointer: &str, highlight: Option<&str>) {
    let kinds = variants_at(&format!("{pointer}/0"));
    let list = list_at(document, pointer);
    let options: Vec<(String, String, String)> = kinds
        .iter()
        .map(|kind| {
            (
                kind.name.to_string(),
                matcher_called(kind.name),
                kind.description.to_string(),
            )
        })
        .collect();
    let mut remove = None;
    for (index, matcher) in list.iter().enumerate() {
        let place = format!("{pointer}/{index}");
        let chosen = kinds.iter().find(|kind| kind.matches(matcher));
        let fields = chosen.map(fields_of).unwrap_or_default();
        // One thing to say: beside what is asked. More: below it.
        let beside = match fields {
            [field] if fits_a_line(&field.node.kind()) => Some(field),
            _ => None,
        };
        ui.push_id(&place, |ui| {
            ui.horizontal_wrapped(|ui| {
                let name = matcher["match"].as_str().unwrap_or("").to_string();
                let mut options = options.clone();
                if chosen.is_none() {
                    options.insert(0, (name.clone(), matcher_called(&name), String::new()));
                }
                if let Some(picked) = widgets::dropdown(ui, "what is asked", &name, &options, 190.0)
                {
                    if let Some(kind) = kinds.iter().find(|kind| kind.name == picked) {
                        document.set(&place, kind.blank());
                    }
                }
                if let Some(field) = beside {
                    let at = format!("{place}/{}", field.name);
                    let current = matcher[field.name].clone();
                    if let Some(new) = on_a_line(ui, &field.node.kind(), &current) {
                        document.set(&at, new);
                    }
                }
                if widgets::small_icon_button(ui, icons::TRASH, "Take this condition out").clicked()
                {
                    remove = Some(index);
                }
            });
            if beside.is_none() && !fields.is_empty() {
                ui.indent("what it is asked against", |ui| {
                    let mut form = Form::new(document);
                    form.highlight = highlight;
                    for field in fields {
                        form.place(
                            ui,
                            &format!("{place}/{}", field.name),
                            field.node,
                            &title(field.name),
                            1,
                        );
                    }
                });
            }
        });
    }
    if widgets::text_button(ui, icons::PLUS, "Another condition").clicked() {
        if let Some(first) = kinds.first() {
            let new = first.blank();
            document.edit(pointer, |list| {
                if let Some(list) = list.as_array_mut() {
                    list.push(new);
                }
            });
        }
    }
    if let Some(index) = remove {
        document.edit(pointer, |list| {
            if let Some(list) = list.as_array_mut() {
                if index < list.len() {
                    list.remove(index);
                }
            }
        });
    }
}

/// Whether a condition's one value has a control that stands beside it.
fn fits_a_line(kind: &Kind<'_>) -> bool {
    match kind {
        Kind::Text | Kind::Integer { .. } => true,
        Kind::List(item) => matches!(item.kind(), Kind::Text | Kind::Choice(_)),
        _ => false,
    }
}

/// The control for a condition's one value. A list of short texts is one
/// text, parted by commas; a list of words to choose from is a tick each.
fn on_a_line(ui: &mut Ui, kind: &Kind<'_>, current: &Value) -> Option<Value> {
    match kind {
        Kind::Text => {
            let mut text = current.as_str().unwrap_or("").to_string();
            widgets::text_field(ui, &mut text, 220.0)
                .on_hover_text("* stands for any run of letters, ? for one. Large and small letters are the same.")
                .changed()
                .then(|| Value::String(text))
        }
        Kind::Integer { min, max } => {
            let mut number = current.as_i64().unwrap_or(0);
            let changed = widgets::number_box(ui, &mut number, 1.0).changed();
            let number = number.clamp(min.unwrap_or(i64::MIN), max.unwrap_or(i64::MAX));
            changed.then(|| Value::from(number))
        }
        Kind::List(item) => match item.kind() {
            Kind::Text => {
                // (What is typed is kept as it is typed while the field
                // has the keyboard: a comma just written is not lost.)
                let id = ui.scope_id().with("a list as one text");
                let typed: Option<String> = ui.data(|data| data.get_temp(id));
                let mut text = typed.unwrap_or_else(|| words_of(current).join(", "));
                let response = widgets::text_field(ui, &mut text, 220.0)
                    .on_hover_text("Parted by commas: pdf, docx, txt");
                if response.changed() {
                    let new = listed(&text);
                    ui.data_mut(|data| data.insert_temp(id, text));
                    return Some(Value::from(new));
                }
                if !response.has_focus() {
                    ui.data_mut(|data| data.remove::<String>(id));
                }
                None
            }
            Kind::Choice(words) => {
                let mut chosen = words_of(current);
                let mut changed = false;
                for word in words {
                    let mut on = chosen.iter().any(|known| known == word.value);
                    if widgets::checkbox(ui, &mut on, title(word.value)).changed() {
                        changed = true;
                        if on {
                            chosen.push(word.value.to_string());
                        } else {
                            chosen.retain(|known| known != word.value);
                        }
                    }
                }
                changed.then(|| Value::from(chosen))
            }
            _ => None,
        },
        _ => None,
    }
}

/// What an action is called where one is chosen.
fn action_called(name: &str) -> String {
    match name {
        "assign_to_zone" => "is shown in a zone".to_string(),
        "move_to_folder" => "is moved to a folder".to_string(),
        other => title(other).to_lowercase(),
    }
}

/// What is done with an item: which of the things that can be done, and
/// the zone or the folder it is done with.
fn action_editor(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    action: &Value,
    zones: &[Value],
    highlight: Option<&str>,
) {
    let kinds = variants_at(pointer);
    let chosen = kinds.iter().find(|kind| kind.matches(action));
    let name = action["action"].as_str().unwrap_or("").to_string();
    ui.push_id(pointer, |ui| {
        ui.horizontal_wrapped(|ui| {
            let called: Vec<String> = kinds.iter().map(|kind| action_called(kind.name)).collect();
            let options: Vec<(String, &str)> = kinds
                .iter()
                .zip(&called)
                .map(|(kind, called)| (kind.name.to_string(), called.as_str()))
                .collect();
            let picked = widgets::segmented(
                ui,
                ui.scope_id().with("what is done"),
                Some(&name),
                &options,
                false,
            );
            if let Some(kind) =
                picked.and_then(|picked| kinds.iter().find(|kind| kind.name == picked))
            {
                let mut new = kind.blank();
                if kind.name == "assign_to_zone" {
                    new["zone"] = Value::from(first_id(zones));
                }
                document.set(pointer, new);
                return;
            }
            for field in chosen.map(fields_of).unwrap_or_default() {
                let place = format!("{pointer}/{}", field.name);
                match field.name {
                    "zone" => {
                        let id = action["zone"].as_str().unwrap_or("").to_string();
                        if let Some(picked) = zone_picker(ui, zones, &id) {
                            document.set(&place, Value::String(picked));
                        }
                    }
                    "folder" => {
                        let mut folder = action["folder"].as_str().unwrap_or("").to_string();
                        let home = dirs::home_dir();
                        if widgets::path_field(
                            ui,
                            &mut folder,
                            widgets::PathKind::Folder,
                            home.as_deref(),
                            240.0,
                        )
                        .changed()
                        {
                            document.set(&place, Value::String(folder));
                        }
                    }
                    _ => {}
                }
            }
        });
        // What an action has that this page does not know by name.
        let mut form = Form::new(document);
        form.highlight = highlight;
        for field in chosen.map(fields_of).unwrap_or_default() {
            if !matches!(field.name, "zone" | "folder") {
                form.place(
                    ui,
                    &format!("{pointer}/{}", field.name),
                    field.node,
                    &title(field.name),
                    1,
                );
            }
        }
    });
    if action["action"] == "move_to_folder" {
        let palette = theme::pal(ui.ctx());
        ui.label(
            RichText::new(
                "This moves the file itself, not only where it is shown. It is done when the \
                 rules are applied by hand, never by itself, and can be taken back.",
            )
            .size(12.0)
            .color(palette.muted),
        );
    }
}

/// The zone an item goes to: the one that is chosen, the others on a
/// press. An id none of them has is shown as it is, and said to be none.
fn zone_picker(ui: &mut Ui, zones: &[Value], id: &str) -> Option<String> {
    let mut options: Vec<(String, String, String)> = zones
        .iter()
        .filter_map(|zone| zone["id"].as_str())
        .filter(|known| !known.trim().is_empty())
        .map(|known| {
            let called = zone_called(zones, known);
            let shown = if called == known {
                called
            } else {
                format!("{called}  ({known})")
            };
            (known.to_string(), shown, String::new())
        })
        .collect();
    if !options.iter().any(|(known, _, _)| known == id) {
        let shown = if id.trim().is_empty() {
            "(none chosen)".to_string()
        } else {
            format!("{id}  (no such zone)")
        };
        options.push((
            id.to_string(),
            shown,
            "No zone of the configuration has this name.".to_string(),
        ));
    }
    widgets::dropdown(ui, "the zone it goes to", &id.to_string(), &options, 220.0)
}

/// Whether a field of a look is a colour, by what it is called (the
/// model writes a colour as a text).
fn is_colour(name: &str, kind: &Kind<'_>) -> bool {
    let text = match kind {
        Kind::Text => true,
        Kind::Optional(inner) => matches!(inner.kind(), Kind::Text),
        _ => false,
    };
    text && (name == "background"
        || name.ends_with("tint")
        || name.ends_with("colour")
        || name.ends_with("color"))
}

/// The fields of a look (every zone's, or one zone's own): a colour is
/// chosen as a colour, the rest is the form's.
fn look_fields(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    highlight: Option<&str>,
    palette: &Palette,
) {
    let Some(node) = gevrek_schema::schema().at(pointer) else {
        return;
    };
    let fields = match node.kind() {
        Kind::Object(fields) => fields,
        _ => return,
    };
    for field in fields {
        let place = format!("{pointer}/{}", field.name);
        if !is_colour(field.name, &field.node.kind()) {
            let mut form = Form::new(document);
            form.highlight = highlight;
            form.place(ui, &place, field.node, &title(field.name), 1);
            continue;
        }
        colour_row(
            ui, document, &place, field.node, field.name, highlight, palette,
        );
    }
}

fn colour_row(
    ui: &mut Ui,
    document: &mut Document,
    place: &str,
    node: Node<'_>,
    name: &str,
    highlight: Option<&str>,
    palette: &Palette,
) {
    let current = document
        .get(place)
        .and_then(Value::as_str)
        .map(str::to_string);
    // (What the theme gives where nothing is set: shown in the swatch.)
    let fallback = match name {
        "background" => palette.surface,
        // (A tint that is not set lays nothing over the icons.)
        name if name.ends_with("tint") => egui::Color32::TRANSPARENT,
        _ => palette.text,
    };
    let mut new = None;
    let ((), reset) = widgets::row(
        ui,
        &title(name),
        node.description(),
        current.is_some(),
        highlight == Some(place),
        |ui| {
            ui.push_id(place, |ui| {
                new = widgets::color_field_or(
                    ui,
                    ui.scope_id().with("colour"),
                    current.as_deref(),
                    Some(fallback),
                );
                if current.is_none() {
                    ui.weak("from the theme");
                }
            });
        },
    );
    if reset {
        document.set(place, Value::Null);
    } else if let Some(new) = new {
        // (Zones take a colour as #rrggbb or #rrggbbaa and nothing else:
        // one that is typed by its name is written so.)
        let written = gevrek_ui::color::parse(&new).map_or(new, gevrek_ui::color::format);
        document.set(place, Value::String(written));
    }
}

/// A zone drawn small with a look, on something that shows through it.
fn look_preview(ui: &mut Ui, look: &ZoneLook, title: &str, palette: &Palette) {
    let width = ui.available_width().clamp(200.0, 420.0);
    let (ground, _) = ui.allocate_exact_size(egui::vec2(width, 150.0), egui::Sense::hover());
    let painter = ui.painter_at(ground);
    // A desktop behind it: a ground and two shapes, so that what shows
    // through the zone is seen to.
    painter.rect_filled(ground, 8.0, theme::mix(palette.bg, palette.accent, 0.28));
    painter.circle_filled(
        ground.left_top() + egui::vec2(width * 0.22, 40.0),
        62.0,
        palette.purple.gamma_multiply(0.55),
    );
    painter.circle_filled(
        ground.right_bottom() - egui::vec2(width * 0.2, 30.0),
        70.0,
        palette.accent.gamma_multiply(0.6),
    );
    let zone = ground.shrink2(egui::vec2(28.0, 20.0));
    let radius = look
        .corner_radius
        .map_or(12.0, |radius| radius as f32)
        .min(zone.height() / 2.0);
    let fill = look
        .background
        .as_deref()
        .and_then(gevrek_ui::color::parse)
        .unwrap_or(palette.surface);
    let opacity = f32::from(look.opacity.unwrap_or(70).min(100)) / 100.0;
    painter.rect_filled(zone, radius, fill.gamma_multiply(opacity));
    painter.rect_stroke(
        zone,
        radius,
        egui::Stroke::new(1.0, palette.border.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    let ink = look
        .title_colour
        .as_deref()
        .and_then(gevrek_ui::color::parse)
        .unwrap_or(palette.text);
    // (A title shown only under the pointer: drawn faintly here.)
    let ink = if look.title_on_hover.unwrap_or(false) {
        ink.gamma_multiply(0.35)
    } else {
        ink
    };
    let head = egui::pos2(zone.left() + 12.0, zone.top() + 16.0);
    painter.rect_filled(
        egui::Rect::from_center_size(head, egui::vec2(3.0, 12.0)),
        1.5,
        palette.accent,
    );
    let shown = if title.trim().is_empty() {
        "A zone"
    } else {
        title.trim()
    };
    painter.text(
        head + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        widgets::ellipsize(shown, zone.width() - 40.0),
        egui::FontId::proportional(13.0),
        ink,
    );
    // A few items: a picture and a line where the name would be.
    for item in 0..4 {
        let left = zone.left() + 18.0 + item as f32 * 62.0;
        if left + 44.0 > zone.right() {
            break;
        }
        let icon =
            egui::Rect::from_min_size(egui::pos2(left, zone.top() + 36.0), egui::vec2(34.0, 34.0));
        painter.rect_filled(
            icon,
            7.0,
            theme::mix(palette.control, palette.accent, 0.18 + 0.12 * item as f32),
        );
        let name = egui::Rect::from_min_size(
            egui::pos2(left - 3.0, icon.bottom() + 7.0),
            egui::vec2(40.0, 5.0),
        );
        painter.rect_filled(name, 2.5, palette.text.gamma_multiply(0.55));
    }
}

/// What came of asking, where it was asked: said in a line, or shown as
/// the table of a plan.
fn outcome_said(ui: &mut Ui, outcome: &Outcome, palette: &Palette) {
    match outcome {
        Outcome::NotRunning(text) => {
            widgets::banner(ui, palette.muted, icons::INFO, |ui| {
                ui.label(text);
            });
        }
        Outcome::Said { bad, text } => {
            let (colour, icon) = if *bad {
                (palette.error, icons::X_CIRCLE)
            } else {
                (palette.ok, icons::CHECK_CIRCLE)
            };
            widgets::banner(ui, colour, icon, |ui| {
                ui.label(text);
            });
        }
        Outcome::Plan { table, done, .. } => plan_table(ui, table, *done, palette),
        Outcome::ByType { table, .. } => types_table(ui, table, palette),
    }
}

/// The zones that would be made by type, as a table; then what is left
/// where it is.
fn types_table(ui: &mut Ui, table: &ByType, palette: &Palette) {
    let quiet = table.rows.is_empty();
    let colour = if quiet { palette.muted } else { palette.accent };
    widgets::banner(ui, colour, icons::INFO, |ui| {
        ui.label(by_type_summed(table));
    });
    if !quiet {
        ui.add_space(8.0);
        let on = if table.display.is_empty() || !table.rows.iter().any(|row| row.new) {
            "Would be made".to_string()
        } else {
            format!("Would be made on {}", table.display)
        };
        widgets::caption(ui, &on);
        egui::ScrollArea::vertical()
            .id_salt("zones by type")
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("zones by type")
                    .num_columns(4)
                    .striped(true)
                    .spacing(egui::vec2(18.0, 6.0))
                    .show(ui, |ui| {
                        for head in ["Zone", "Items", "Where", "For what comes later"] {
                            ui.label(RichText::new(head).size(12.0).strong().color(palette.muted));
                        }
                        ui.end_row();
                        for row in &table.rows {
                            ui.label(
                                RichText::new(widgets::ellipsize(&row.title, 300.0))
                                    .color(palette.text),
                            )
                            .on_hover_text(&row.title);
                            ui.label(counted(row.items, "item"));
                            if row.new {
                                ui.label(format!("new, {}", row.place));
                            } else {
                                ui.label(RichText::new("is there already").color(palette.muted));
                            }
                            ui.label(
                                RichText::new(if row.rule { "a rule" } else { "" })
                                    .color(palette.muted),
                            );
                            ui.end_row();
                        }
                    });
            });
    }
    if !table.left.is_empty() {
        const NAMED: usize = 12;
        let mut names: Vec<&str> = table.left.iter().take(NAMED).map(String::as_str).collect();
        let more = format!("and {} more", table.left.len().saturating_sub(NAMED));
        if table.left.len() > NAMED {
            names.push(&more);
        }
        ui.add_space(8.0);
        widgets::caption(ui, "Left where they are");
        ui.label(RichText::new(names.join(", ")).color(palette.muted));
        ui.label(
            RichText::new(
                "Too few of a type for a zone of its own, or the zone of their type is locked.",
            )
            .size(12.0)
            .color(palette.faint),
        );
    }
}

/// The plan as a table: what would be done, then what is left alone.
fn plan_table(ui: &mut Ui, table: &Table, done: bool, palette: &Palette) {
    let quiet = table.steps.is_empty();
    let (colour, icon) = if quiet {
        (palette.muted, icons::INFO)
    } else if done {
        (palette.ok, icons::CHECK_CIRCLE)
    } else {
        (palette.accent, icons::INFO)
    };
    widgets::banner(ui, colour, icon, |ui| {
        ui.label(summed(table, done));
    });
    let part = |ui: &mut Ui, name: &str, heads: [&str; 3], rows: &[Row]| {
        if rows.is_empty() {
            return;
        }
        ui.add_space(8.0);
        widgets::caption(ui, name);
        egui::ScrollArea::vertical()
            .id_salt(name)
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(name)
                    .num_columns(3)
                    .striped(true)
                    .spacing(egui::vec2(18.0, 6.0))
                    .show(ui, |ui| {
                        for head in heads {
                            ui.label(RichText::new(head).size(12.0).strong().color(palette.muted));
                        }
                        ui.end_row();
                        for row in rows {
                            ui.label(
                                RichText::new(widgets::ellipsize(&row.item, 300.0))
                                    .color(palette.text),
                            )
                            .on_hover_text(&row.item);
                            ui.label(&row.does);
                            ui.label(RichText::new(&row.by).color(palette.muted));
                            ui.end_row();
                        }
                    });
            });
    };
    let does = if done {
        "What was done"
    } else {
        "What would happen"
    };
    part(
        ui,
        if done { "Sorted" } else { "Would be sorted" },
        ["Item", does, "Decided by"],
        &table.steps,
    );
    part(ui, "Left alone", ["Item", "Why", "Matched by"], &table.held);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::Config;

    #[test]
    fn an_entry_is_taken_out_and_moved_and_what_is_open_stays_with_it() {
        let after = |change: Change| {
            let mut list = vec!['a', 'b', 'c', 'd'];
            let did = changed(&mut list, change);
            (list.into_iter().collect::<String>(), did)
        };
        assert_eq!(after(Change::Remove(1)), ("acd".to_string(), true));
        assert_eq!(after(Change::Remove(9)), ("abcd".to_string(), false));
        assert_eq!(after(Change::up(2).unwrap()), ("acbd".to_string(), true));
        assert_eq!(
            after(Change::down(2, 4).unwrap()),
            ("abdc".to_string(), true)
        );
        // The first has nowhere to go up to, the last nowhere down.
        assert_eq!(Change::up(0), None);
        assert_eq!(Change::down(3, 4), None);

        // What is open follows its entry, and closes with it.
        assert_eq!(follows(Some(2), 4, Change::up(2).unwrap()), Some(1));
        assert_eq!(follows(Some(1), 4, Change::up(2).unwrap()), Some(2));
        assert_eq!(follows(Some(3), 4, Change::Remove(1)), Some(2));
        assert_eq!(follows(Some(1), 4, Change::Remove(1)), None);
        assert_eq!(
            follows(Some(0), 4, Change::Move { from: 0, to: 4 }),
            Some(3)
        );
        assert_eq!(follows(None, 4, Change::Remove(1)), None);
    }

    #[test]
    fn a_zone_that_is_added_has_a_name_of_its_own_and_is_one_the_model_reads() {
        let zones = vec![
            json!({"id": "zone-1"}),
            json!({"id": "work"}),
            json!({"id": "zone-3"}),
        ];
        assert_eq!(free_id(&zones), "zone-2");
        let zone = another_zone(&zones);
        assert_eq!(zone["id"], "zone-2");
        assert_eq!(zone["title"], "Zone 2");
        let read: gevrek_model::zones::ZoneDefinition =
            serde_json::from_value(zone).expect("a zone");
        assert_eq!(read.kind, gevrek_model::zones::ZoneKind::Manual);
    }

    #[test]
    fn the_rules_follow_a_zone_that_is_given_another_name() {
        let assign = |zone: &str| json!({"action": "assign_to_zone", "zone": zone});
        let mut section = json!({
            "zones": [{"id": "work"}, {"id": "play"}, {"id": "twice"}, {"id": "twice"}],
            "rules": [
                {"action": assign("work")},
                {"action": assign("play")},
                {"action": {"action": "move_to_folder", "folder": "work"}},
                {"action": assign("twice")},
            ],
            "catch_all": assign("work"),
        });
        rename_zone(&mut section, 0, "office");
        assert_eq!(section["zones"][0]["id"], "office");
        assert_eq!(section["rules"][0]["action"]["zone"], "office");
        assert_eq!(section["rules"][1]["action"]["zone"], "play");
        // (A folder that happens to be called the same is no zone.)
        assert_eq!(section["rules"][2]["action"]["folder"], "work");
        assert_eq!(section["catch_all"]["zone"], "office");
        // Of two zones with one id nobody can say which a rule meant.
        rename_zone(&mut section, 2, "third");
        assert_eq!(section["zones"][2]["id"], "third");
        assert_eq!(section["rules"][3]["action"]["zone"], "twice");
        // No such zone: nothing.
        let before = section.clone();
        rename_zone(&mut section, 9, "nowhere");
        assert_eq!(section, before);
    }

    #[test]
    fn a_zones_line_says_only_what_is_other_than_usual() {
        assert_eq!(
            zone_said(&json!({"kind": {"type": "manual"}, "view": "grid"})),
            ""
        );
        let zone = json!({
            "kind": {"type": "portal", "directory": "/home/ada/Downloads"},
            "view": "list",
            "rolled_up": true,
            "lock": {"geometry": true, "members": true},
            "tab_group": "left",
        });
        assert_eq!(
            zone_said(&zone),
            "/home/ada/Downloads  ·  as a list  ·  rolled up  ·  locked  ·  a tab of \u{201c}left\u{201d}"
        );
        let smart = json!({"kind": {"type": "smart", "query": [{}]}, "lock": {"members": true}, "always_visible": true});
        assert_eq!(
            zone_said(&smart),
            "1 condition  ·  always visible  ·  items locked"
        );
        assert_eq!(
            zone_said(&json!({"kind": {"type": "portal", "directory": " "}})),
            "no folder yet"
        );
    }

    #[test]
    fn extensions_are_typed_as_one_text() {
        assert_eq!(
            listed("pdf, docx;.txt  *.md PDF"),
            ["pdf", "docx", "txt", "md"]
        );
        assert_eq!(listed(" , "), Vec::<String>::new());
        // While it is typed: a comma just written adds nothing yet.
        assert_eq!(listed("pdf,"), ["pdf"]);
    }

    #[test]
    fn a_rule_reads_as_a_sentence() {
        let zones = vec![
            json!({"id": "docs", "title": "Documents"}),
            json!({"id": "bare"}),
        ];
        let rule = json!({
            "matchers": [
                {"match": "extension", "any_of": ["pdf", "docx"]},
                {"match": "untouched", "days": 30},
            ],
            "action": {"action": "assign_to_zone", "zone": "docs"},
        });
        assert_eq!(
            rule_said(&rule, &zones),
            format!("extension is pdf, docx and untouched for 30 days  {THEN}  into \u{201c}Documents\u{201d}")
        );
        let moves =
            json!({"matchers": [], "action": {"action": "move_to_folder", "folder": "/tmp/old"}});
        assert_eq!(
            rule_said(&moves, &zones),
            format!("matches nothing yet  {THEN}  the file is moved to /tmp/old")
        );
        // A zone without a title, a zone that is not there, none at all.
        let into =
            |zone: &str| action_said(&json!({"action": "assign_to_zone", "zone": zone}), &zones);
        assert_eq!(into("bare"), "into \u{201c}bare\u{201d}");
        assert_eq!(into("gone"), "into \u{201c}gone\u{201d}");
        assert_eq!(into(""), "into a zone (none chosen)");

        let said = |matcher: Value| matcher_said(&matcher);
        assert_eq!(
            said(json!({"match": "mime", "pattern": "image/*"})),
            "type fits image/*"
        );
        assert_eq!(
            said(json!({"match": "name", "pattern": ""})),
            "name fits (nothing yet)"
        );
        assert_eq!(
            said(json!({"match": "kind", "any_of": ["folder", "shortcut"]})),
            "is a folder or a shortcut"
        );
        assert_eq!(
            said(json!({"match": "untouched", "days": 1})),
            "untouched for 1 day"
        );
        assert_eq!(said(json!({"match": "size"})), "any size");
        assert!(said(json!({"match": "size", "at_least": 1048576})).starts_with("at least "));
        assert_eq!(
            said(json!({"match": "date", "field": "created", "newer_than_days": 7})),
            "created in the last 7 days"
        );
        assert_eq!(
            said(json!({"match": "date", "field": "modified", "older_than_days": 1})),
            "modified more than 1 day ago"
        );
        // One a newer model has: by its own name.
        assert_eq!(
            said(json!({"match": "from_the_future", "x": 1})),
            "from the future"
        );
    }

    #[test]
    fn a_plan_is_a_table_with_the_rules_and_the_zones_by_their_names() {
        let rules = vec![json!({"name": "Papers"}), json!({"name": " "})];
        let zones = vec![json!({"id": "docs", "title": "Documents"})];
        let plan = json!({
            "steps": [
                {"item": "/home/ada/Desktop/report.pdf", "decided": {"rule": 0},
                 "action": {"action": "assign_to_zone", "zone": "docs"}},
                {"item": "C:\\Users\\ada\\Desktop\\old.txt", "decided": {"rule": 1},
                 "action": {"action": "move_to_folder", "folder": "C:\\Old"}},
                {"item": "x", "name": "A name told with it", "decided": "catch_all",
                 "action": {"action": "assign_to_zone", "zone": "docs"}},
            ],
            "held": [
                {"item": "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "decided": {"rule": 1}, "reason": "not_a_file"},
                {"item": "/home/ada/Desktop/a.lnk", "decided": {"rule": 0}, "reason": "locked"},
                {"item": "/home/ada/Desktop/b", "decided": {"rule": 7}, "reason": "something_new"},
            ],
        });
        let table = table(&plan, &rules, &zones, &Default::default());
        let row = |item: &str, does: &str, by: &str| Row {
            item: item.to_string(),
            does: does.to_string(),
            by: by.to_string(),
        };
        assert_eq!(
            table.steps,
            [
                row(
                    "report.pdf",
                    "into \u{201c}Documents\u{201d}",
                    "rule 1, \u{201c}Papers\u{201d}"
                ),
                row("old.txt", "the file is moved to C:\\Old", "rule 2"),
                row(
                    "A name told with it",
                    "into \u{201c}Documents\u{201d}",
                    "no rule matched: what is done with the rest"
                ),
            ]
        );
        assert_eq!(
            table.held,
            [
                row(
                    "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
                    "it is no file that could be moved",
                    "rule 2"
                ),
                row(
                    "a.lnk",
                    "the zone is locked",
                    "rule 1, \u{201c}Papers\u{201d}"
                ),
                row("b", "something new", "rule 8"),
            ]
        );
        assert_eq!(
            summed(&table, false),
            "3 items would be sorted. 3 items a rule matched are left alone."
        );
        assert_eq!(
            summed(&table, true),
            "3 items were sorted. 3 items a rule matched are left alone."
        );
        // Nothing in it, or nothing that can be read: an empty table.
        let empty = super::table(
            &json!({"steps": [], "held": []}),
            &rules,
            &zones,
            &Default::default(),
        );
        assert_eq!(empty, Table::default());
        assert_eq!(
            super::table(&Value::Null, &rules, &zones, &Default::default()),
            Table::default()
        );
        assert_eq!(
            summed(&empty, false),
            "Nothing would be moved: every item is where its rule wants it."
        );
        let one = Table {
            steps: vec![row("a", "b", "c")],
            held: vec![row("d", "e", "f")],
        };
        assert_eq!(
            summed(&one, false),
            "1 item would be sorted. 1 item a rule matched is left alone."
        );
    }

    #[test]
    fn an_item_is_called_by_its_files_name() {
        assert_eq!(item_name("/home/ada/Desktop/report.pdf"), "report.pdf");
        assert_eq!(
            item_name("C:\\Users\\ada\\Desktop\\Old stuff\\"),
            "Old stuff"
        );
        assert_eq!(item_name("file:///home/ada/Desktop/a%20b.txt"), "a%20b.txt");
        assert_eq!(item_name("plain"), "plain");
        assert_eq!(
            item_name("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            "::{645FF040-5081-101B-9F08-00AA002F954E}"
        );
    }

    #[test]
    fn that_no_zones_run_is_said_plainly() {
        let not_running = refusal(
            "Zones are not running: no gevrek-shell that runs them is attached to this window manager",
            false,
        );
        assert!(
            matches!(&not_running, Outcome::NotRunning(text) if text.starts_with("Zones are not running"))
        );
        assert!(matches!(
            refusal("zones are switched off (desktop.zones.enabled)", true),
            Outcome::NotRunning(text) if text.starts_with("Zones are switched off")
        ));
        assert!(matches!(
            refusal("status: Unimplemented, message: \"\"", false),
            Outcome::NotRunning(text) if text.contains("older")
        ));
        // Not delivered for a reason nobody foresaw: said with the reason.
        assert_eq!(
            refusal("no answer", false),
            Outcome::NotRunning("The zones could not be asked: no answer".to_string())
        );
        // Anything else is the zones' own word.
        assert_eq!(
            refusal("no zone is called nowhere", true),
            Outcome::Said {
                bad: true,
                text: "no zone is called nowhere".to_string()
            }
        );
    }

    #[test]
    fn the_zones_that_run_are_known_by_what_the_window_manager_says() {
        let told = json!({
            "version": 1, "enabled": true,
            "zones": [
                {"id": "work", "items": [{"id": "/d/a.pdf", "name": "a.pdf"}]},
                {"id": "empty", "items": []},
            ],
            "loose": [{"id": "::{BIN}", "name": "Recycle Bin"}, {"id": "/d/b.txt", "name": "b.txt"}],
        })
        .to_string();
        let running = Running::of(true, false, &told, "");
        assert!(running.can_be_asked());
        assert_eq!((running.zones, running.loose), (2, 2));
        assert_eq!(
            running.names.get("::{BIN}").map(String::as_str),
            Some("Recycle Bin")
        );
        assert_eq!(
            running.said(),
            (
                false,
                "The zones that run: 2 zones, 2 items in none.".to_string()
            )
        );
        // Switched on and not shown: said with the reason, and still asked.
        let hindered =
            json!({"enabled": true, "not_shown": "Not shown: the journal could not be written"});
        let running = Running::of(true, false, &hindered.to_string(), "");
        assert!(running.can_be_asked());
        assert!(running
            .said()
            .1
            .ends_with("Not shown: the journal could not be written"));
        // Switched off: nobody to ask about rules.
        let running = Running::of(true, false, r#"{"enabled": false}"#, "");
        assert!(!running.can_be_asked());
        assert!(running.said().0 && running.said().1.starts_with("Zones are switched off"));
        // No shell: the window manager's own words. What a shell that
        // has gone last told is nobody's state.
        let gone = Running::of(
            false,
            true,
            &told,
            "Zones are not running: no gevrek-shell is attached",
        );
        assert!(!gone.can_be_asked() && gone.names.is_empty());
        assert_eq!(
            gone.said(),
            (
                true,
                "Zones are not running: no gevrek-shell is attached".to_string()
            )
        );
        assert!(Running::of(false, false, "", "")
            .said()
            .1
            .starts_with("Zones are not running"));
    }

    #[test]
    fn a_dry_run_is_asked_answered_and_shown() {
        let document = Document::new(&Config::default());
        let mut page = ZonesPage::default();
        page.state(
            true,
            false,
            r#"{"enabled": true, "loose": [{"id": "shell:42", "name": "Holiday.lnk"}]}"#,
            "",
        );
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules", "scope": "loose"}),
            &mut wishes,
        );
        let [Wish::Ask(command)] = wishes.as_slice() else {
            panic!("one command is asked: {wishes:?}");
        };
        let sent: Value = serde_json::from_str(command).unwrap();
        assert_eq!(sent, json!({"command": "preview_rules", "scope": "loose"}));
        // An answer to what another page asked is not this page's.
        assert!(!page.answered(
            r#"{"command":"quick_hide"}"#,
            true,
            true,
            None,
            &[],
            &document
        ));
        assert!(page.asked.is_some() && page.outcome.is_none());
        // The answer, with the plan among what happened.
        let events = vec![json!({"event": "rules_previewed", "plan": {"steps": [
            {"item": "/d/a.pdf", "decided": {"rule": 0}, "action": {"action": "assign_to_zone", "zone": "docs"}},
            {"item": "shell:42", "decided": "catch_all", "action": {"action": "assign_to_zone", "zone": "docs"}},
        ], "held": []}})
        .to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert!(page.asked.is_none());
        let Some(Outcome::Plan {
            table,
            done: false,
            every_item: false,
        }) = &page.outcome
        else {
            panic!("a plan is shown: {:?}", page.outcome);
        };
        // An item is called what the zones call it, its file's name else.
        let items: Vec<&str> = table.steps.iter().map(|row| row.item.as_str()).collect();
        assert_eq!(items, ["a.pdf", "Holiday.lnk"]);
        assert!(
            !page.answered(command, true, true, None, &events, &document),
            "answered once"
        );

        // Carried out: the same table, as what was done.
        let mut wishes = Vec::new();
        page.ask(
            Ask::Apply,
            json!({"command": "apply_rules", "scope": "loose"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events =
            vec![json!({"event": "rules_applied", "plan": {"steps": [], "held": []}}).to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert!(matches!(
            &page.outcome,
            Some(Outcome::Plan { done: true, .. })
        ));
        // An answer without the plan says so, and shows no empty table.
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        assert!(page.answered(command, true, true, None, &[], &document));
        assert!(matches!(
            &page.outcome,
            Some(Outcome::Said { bad: true, .. })
        ));
        // Taken back.
        let mut wishes = Vec::new();
        page.ask(Ask::Undo, json!({"command": "undo_rules"}), &mut wishes);
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        assert!(page.answered(
            command,
            true,
            true,
            None,
            &[r#"{"event":"rules_undone"}"#.to_string()],
            &document
        ));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: false,
                text: "The last sorting was taken back.".to_string()
            })
        );
    }

    #[test]
    fn asking_zones_that_do_not_run_says_so() {
        let document = Document::new(&Config::default());
        let mut page = ZonesPage::default();
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        assert!(page.answered(
            command,
            false,
            false,
            Some("Zones are not running: no gevrek-shell that runs them is attached to this window manager"),
            &[],
            &document,
        ));
        assert!(page.asked.is_none());
        assert!(matches!(page.outcome, Some(Outcome::NotRunning(_))));
        // Refused by the zones themselves: their reason.
        let mut wishes = Vec::new();
        page.ask(Ask::Undo, json!({"command": "undo_rules"}), &mut wishes);
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        assert!(page.answered(
            command,
            false,
            true,
            Some("nothing to take back"),
            &[],
            &document
        ));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: true,
                text: "nothing to take back".to_string()
            })
        );
    }

    #[test]
    fn the_page_asks_how_the_zones_are_while_a_window_manager_is_there() {
        let context = egui::Context::default();
        let mut page = ZonesPage::default();
        let around = |connected: bool| Around {
            connected,
            dirty: false,
            windows: false,
            highlight: None,
        };
        let mut wishes = Vec::new();
        page.follow(&context, &around(false), &mut wishes);
        assert!(wishes.is_empty(), "nobody to ask");
        page.follow(&context, &around(true), &mut wishes);
        page.follow(&context, &around(true), &mut wishes);
        assert_eq!(wishes, [Wish::AskState], "asked once, not with every frame");
        // Something happened to the zones: asked again.
        page.heard("window.opened");
        page.follow(&context, &around(true), &mut wishes);
        assert_eq!(wishes.len(), 1);
        page.heard("zones.items_changed");
        page.follow(&context, &around(true), &mut wishes);
        assert_eq!(wishes.len(), 2);
        // The window manager went: what it said is not known any more.
        page.state(true, false, r#"{"enabled": true}"#, "");
        page.follow(&context, &around(false), &mut wishes);
        assert!(page.running.is_none());
    }

    #[test]
    fn the_arrangements_that_are_kept_are_listed_as_the_zones_tell_them() {
        let told = json!({"enabled": true, "can_undo_rules": false, "snapshots": [
            {"name": "Work", "taken_at": 1_795_000_000u64, "zones": 4, "automatic": false},
            {"name": "Before import", "taken_at": 0, "zones": [{}, {}], "automatic": true},
            {"taken_at": 5},
        ]});
        let running = Running::of(true, false, &told.to_string(), "");
        let names: Vec<&str> = running.kept.iter().map(|kept| kept.name.as_str()).collect();
        assert_eq!(names, ["Work", "Before import"]);
        assert!(!running.can_undo);
        assert_eq!(
            running.kept[0].said(),
            "4 zones  \u{b7}  2026-11-18 11:06 UTC"
        );
        assert_eq!(
            running.kept[1].said(),
            "2 zones  \u{b7}  kept by the zones themselves"
        );
        // Zones that do not say whether there is something to take back
        // are asked, and answer for themselves.
        assert!(Running::of(true, false, r#"{"enabled": true}"#, "").can_undo);
    }

    #[test]
    fn keeping_writing_and_reading_arrangements_is_answered_where_it_was_asked() {
        let document = Document::new(&Config::default());
        let mut page = ZonesPage::default();
        let mut wishes = Vec::new();
        page.ask(
            Ask::Snapshot,
            json!({"command": "save_snapshot", "name": "work"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events = vec![r#"{"event":"snapshot_saved","name":"work"}"#.to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert!(page.below, "said in the arrangements' card");
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: false,
                text: "The arrangement \u{201c}work\u{201d} is kept.".to_string()
            })
        );

        // Written to a file: the document the zones gave, as it is.
        let folder =
            std::env::temp_dir().join(format!("gevrek-studio-zones-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("work.json").display().to_string();
        let exported =
            json!({"format": "gevrek-zones-layout", "version": 1, "layout": {"name": "Work"}});
        let mut wishes = Vec::new();
        page.ask(
            Ask::Export(file.clone()),
            json!({"command": "export_layout", "name": "Work"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events = vec![json!({"event": "layout_exported", "document": exported}).to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: false,
                text: format!("Written to {file}.")
            })
        );
        let read: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(read, exported);
        // Nowhere to write it: said, with the reason.
        let nowhere = folder
            .join("no such folder")
            .join("work.json")
            .display()
            .to_string();
        assert!(written(&exported, &nowhere).is_err());
        assert!(
            written(&Value::Null, &file).is_err(),
            "nothing is no document"
        );
        let _ = std::fs::remove_dir_all(&folder);

        // Read in.
        let applied = json!({"event": "layout_imported", "name": "Work (2)", "applied": true, "kept_as": "Before import"});
        assert_eq!(
            imported_said(&applied),
            "Read in as \u{201c}Work (2)\u{201d}, and the zones are as it says. \
             What there was is kept as \u{201c}Before import\u{201d}."
        );
        assert_eq!(
            imported_said(&json!({"name": "Work", "applied": false})),
            "Read in and kept as \u{201c}Work\u{201d}."
        );
        // A dry run is said with the rules again.
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules"}),
            &mut wishes,
        );
        assert!(!page.below && page.outcome.is_none());
    }

    #[test]
    fn the_zones_own_lines_of_a_plan_are_the_table() {
        let rules = vec![json!({"name": "Renamed here since"})];
        let lines = vec![
            json!({"item": "/desk/cat.png", "name": "cat.png", "from": "inbox",
                   "destination": {"to": "zone", "zone": "pictures", "title": "Pictures"},
                   "decided": {"rule": 0}, "rule": "Pictures"}),
            json!({"item": "/desk/b.tmp", "destination": {"to": "folder", "folder": "/home/ada/Archive"},
                   "decided": {"rule": 0}, "rule": ""}),
            json!({"item": "/desk/old.doc", "name": "old.doc",
                   "destination": {"to": "folder", "folder": "/home/ada/Archive"},
                   "decided": "catch_all", "rule": "", "held": "not_a_file"}),
        ];
        let table = table_of_lines(&lines, &rules);
        let row = |item: &str, does: &str, by: &str| Row {
            item: item.to_string(),
            does: does.to_string(),
            by: by.to_string(),
        };
        assert_eq!(
            table.steps,
            [
                // (The rule by the name the zones have for it.)
                row(
                    "cat.png",
                    "into \u{201c}Pictures\u{201d}",
                    "rule 1, \u{201c}Pictures\u{201d}"
                ),
                row(
                    "b.tmp",
                    "the file is moved to /home/ada/Archive",
                    "rule 1, \u{201c}Renamed here since\u{201d}"
                ),
            ]
        );
        assert_eq!(
            table.held,
            [row(
                "old.doc",
                "it is no file that could be moved",
                "no rule matched: what is done with the rest"
            )]
        );
        // An answer that has them is shown by them.
        let document = Document::new(&Config::default());
        let mut page = ZonesPage::default();
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events = vec![
            json!({"event": "rules_previewed", "plan": {"steps": [], "held": []}, "lines": lines})
                .to_string(),
        ];
        assert!(page.answered(command, true, true, None, &events, &document));
        let by_lines = table_of_lines(&lines, &[]);
        assert!(
            matches!(&page.outcome, Some(Outcome::Plan { table: shown, .. }) if *shown == by_lines)
        );
    }

    #[test]
    fn a_plan_of_zones_by_type_is_a_table_and_a_sentence() {
        let plan = json!({
            "display": "DEL-A1B2-77",
            "zones": [
                {"category": "folders", "title": "Folders", "rect": {"x": 8, "y": 48, "width": 172, "height": 126},
                 "items": ["/desk/Archive", "/desk/Projects"], "rule": true},
                {"category": "pictures", "title": "Photos", "zone": "zone-4",
                 "rect": {"x": 900, "y": 600, "width": 412, "height": 384}, "items": ["/desk/one.gif"], "rule": false},
            ],
            "left": ["/desk/song.mp3", "shell:42"],
        });
        let names = [("shell:42".to_string(), "Holiday.lnk".to_string())].into();
        let table = by_type(&plan, &names);
        let row = |title: &str, new: bool, items: usize, place: &str, rule: bool| TypeRow {
            title: title.to_string(),
            new,
            items,
            place: place.to_string(),
            rule,
        };
        assert_eq!(
            table.rows,
            [
                row("Folders", true, 2, "172 \u{d7} 126 at 8, 48", true),
                row("Photos", false, 1, "412 \u{d7} 384 at 900, 600", false),
            ]
        );
        assert_eq!(table.display, "DEL-A1B2-77");
        // An item is called what the zones call it, its file's name else.
        assert_eq!(table.left, ["song.mp3", "Holiday.lnk"]);
        assert_eq!(
            by_type_summed(&table),
            "1 zone would be made, and 3 items would be put into the zones of their types. \
             2 items are left where they are."
        );
        let only_old = ByType {
            rows: vec![row("Photos", false, 1, "", false)],
            ..ByType::default()
        };
        assert_eq!(
            by_type_summed(&only_old),
            "No zone would be made; 1 item would be put into its type\u{2019}s zone."
        );
        let nothing = by_type(
            &json!({"zones": [], "left": ["/desk/a.mp3"]}),
            &Default::default(),
        );
        assert!(by_type_summed(&nothing).starts_with("There is nothing to make"));
        assert!(by_type_summed(&nothing).ends_with("1 item is left where it is."));
        assert_eq!(
            by_type(&Value::Null, &Default::default()),
            ByType::default()
        );
        // What was made, and what was taken back.
        assert_eq!(
            made_said(
                &json!({"event": "auto_created", "zones": ["zone-4", "zone-5"], "items": 7, "rules": 2})
            ),
            "2 zones made, 7 items sorted. 2 rules added for what comes later."
        );
        assert_eq!(
            made_said(&json!({"event": "auto_created", "zones": [], "items": 1, "rules": 0})),
            "1 item sorted into the zones made by type before."
        );
        assert_eq!(
            made_said(&json!({"zones": [], "items": 0, "rules": 0})),
            "There was nothing to do."
        );
        assert_eq!(
            unmade_said(&json!({"event": "auto_undone", "zones": 3, "items": 1, "rules": 2})),
            "Taken back: 3 zones taken away, 1 item put back, 2 rules taken out."
        );
    }

    #[test]
    fn zones_by_type_are_asked_for_shown_made_and_taken_back() {
        let document = Document::new(&Config::default());
        let mut page = ZonesPage::default();
        page.state(
            true,
            false,
            r#"{"enabled": true, "loose": [{"id": "shell:42", "name": "Holiday.lnk"}]}"#,
            "",
        );
        assert!(
            !page.running.as_ref().unwrap().can_undo_types,
            "nothing was made yet"
        );
        let mut wishes = Vec::new();
        page.ask(
            Ask::TypesPreview,
            json!({"command": "auto_create", "scope": "loose", "dry_run": true}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let sent: Value = serde_json::from_str(command).unwrap();
        assert_eq!(
            sent,
            json!({"command": "auto_create", "scope": "loose", "dry_run": true})
        );
        let events = vec![json!({"event": "auto_planned", "plan": {
            "display": "fake-1",
            "zones": [{"category": "pictures", "title": "Pictures",
                       "rect": {"x": 8, "y": 8, "width": 172, "height": 126},
                       "items": ["/desk/a.png", "/desk/b.png"], "rule": true}],
            "left": ["shell:42"]}})
        .to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert!(
            page.of_types && !page.below,
            "said in the card of the zones by type"
        );
        let Some(Outcome::ByType {
            table,
            every_item: false,
        }) = &page.outcome
        else {
            panic!("the plan is shown: {:?}", page.outcome);
        };
        assert_eq!(
            (table.rows.len(), table.rows[0].items, table.rows[0].new),
            (1, 2, true)
        );
        assert_eq!(table.left, ["Holiday.lnk"]);

        // Made: what the zones say they made.
        let mut wishes = Vec::new();
        page.ask(
            Ask::TypesMake,
            json!({"command": "auto_create", "scope": "loose"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events = vec![
            r#"{"event":"zone_created","zone":"zone-4"}"#.to_string(),
            r#"{"event":"auto_created","zones":["zone-4"],"items":2,"rules":1}"#.to_string(),
        ];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: false,
                text: "1 zone made, 2 items sorted. 1 rule added for what comes later.".to_string()
            })
        );
        // The zones say there is something to take back now.
        page.state(
            true,
            false,
            r#"{"enabled": true, "can_undo_auto": true}"#,
            "",
        );
        assert!(page.running.as_ref().unwrap().can_undo_types);
        let mut wishes = Vec::new();
        page.ask(
            Ask::TypesUndo,
            json!({"command": "undo_auto_create"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let events = vec![r#"{"event":"auto_undone","zones":1,"items":2,"rules":1}"#.to_string()];
        assert!(page.answered(command, true, true, None, &events, &document));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: false,
                text: "Taken back: 1 zone taken away, 2 items put back, 1 rule taken out."
                    .to_string()
            })
        );
        // An answer without the plan says so; a refusal gives its reason.
        let mut wishes = Vec::new();
        page.ask(
            Ask::TypesPreview,
            json!({"command": "auto_create", "dry_run": true}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        assert!(page.answered(command, true, true, None, &[], &document));
        assert!(matches!(
            &page.outcome,
            Some(Outcome::Said { bad: true, .. })
        ));
        let mut wishes = Vec::new();
        page.ask(
            Ask::TypesMake,
            json!({"command": "auto_create"}),
            &mut wishes,
        );
        let Wish::Ask(command) = &wishes[0] else {
            panic!()
        };
        let locked = "every zone is locked: nothing goes in or out";
        assert!(page.answered(command, false, true, Some(locked), &[], &document));
        assert_eq!(
            page.outcome,
            Some(Outcome::Said {
                bad: true,
                text: locked.to_string()
            })
        );
        // Asking the rules afterwards is said with the rules again.
        let mut wishes = Vec::new();
        page.ask(
            Ask::Preview,
            json!({"command": "preview_rules"}),
            &mut wishes,
        );
        assert!(!page.of_types);
    }

    /// The plan of zones by type drawn into a picture, to be looked at
    /// (no window manager and no zones are needed for it):
    ///
    ///     GEVREK_LOOK_OUT=types.png cargo test -p gevrek-studio the_plan_of_zones_by_type_is_drawn
    ///
    /// Without `GEVREK_LOOK_OUT` it is drawn and thrown away, which every
    /// run of the tests does: the table draws whatever a plan holds.
    #[test]
    fn the_plan_of_zones_by_type_is_drawn() {
        let plan = json!({
            "display": "DEL-A1B2-77",
            "zones": [
                {"category": "folders", "title": "Folders", "rect": {"x": 8, "y": 48, "width": 172, "height": 126},
                 "items": ["/d/Archive", "/d/Projects"], "rule": true},
                {"category": "programs", "title": "Programs", "rect": {"x": 188, "y": 48, "width": 332, "height": 298},
                 "items": ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"], "rule": true},
                {"category": "documents", "title": "Documents", "rect": {"x": 528, "y": 48, "width": 252, "height": 212},
                 "items": ["1", "2", "3", "4", "5"], "rule": true},
                {"category": "pictures", "title": "Photos", "zone": "zone-4",
                 "rect": {"x": 900, "y": 600, "width": 412, "height": 384}, "items": ["/d/one.gif"], "rule": false},
                {"category": "other", "title": "Other", "rect": {"x": 788, "y": 48, "width": 172, "height": 212},
                 "items": ["1", "2", "3"], "rule": false},
            ],
            "left": ["/home/ada/Desktop/song.mp3", "/home/ada/Desktop/film.mkv"],
        });
        let mut page = ZonesPage {
            outcome: Some(Outcome::ByType {
                table: by_type(&plan, &Default::default()),
                every_item: false,
            }),
            of_types: true,
            running: Some(Running::of(
                true,
                false,
                r#"{"enabled": true, "zones": [{}], "loose": [{}, {}], "can_undo_auto": true}"#,
                "",
            )),
            // (Asked a moment ago: drawing asks nothing.)
            state_asked: Some(Instant::now()),
            ..ZonesPage::default()
        };
        let around = Around {
            connected: true,
            dirty: false,
            windows: false,
            highlight: None,
        };
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        gevrek_ui::theme::apply(&context, &palette, &settings);
        let mut draw = |ui: &mut Ui| {
            let palette = theme::pal(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                let mut wishes = Vec::new();
                widgets::card(ui, "Zones by type", |ui| {
                    page.types_card(ui, &around, &palette, &mut wishes);
                });
                assert!(wishes.is_empty(), "nothing is asked by drawing");
            });
        };
        let Some(out) = std::env::var_os("GEVREK_LOOK_OUT") else {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 760.0),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| draw(ui));
            output.textures_delta.clear();
            return;
        };
        let size = (1000, 640);
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let mut pixels = Vec::new();
        for step in 0..3 {
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.5, Vec::new(), |ui| {
                draw(ui)
            });
        }
        image::save_buffer(&out, &pixels, size.0, size.1, image::ColorType::Rgba8)
            .expect("the picture is written");
    }

    /// The answer to a dry run drawn into a picture, to be looked at (no
    /// window manager and no zones are needed for it):
    ///
    ///     GEVREK_LOOK_OUT=plan.png cargo test -p gevrek-studio the_answer_to_a_dry_run_is_drawn
    ///
    /// Without `GEVREK_LOOK_OUT` it is drawn and thrown away, which every
    /// run of the tests does: the table draws whatever a plan holds.
    #[test]
    fn the_answer_to_a_dry_run_is_drawn() {
        let rules = vec![json!({"name": "Papers"}), json!({"name": "Old installers"})];
        let zones = vec![json!({"id": "work", "title": "Work"})];
        let assign = json!({"action": "assign_to_zone", "zone": "work"});
        let plan = json!({
            "steps": [
                {"item": "/home/ada/Desktop/report.pdf", "decided": {"rule": 0}, "action": assign},
                {"item": "/home/ada/Desktop/letter to the landlord.docx", "decided": {"rule": 0}, "action": assign},
                {"item": "/home/ada/Desktop/setup-2019.exe", "decided": {"rule": 1},
                 "action": {"action": "move_to_folder", "folder": "/home/ada/Old"}},
                {"item": "/home/ada/Desktop/notes.txt", "decided": "catch_all", "action": assign},
            ],
            "held": [
                {"item": "/home/ada/Desktop/contract.pdf", "decided": {"rule": 0}, "reason": "locked"},
                {"item": "trash:///", "name": "Bin", "decided": {"rule": 1}, "reason": "not_a_file"},
            ],
        });
        let mut page = ZonesPage {
            outcome: Some(Outcome::Plan {
                table: table(&plan, &rules, &zones, &Default::default()),
                every_item: false,
                done: false,
            }),
            running: Some(Running::of(
                true,
                false,
                r#"{"enabled": true, "zones": [{}, {}, {}], "loose": [{}, {}], "snapshots": [
                    {"name": "Work", "taken_at": 1795000000, "zones": 3},
                    {"name": "Before restoring", "taken_at": 1795003600, "zones": 2, "automatic": true}]}"#,
                "",
            )),
            // (Asked a moment ago: drawing asks nothing.)
            state_asked: Some(Instant::now()),
            ..ZonesPage::default()
        };
        let around = Around {
            connected: true,
            dirty: true,
            windows: false,
            highlight: None,
        };
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        gevrek_ui::theme::apply(&context, &palette, &settings);
        let mut draw = |ui: &mut Ui| {
            let palette = theme::pal(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                let mut wishes = Vec::new();
                widgets::card(ui, "What the rules would do", |ui| {
                    page.dry_run_card(ui, &around, &palette, &mut wishes);
                });
                widgets::card(ui, "Arrangements kept by name", |ui| {
                    page.snapshots_card(ui, &around, &palette, &mut wishes);
                });
                assert!(wishes.is_empty(), "nothing is asked by drawing");
            });
        };
        let Some(out) = std::env::var_os("GEVREK_LOOK_OUT") else {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 760.0),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| draw(ui));
            output.textures_delta.clear();
            return;
        };
        let size = (1000, 1100);
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let mut pixels = Vec::new();
        for step in 0..3 {
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.5, Vec::new(), |ui| {
                draw(ui)
            });
        }
        image::save_buffer(&out, &pixels, size.0, size.1, image::ColorType::Rgba8)
            .expect("the picture is written");
    }

    #[test]
    fn the_checks_messages_are_said_where_they_belong() {
        let issues = vec![
            ConfigIssue::error("desktop.zones.zones[1].id", "A zone needs an id"),
            ConfigIssue::warning(
                "desktop.zones.rules[2].action.zone",
                "No zone is called 'gone'",
            ),
            ConfigIssue::warning(
                "desktop.zones.catch_all.zone",
                "No zone is called 'nowhere'",
            ),
            ConfigIssue::error("desktop.zones.rules_on_arrival", "not one of the rules"),
            ConfigIssue::error("desktop.focus.history_size", "elsewhere"),
        ];
        assert_eq!(
            issues_under(&issues, "desktop.zones.zones"),
            [(true, "Zone 2: A zone needs an id".to_string())]
        );
        assert_eq!(
            issues_under(&issues, "desktop.zones.rules"),
            [(false, "Rule 3: No zone is called 'gone'".to_string())]
        );
        assert_eq!(
            issues_under(&issues, "desktop.zones.catch_all"),
            [(
                false,
                "What no rule matches: No zone is called 'nowhere'".to_string()
            )]
        );
        assert_eq!(place_said("desktop.zones.look.opacity"), "");
    }

    #[test]
    fn what_the_page_writes_is_a_configuration_the_model_reads() {
        // The lists as the page changes them, in a document: a zone
        // added, renamed with a rule that follows, moved, taken out.
        let mut document = Document::new(&Config::default());
        for _ in 0..2 {
            let zone = another_zone(&list_at(&document, LIST));
            document.edit(LIST, |list| list.as_array_mut().unwrap().push(zone));
        }
        document.edit(RULES, |list| {
            list.as_array_mut()
                .unwrap()
                .push(json!({"name": "r", "matchers": [], "action": {"action": "assign_to_zone", "zone": "zone-2"}}));
        });
        document.edit(ZONES, |section| rename_zone(section, 1, "papers"));
        document.edit(LIST, |list| {
            changed(list.as_array_mut().unwrap(), Change::up(1).unwrap());
        });
        let config = document.config().expect("the model reads it");
        let zones = &config.desktop.zones;
        let ids: Vec<&str> = zones.zones.iter().map(|zone| zone.id.as_str()).collect();
        assert_eq!(ids, ["papers", "zone-1"]);
        assert_eq!(
            zones.rules[0].action,
            gevrek_model::zones::RuleAction::AssignToZone {
                zone: gevrek_model::zones::ZoneId::new("papers")
            }
        );
        assert!(document
            .issues()
            .iter()
            .all(|issue| !issue.path.starts_with("desktop.zones")));
        // Taken out: its rule is said to name nothing, and stops nothing.
        document.edit(LIST, |list| {
            changed(list.as_array_mut().unwrap(), Change::Remove(0));
        });
        assert_eq!(
            issues_under(document.issues(), "desktop.zones.rules"),
            [(false, "Rule 1: No zone is called 'papers'".to_string())]
        );
    }

    #[test]
    fn every_kind_of_zone_condition_and_action_the_schema_has_is_offered() {
        // The pickers are filled from the schema: what the model gains
        // is offered here. These are the ones this page has words for.
        let names = |pointer: &str| -> Vec<String> {
            variants_at(pointer)
                .iter()
                .map(|kind| kind.name.to_string())
                .collect()
        };
        let kinds = names("/desktop/zones/zones/0/kind");
        for known in ["manual", "portal", "smart"] {
            assert!(
                kinds.iter().any(|kind| kind == known),
                "{known} is no kind of zone: {kinds:?}"
            );
        }
        let matchers = names("/desktop/zones/rules/0/matchers/0");
        for known in [
            "extension",
            "mime",
            "name",
            "kind",
            "date",
            "size",
            "shortcut_target",
            "untouched",
        ] {
            assert!(
                matchers.iter().any(|kind| kind == known),
                "{known} is no condition: {matchers:?}"
            );
        }
        assert_eq!(names("/desktop/zones/zones/0/kind/query/0"), matchers);
        let actions = names("/desktop/zones/rules/0/action");
        assert!(
            actions.contains(&"assign_to_zone".to_string())
                && actions.contains(&"move_to_folder".to_string())
        );
        assert_eq!(names(CATCH_ALL), actions);
    }
}
