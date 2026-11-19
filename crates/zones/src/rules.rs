//! The rules at work. A plan says what the rules would do to the items as
//! they are now, and changes nothing; applying a plan does it and says
//! how to take it back; undoing takes it back. The time of the day is
//! given, never read, so the same items give the same plan in a test.
//!
//! Moving a file is not done here: it is returned as an operation for the
//! backend to carry out.

use crate::backend::FileOperation;
use crate::config::{DateField, Matcher, Rule, RuleAction, ZoneKind, ZonesConfig};
use crate::ids::{ItemId, ZoneId};
use crate::item::Item;
use crate::paths;
use crate::placement::{Placements, Whereabouts};
use serde::{Deserialize, Serialize};

const DAY: u64 = 24 * 60 * 60;

/// Whether `text` fits a glob: `*` stands for any run of letters, `?` for
/// one. Letters are compared without regard to case.
pub fn glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut at_pattern = 0;
    let mut at_text = 0;
    // Where the last star was, and how much of the text it has taken.
    let mut star: Option<(usize, usize)> = None;
    while at_text < text.len() {
        if at_pattern < pattern.len()
            && (pattern[at_pattern] == '?' || pattern[at_pattern] == text[at_text])
        {
            at_pattern += 1;
            at_text += 1;
        } else if at_pattern < pattern.len() && pattern[at_pattern] == '*' {
            star = Some((at_pattern, at_text));
            at_pattern += 1;
        } else if let Some((star_pattern, star_text)) = star {
            // The star takes one letter more.
            at_pattern = star_pattern + 1;
            at_text = star_text + 1;
            star = Some((star_pattern, star_text + 1));
        } else {
            return false;
        }
    }
    pattern[at_pattern..].iter().all(|letter| *letter == '*')
}

/// Whether one matcher holds for an item at the time `now` (seconds since
/// 1970). A matcher that asks for a fact the item does not have does not
/// hold.
pub fn holds(matcher: &Matcher, item: &Item, now: u64) -> bool {
    let facts = &item.facts;
    match matcher {
        Matcher::Extension { any_of } => facts.extension.as_ref().is_some_and(|extension| {
            any_of
                .iter()
                .any(|wanted| wanted.trim_start_matches('.').eq_ignore_ascii_case(extension))
        }),
        Matcher::Mime { pattern } => facts.mime.as_ref().is_some_and(|mime| glob(pattern, mime)),
        Matcher::Name { pattern } => glob(pattern, &item.name),
        Matcher::Kind { any_of } => any_of.contains(&item.kind),
        Matcher::Date { field, after, before, newer_than_days, older_than_days } => {
            let time = match field {
                DateField::Created => facts.created,
                DateField::Modified => facts.modified,
                DateField::Accessed => facts.accessed,
            };
            let Some(time) = time else {
                return false;
            };
            let days_ago = |days: u32| now.saturating_sub(u64::from(days) * DAY);
            after.is_none_or(|after| time > after)
                && before.is_none_or(|before| time < before)
                && newer_than_days.is_none_or(|days| time > days_ago(days))
                && older_than_days.is_none_or(|days| time <= days_ago(days))
        }
        Matcher::Size { at_least, at_most } => facts.size.is_some_and(|size| {
            at_least.is_none_or(|least| size >= least) && at_most.is_none_or(|most| size <= most)
        }),
        Matcher::ShortcutTarget { pattern } => facts
            .shortcut_target
            .as_ref()
            .is_some_and(|target| glob(pattern, target)),
        Matcher::Untouched { days } => facts
            .last_touched()
            .is_some_and(|touched| touched <= now.saturating_sub(u64::from(*days) * DAY)),
    }
}

/// Whether a rule matches an item: it is enabled, asks for something, and
/// everything it asks for holds.
pub fn matches(rule: &Rule, item: &Item, now: u64) -> bool {
    rule.enabled
        && !rule.matchers.is_empty()
        && rule.matchers.iter().all(|matcher| holds(matcher, item, now))
}

/// Which rule decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decided {
    /// The rule at this place in the list.
    Rule(usize),
    /// No rule matched; the catch-all took it.
    CatchAll,
}

/// The first rule that matches an item, and what it wants done.
pub fn decide<'a>(
    rules: &'a [Rule],
    catch_all: Option<&'a RuleAction>,
    item: &Item,
    now: u64,
) -> Option<(Decided, &'a RuleAction)> {
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| matches(rule, item, now))
        .map(|(index, rule)| (Decided::Rule(index), &rule.action))
        .or_else(|| catch_all.map(|action| (Decided::CatchAll, action)))
}

/// Which items the rules are asked about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Only the items that are in no zone: what the person sorted by hand
    /// stays sorted.
    #[default]
    Loose,
    /// Every item of the desktop.
    All,
}

/// One thing a plan would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub item: ItemId,
    pub decided: Decided,
    pub action: RuleAction,
}

/// Why the rules leave an item alone although one matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The rule names a zone that is not there.
    NoSuchZone,
    /// The zone is a portal or a smart one: it fills itself.
    ZoneFillsItself,
    /// The zone the item would go to, or the one it is in, is locked.
    Locked,
    /// The item is no file that could be moved.
    NotAFile,
}

/// An item a rule matched and nothing is done with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub item: ItemId,
    pub decided: Decided,
    pub reason: Reason,
}

/// What the rules would do, in the order it would be done.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub held: Vec<Held>,
}

/// What the rules would do to `items` as the placements are now. Nothing
/// is changed. An item that is already where its rule wants it is not in
/// the plan, so a plan made after applying one is empty.
pub fn plan<'a>(
    config: &ZonesConfig,
    items: impl IntoIterator<Item = &'a Item>,
    placements: &Placements,
    scope: Scope,
    now: u64,
) -> Plan {
    let mut plan = Plan::default();
    // (Locked by its own lock, or by the one for all zones.)
    let locked =
        |zone: &ZoneId| config.zone(zone).is_some_and(|zone| zone.lock.members || config.lock.members);
    // A zone that is not in the configuration holds nothing that is shown.
    let shown_zone_of =
        |item: &ItemId| placements.zone_of(item).filter(|zone| config.zone(zone).is_some());
    for item in items {
        let current = shown_zone_of(&item.id);
        if scope == Scope::Loose && current.is_some() {
            continue;
        }
        let Some((decided, action)) = decide(&config.rules, config.catch_all.as_ref(), item, now)
        else {
            continue;
        };
        let mut hold = |reason: Reason| {
            plan.held.push(Held { item: item.id.clone(), decided, reason });
        };
        if current.is_some_and(locked) {
            hold(Reason::Locked);
            continue;
        }
        match action {
            RuleAction::AssignToZone { zone } => {
                if current == Some(zone) {
                    continue;
                }
                match config.zone(zone) {
                    None => hold(Reason::NoSuchZone),
                    Some(target) if target.kind != ZoneKind::Manual => {
                        hold(Reason::ZoneFillsItself)
                    }
                    Some(target) if target.lock.members || config.lock.members => hold(Reason::Locked),
                    Some(_) => plan.steps.push(Step {
                        item: item.id.clone(),
                        decided,
                        action: action.clone(),
                    }),
                }
            }
            RuleAction::MoveToFolder { folder } => match &item.path {
                None => hold(Reason::NotAFile),
                // Already there (a rule that moves into the desktop's own folder).
                Some(path) if paths::parent(path).is_some_and(|at| paths::same_folder(at, folder)) => {}
                Some(_) => plan.steps.push(Step {
                    item: item.id.clone(),
                    decided,
                    action: action.clone(),
                }),
            },
        }
    }
    plan
}

/// Whether a matcher comes true by itself as time passes: it asks how
/// long ago something was.
pub fn asks_the_time(matcher: &Matcher) -> bool {
    match matcher {
        Matcher::Untouched { .. } => true,
        Matcher::Date { newer_than_days, older_than_days, .. } => {
            newer_than_days.is_some() || older_than_days.is_some()
        }
        _ => false,
    }
}

/// Whether a rule is one of those that are tried again on a schedule.
pub fn is_timed(rule: &Rule) -> bool {
    rule.enabled && rule.matchers.iter().any(asks_the_time)
}

/// What the rules that ask the time would do now, to every item of the
/// desktop wherever it is: an item that was sorted by hand a month ago
/// and not touched since is exactly what such a rule is for. Only what a
/// timed rule decides is in the plan: an item whose first matching rule
/// asks nothing of the time is left to `plan`. A rule that moves the file
/// itself is in it only where `move_files` says so.
pub fn plan_timed<'a>(
    config: &ZonesConfig,
    items: impl IntoIterator<Item = &'a Item>,
    placements: &Placements,
    now: u64,
    move_files: bool,
) -> Plan {
    let mut plan = plan(config, items, placements, Scope::All, now);
    let timed = |decided: &Decided| match decided {
        Decided::Rule(index) => config.rules.get(*index).is_some_and(is_timed),
        Decided::CatchAll => false,
    };
    plan.steps.retain(|step| {
        timed(&step.decided) && (move_files || matches!(step.action, RuleAction::AssignToZone { .. }))
    });
    plan.held.retain(|held| timed(&held.decided));
    plan
}

/// Where a step of a plan takes an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Destination {
    /// Shown in a zone; the file stays where it is.
    Zone { zone: ZoneId, title: String },
    /// The file itself goes into a folder.
    Folder { folder: String },
}

/// One line of a plan as it is shown to a person: what would go where,
/// from where, and which rule says so. Lines of items that are held back
/// say why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanLine {
    pub item: ItemId,
    /// The item's name as it is shown.
    pub name: String,
    /// The zone it is in now; none for a loose item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ZoneId>,
    pub destination: Destination,
    pub decided: Decided,
    /// What the person called the rule; empty for the catch-all and for a
    /// rule without a name.
    pub rule: String,
    /// Why nothing is done with it, where nothing is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Reason>,
}

/// A plan in the words a person is shown: the steps in their order, then
/// what is held back. Made before the plan is applied, since it says
/// where each item is now.
pub fn explain<'a>(
    plan: &Plan,
    config: &ZonesConfig,
    item: &dyn Fn(&ItemId) -> Option<&'a Item>,
    placements: &Placements,
) -> Vec<PlanLine> {
    let action_of = |decided: &Decided| match decided {
        Decided::Rule(index) => config.rules.get(*index).map(|rule| &rule.action),
        Decided::CatchAll => config.catch_all.as_ref(),
    };
    let line = |id: &ItemId, decided: Decided, action: Option<&RuleAction>, held: Option<Reason>| {
        let destination = match action? {
            RuleAction::AssignToZone { zone } => Destination::Zone {
                zone: zone.clone(),
                title: config.zone(zone).map(|zone| zone.title.clone()).unwrap_or_default(),
            },
            RuleAction::MoveToFolder { folder } => Destination::Folder { folder: folder.clone() },
        };
        Some(PlanLine {
            item: id.clone(),
            name: item(id)
                .map(|item| item.name.clone())
                .unwrap_or_else(|| paths::file_name(id.as_str()).to_string()),
            from: placements.zone_of(id).filter(|zone| config.zone(zone).is_some()).cloned(),
            destination,
            decided,
            rule: match decided {
                Decided::Rule(index) => config.rules.get(index).map(|rule| rule.name.clone()).unwrap_or_default(),
                Decided::CatchAll => String::new(),
            },
            held,
        })
    };
    let steps = plan.steps.iter().filter_map(|step| line(&step.item, step.decided, Some(&step.action), None));
    let held = plan
        .held
        .iter()
        .filter_map(|held| line(&held.item, held.decided, action_of(&held.decided), Some(held.reason.clone())));
    steps.chain(held).collect()
}

/// One file a rule moved, or asked to be moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub item: ItemId,
    /// What the backend was asked to do.
    pub operation: FileOperation,
    /// What takes it back: as the backend said once it had done it (the
    /// file may have got another name where one of that name was), and
    /// until then what would take it back had it gone as asked.
    pub undo: FileOperation,
}

/// What one application of the rules did, kept so that it can be undone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UndoRecord {
    /// When it was applied, in seconds since 1970.
    pub applied_at: u64,
    /// Where each item was before, and where it was put.
    pub entries: Vec<UndoEntry>,
    pub moved: Vec<Moved>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: gevrek_model::Extra,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoEntry {
    pub item: ItemId,
    pub before: Whereabouts,
    pub action: RuleAction,
}

/// What applying a plan gave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub undo: UndoRecord,
    /// For the backend to carry out, in this order.
    pub operations: Vec<FileOperation>,
}

/// Do what a plan says. Items assigned to a zone come after those that
/// are there, in the plan's order. A file that is to be moved is taken out
/// of wherever it was shown; the move itself is returned.
pub fn apply(plan: &Plan, items: &dyn Fn(&ItemId) -> Option<String>, placements: &mut Placements, now: u64) -> Applied {
    let mut applied = Applied {
        undo: UndoRecord { applied_at: now, ..UndoRecord::default() },
        operations: Vec::new(),
    };
    for step in &plan.steps {
        let before = placements.whereabouts(&step.item);
        match &step.action {
            RuleAction::AssignToZone { zone } => {
                placements.assign(&step.item, zone, None, None);
            }
            RuleAction::MoveToFolder { folder } => {
                let Some(path) = items(&step.item) else {
                    continue;
                };
                let operation = FileOperation::Move { from: path.clone(), to_folder: folder.clone() };
                applied.undo.moved.push(Moved {
                    item: step.item.clone(),
                    undo: operation.reversed().unwrap_or_else(|| operation.clone()),
                    operation: operation.clone(),
                });
                applied.operations.push(operation);
                placements.take(&step.item);
            }
        }
        applied.undo.entries.push(UndoEntry {
            item: step.item.clone(),
            before,
            action: step.action.clone(),
        });
    }
    applied
}

/// Take back what an application did: every item it touched is where it
/// was, in the order it was in. An item the person has since put
/// elsewhere by hand stays where they put it. The files that were moved
/// are returned as operations that bring them back, the last one first.
pub fn undo(record: &UndoRecord, placements: &mut Placements) -> Vec<FileOperation> {
    // The last step first: each entry says where its item was when its
    // step was taken, which is where it is again once the later steps
    // are taken back.
    for entry in record.entries.iter().rev() {
        let still_there = match &entry.action {
            RuleAction::AssignToZone { zone } => placements.zone_of(&entry.item) == Some(zone),
            RuleAction::MoveToFolder { .. } => {
                placements.whereabouts(&entry.item) == Whereabouts::Loose { position: None }
            }
        };
        if still_there {
            placements.put_back(&entry.item, &entry.before);
        }
    }
    record.moved.iter().rev().map(|moved| moved.undo.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZoneDefinition;
    use crate::item::{ItemFacts, ItemKind};

    const NOW: u64 = 1_800_000_000;

    fn file(name: &str) -> Item {
        Item::file(&format!("/desk/{name}"))
    }

    fn with(name: &str, change: impl FnOnce(&mut ItemFacts)) -> Item {
        let mut item = file(name);
        change(&mut item.facts);
        item
    }

    fn assign(name: &str, matchers: Vec<Matcher>, zone: &str) -> Rule {
        Rule {
            name: name.to_string(),
            matchers,
            action: RuleAction::AssignToZone { zone: ZoneId::new(zone) },
            ..Rule::default()
        }
    }

    fn extension(extensions: &[&str]) -> Matcher {
        Matcher::Extension { any_of: extensions.iter().map(|text| text.to_string()).collect() }
    }

    fn config(rules: Vec<Rule>) -> ZonesConfig {
        ZonesConfig {
            zones: ["pictures", "papers", "old", "rest"]
                .iter()
                .map(|id| ZoneDefinition::manual(id, id))
                .collect(),
            rules,
            ..ZonesConfig::default()
        }
    }

    fn paths_of(items: &[Item]) -> impl Fn(&ItemId) -> Option<String> + '_ {
        |id| items.iter().find(|item| &item.id == id).and_then(|item| item.path.clone())
    }

    #[test]
    fn a_glob_fits_or_does_not() {
        for (pattern, text, fits) in [
            ("*.png", "photo.PNG", true),
            ("*.png", "photo.png.txt", false),
            ("report-??.pdf", "report-07.pdf", true),
            ("report-??.pdf", "report-7.pdf", false),
            ("*", "", true),
            ("", "", true),
            ("", "a", false),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("*screenshot*", "My Screenshot (3).png", true),
            ("image/*", "image/jpeg", true),
            ("image/*", "video/mp4", false),
            ("**a", "bbba", true),
            ("ü*", "Über.txt", true),
        ] {
            assert_eq!(glob(pattern, text), fits, "{pattern} on {text}");
        }
    }

    #[test]
    fn each_matcher_asks_for_its_own_fact() {
        let day = DAY;
        let photo = with("Holiday.JPG", |facts| {
            facts.mime = Some("image/jpeg".to_string());
            facts.size = Some(2_000_000);
            facts.created = Some(NOW - 400 * day);
            facts.modified = Some(NOW - 100 * day);
            facts.accessed = Some(NOW - 40 * day);
        });
        let mut link = with("Editor.lnk", |facts| {
            facts.shortcut_target = Some("C:\\Program Files\\Editor\\editor.exe".to_string());
            facts.modified = Some(NOW - 2 * day);
        });
        link.kind = ItemKind::Shortcut;
        let bare = file("notes");
        let date = |field, newer: Option<u32>, older: Option<u32>| Matcher::Date {
            field,
            after: None,
            before: None,
            newer_than_days: newer,
            older_than_days: older,
        };
        // (matcher, photo, link, bare)
        for (matcher, on_photo, on_link, on_bare) in [
            (extension(&["jpg", "png"]), true, false, false),
            (extension(&[".JPG"]), true, false, false),
            (extension(&[]), false, false, false),
            (Matcher::Mime { pattern: "image/*".to_string() }, true, false, false),
            (Matcher::Name { pattern: "holi*".to_string() }, true, false, false),
            (Matcher::Name { pattern: "*".to_string() }, true, true, true),
            (Matcher::Kind { any_of: vec![ItemKind::Shortcut] }, false, true, false),
            (Matcher::Kind { any_of: vec![ItemKind::File] }, true, false, true),
            (date(DateField::Modified, Some(7), None), false, true, false),
            (date(DateField::Modified, None, Some(30)), true, false, false),
            (date(DateField::Created, None, Some(365)), true, false, false),
            (date(DateField::Accessed, Some(60), Some(30)), true, false, false),
            (
                Matcher::Date {
                    field: DateField::Modified,
                    after: Some(NOW - 101 * day),
                    before: Some(NOW - 99 * day),
                    newer_than_days: None,
                    older_than_days: None,
                },
                true,
                false,
                false,
            ),
            (Matcher::Size { at_least: Some(1_000_000), at_most: None }, true, false, false),
            (Matcher::Size { at_least: None, at_most: Some(1_000_000) }, false, false, false),
            (Matcher::Size { at_least: Some(2_000_000), at_most: Some(2_000_000) }, true, false, false),
            (Matcher::ShortcutTarget { pattern: "*\\editor.exe".to_string() }, false, true, false),
            (Matcher::Untouched { days: 30 }, true, false, false),
            (Matcher::Untouched { days: 41 }, false, false, false),
            (Matcher::Untouched { days: 1 }, true, true, false),
        ] {
            assert_eq!(holds(&matcher, &photo, NOW), on_photo, "{matcher:?} on the photo");
            assert_eq!(holds(&matcher, &link, NOW), on_link, "{matcher:?} on the link");
            assert_eq!(holds(&matcher, &bare, NOW), on_bare, "{matcher:?} on the bare file");
        }
    }

    #[test]
    fn untouched_depends_on_the_day_it_is_asked() {
        let item = with("a.txt", |facts| facts.modified = Some(NOW));
        let matcher = Matcher::Untouched { days: 30 };
        assert!(!holds(&matcher, &item, NOW + 29 * DAY));
        assert!(holds(&matcher, &item, NOW + 30 * DAY));
        // A clock that went back does not make everything old.
        assert!(!holds(&matcher, &item, 5));
    }

    #[test]
    fn the_first_rule_that_matches_decides() {
        let rules = vec![
            assign("screenshots", vec![Matcher::Name { pattern: "screenshot*".to_string() }], "papers"),
            assign("pictures", vec![extension(&["png", "jpg"])], "pictures"),
            assign("all pngs again", vec![extension(&["png"])], "old"),
        ];
        let decided = |name: &str, rules: &[Rule], catch_all: Option<&RuleAction>| {
            decide(rules, catch_all, &file(name), NOW).map(|(decided, _)| decided)
        };
        assert_eq!(decided("Screenshot 1.png", &rules, None), Some(Decided::Rule(0)));
        assert_eq!(decided("cat.png", &rules, None), Some(Decided::Rule(1)));
        assert_eq!(decided("notes.txt", &rules, None), None);
        // The order is what counts: turned around, the last is first.
        let mut turned = rules.clone();
        turned.reverse();
        assert_eq!(decided("Screenshot 1.png", &turned, None), Some(Decided::Rule(0)));
        assert_eq!(
            decide(&turned, None, &file("Screenshot 1.png"), NOW).unwrap().1,
            &RuleAction::AssignToZone { zone: ZoneId::new("old") }
        );
        // A rule that is switched off, or asks for nothing, matches nothing.
        let mut off = rules.clone();
        off[0].enabled = false;
        off[1].matchers.clear();
        assert_eq!(decided("Screenshot 1.png", &off, None), Some(Decided::Rule(2)));
        assert_eq!(decided("cat.jpg", &off, None), None);
        // Every matcher of a rule must hold.
        let both = vec![assign(
            "large pictures",
            vec![extension(&["png"]), Matcher::Size { at_least: Some(10), at_most: None }],
            "pictures",
        )];
        assert_eq!(decided("cat.png", &both, None), None);
    }

    #[test]
    fn the_catch_all_takes_what_no_rule_matched() {
        let mut config = config(vec![assign("pictures", vec![extension(&["png"])], "pictures")]);
        let items = [file("cat.png"), file("notes.txt")];
        let placements = Placements::default();
        let without = plan(&config, &items, &placements, Scope::Loose, NOW);
        assert_eq!(without.steps.len(), 1);
        config.catch_all = Some(RuleAction::AssignToZone { zone: ZoneId::new("rest") });
        let with = plan(&config, &items, &placements, Scope::Loose, NOW);
        assert_eq!(with.steps.len(), 2);
        assert_eq!(with.steps[0].decided, Decided::Rule(0));
        assert_eq!(with.steps[1].decided, Decided::CatchAll);
        assert_eq!(with.steps[1].item, items[1].id);
    }

    #[test]
    fn a_dry_run_changes_nothing_and_says_what_applying_does() {
        let mut config = config(vec![
            assign("pictures", vec![extension(&["png"])], "pictures"),
            Rule {
                name: "old".to_string(),
                matchers: vec![Matcher::Untouched { days: 90 }],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
        ]);
        config.catch_all = Some(RuleAction::AssignToZone { zone: ZoneId::new("rest") });
        let items = [
            file("cat.png"),
            with("ancient.doc", |facts| facts.modified = Some(NOW - 200 * DAY)),
            file("notes.txt"),
            file("dog.png"),
        ];
        let mut placements = Placements::default();
        placements.assign(&items[3].id, &ZoneId::new("papers"), None, None);
        let before = placements.clone();

        let dry = plan(&config, &items, &placements, Scope::All, NOW);
        assert_eq!(placements, before, "a dry run changed something");
        assert_eq!(dry, plan(&config, &items, &placements, Scope::All, NOW));
        assert_eq!(dry.steps.len(), 4);

        let applied = apply(&dry, &paths_of(&items), &mut placements, NOW);
        // Every step of the dry run is what happened.
        for step in &dry.steps {
            match &step.action {
                RuleAction::AssignToZone { zone } => {
                    assert_eq!(placements.zone_of(&step.item), Some(zone), "{}", step.item)
                }
                RuleAction::MoveToFolder { folder } => {
                    assert_eq!(placements.zone_of(&step.item), None);
                    assert!(applied.operations.contains(&FileOperation::Move {
                        from: "/desk/ancient.doc".to_string(),
                        to_folder: folder.clone(),
                    }));
                }
            }
        }
        assert_eq!(applied.operations.len(), 1);
        assert_eq!(applied.undo.entries.len(), dry.steps.len());
        assert_eq!(placements.broken(), None);
        // And nothing is left to do, for what is still there.
        let still: Vec<Item> = items.iter().filter(|item| item.name != "ancient.doc").cloned().collect();
        assert_eq!(plan(&config, &still, &placements, Scope::All, NOW), Plan::default());
    }

    #[test]
    fn only_loose_items_are_sorted_unless_all_are_asked_for() {
        let config = config(vec![assign("pictures", vec![extension(&["png"])], "pictures")]);
        let items = [file("cat.png"), file("dog.png")];
        let mut placements = Placements::default();
        placements.assign(&items[0].id, &ZoneId::new("papers"), None, None);
        let loose = plan(&config, &items, &placements, Scope::Loose, NOW);
        assert_eq!(loose.steps.iter().map(|step| &step.item).collect::<Vec<_>>(), [&items[1].id]);
        let all = plan(&config, &items, &placements, Scope::All, NOW);
        assert_eq!(all.steps.len(), 2);
        // An item in a zone that is no longer there counts as loose.
        placements.assign(&items[0].id, &ZoneId::new("gone"), None, None);
        assert_eq!(plan(&config, &items, &placements, Scope::Loose, NOW).steps.len(), 2);
    }

    #[test]
    fn what_cannot_be_done_is_held_back_with_its_reason() {
        let mut config = config(vec![
            assign("to nowhere", vec![extension(&["a"])], "nowhere"),
            assign("to a portal", vec![extension(&["b"])], "portal"),
            assign("to a locked zone", vec![extension(&["c"])], "locked"),
            assign("out of a locked zone", vec![extension(&["d"])], "pictures"),
            Rule {
                name: "move".to_string(),
                matchers: vec![Matcher::Name { pattern: "This PC".to_string() }],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
        ]);
        config.zones.push(ZoneDefinition {
            kind: ZoneKind::Portal { directory: "/downloads".to_string() },
            ..ZoneDefinition::manual("portal", "Portal")
        });
        let mut locked = ZoneDefinition::manual("locked", "Locked");
        locked.lock.members = true;
        config.zones.push(locked);
        let computer = Item {
            id: ItemId::new("::computer"),
            kind: ItemKind::Virtual,
            name: "This PC".to_string(),
            ..Item::default()
        };
        let items = [file("x.a"), file("x.b"), file("x.c"), file("x.d"), computer];
        let mut placements = Placements::default();
        placements.assign(&items[3].id, &ZoneId::new("locked"), None, None);
        let plan = plan(&config, &items, &placements, Scope::All, NOW);
        assert_eq!(plan.steps, []);
        let reasons: Vec<_> = plan.held.iter().map(|held| (held.item.as_str(), &held.reason)).collect();
        assert_eq!(
            reasons,
            [
                ("/desk/x.a", &Reason::NoSuchZone),
                ("/desk/x.b", &Reason::ZoneFillsItself),
                ("/desk/x.c", &Reason::Locked),
                ("/desk/x.d", &Reason::Locked),
                ("::computer", &Reason::NotAFile),
            ]
        );
    }

    #[test]
    fn undoing_puts_everything_exactly_where_it_was() {
        let mut config = config(vec![
            assign("pictures", vec![extension(&["png"])], "pictures"),
            Rule {
                name: "old".to_string(),
                matchers: vec![Matcher::Untouched { days: 90 }],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
        ]);
        config.catch_all = Some(RuleAction::AssignToZone { zone: ZoneId::new("rest") });
        let old = |name: &str| with(name, |facts| facts.modified = Some(NOW - 200 * DAY));
        let items = [
            file("a.png"),
            file("b.txt"),
            file("c.png"),
            old("d.doc"),
            file("e.png"),
            old("f.doc"),
            file("g.txt"),
        ];
        let papers = ZoneId::new("papers");
        let mut placements = Placements::default();
        // papers holds a, d, c, f, e in that order; b has a place of its
        // own; g is simply loose.
        for index in [0, 3, 2, 5, 4] {
            placements.assign(&items[index].id, &papers, None, None);
        }
        placements.loosen(
            &items[1].id,
            Some(crate::placement::LoosePosition {
                display: crate::ids::DisplayId::new("d"),
                x: 30,
                y: 40,
            }),
        );
        let before = placements.clone();

        let dry = plan(&config, &items, &placements, Scope::All, NOW);
        let applied = apply(&dry, &paths_of(&items), &mut placements, NOW);
        assert_ne!(placements, before);
        assert_eq!(placements.items_of(&papers), []);
        assert_eq!(applied.operations.len(), 2);

        let back = undo(&applied.undo, &mut placements);
        assert_eq!(placements, before);
        // The files come back, the last moved first.
        assert_eq!(
            back,
            [
                FileOperation::Move { from: "/archive/f.doc".to_string(), to_folder: "/desk".to_string() },
                FileOperation::Move { from: "/archive/d.doc".to_string(), to_folder: "/desk".to_string() },
            ]
        );
    }

    #[test]
    fn a_rule_is_timed_when_it_asks_how_long_ago() {
        let date = |newer: Option<u32>, older: Option<u32>, after: Option<u64>| Matcher::Date {
            field: DateField::Modified,
            after,
            before: None,
            newer_than_days: newer,
            older_than_days: older,
        };
        for (matcher, asks) in [
            (Matcher::Untouched { days: 30 }, true),
            (date(Some(1), None, None), true),
            (date(None, Some(7), None), true),
            // A day of the calendar is the same day whenever it is asked.
            (date(None, None, Some(NOW)), false),
            (extension(&["png"]), false),
            (Matcher::Size { at_least: Some(1), at_most: None }, false),
        ] {
            assert_eq!(asks_the_time(&matcher), asks, "{matcher:?}");
        }
        let stale = assign("stale", vec![extension(&["doc"]), Matcher::Untouched { days: 30 }], "old");
        assert!(is_timed(&stale));
        assert!(!is_timed(&Rule { enabled: false, ..stale }));
        assert!(!is_timed(&assign("pictures", vec![extension(&["png"])], "pictures")));
    }

    #[test]
    fn the_timed_plan_holds_only_what_a_timed_rule_decides() {
        let mut config = config(vec![
            assign("stale", vec![Matcher::Untouched { days: 30 }], "old"),
            Rule {
                name: "ancient".to_string(),
                matchers: vec![extension(&["tmp"]), Matcher::Untouched { days: 1 }],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
            assign("pictures", vec![extension(&["png"])], "pictures"),
        ]);
        config.catch_all = Some(RuleAction::AssignToZone { zone: ZoneId::new("rest") });
        let aged = |name: &str, days: u64| with(name, |facts| facts.modified = Some(NOW - days * DAY));
        let items = [aged("old.png", 40), aged("new.png", 2), aged("scratch.tmp", 3), aged("note.txt", 2)];
        let mut placements = Placements::default();
        // Sorted by hand long ago: exactly what a timed rule is for.
        placements.assign(&items[0].id, &ZoneId::new("pictures"), None, None);
        let names = |plan: &Plan| plan.steps.iter().map(|step| step.item.as_str().to_string()).collect::<Vec<_>>();

        let timed = plan_timed(&config, &items, &placements, NOW, false);
        assert_eq!(names(&timed), ["/desk/old.png"]);
        assert_eq!(timed.steps[0].decided, Decided::Rule(0));
        // With the files themselves: the one that is to be moved as well.
        let moving = plan_timed(&config, &items, &placements, NOW, true);
        assert_eq!(names(&moving), ["/desk/old.png", "/desk/scratch.tmp"]);
        // The rules that ask nothing of the time, and the catch-all, are
        // left to the plan a person asks for.
        let whole = plan(&config, &items, &placements, Scope::All, NOW);
        assert_eq!(whole.steps.len(), 4);
        // What a timed rule decides and cannot do is said, the rest is not.
        config.zones.retain(|zone| zone.id.as_str() != "old");
        let held = plan_timed(&config, &items, &placements, NOW, false);
        assert!(held.steps.is_empty());
        assert_eq!(held.held, [Held { item: items[0].id.clone(), decided: Decided::Rule(0), reason: Reason::NoSuchZone }]);
    }

    #[test]
    fn a_plan_is_said_in_lines_a_person_can_read() {
        let mut config = config(vec![
            assign("Pictures", vec![extension(&["png"])], "pictures"),
            Rule {
                matchers: vec![extension(&["doc"])],
                action: RuleAction::MoveToFolder { folder: "/archive".to_string() },
                ..Rule::default()
            },
            assign("Lost", vec![extension(&["lost"])], "nowhere"),
        ]);
        config.catch_all = Some(RuleAction::AssignToZone { zone: ZoneId::new("rest") });
        config.zones[0].title = "My pictures".to_string();
        let mut drive = file("drive");
        drive.path = None;
        drive.facts.extension = Some("doc".to_string());
        let items = [file("a.png"), file("b.doc"), drive, file("c.lost"), file("d.txt")];
        let mut placements = Placements::default();
        placements.assign(&items[0].id, &ZoneId::new("papers"), None, None);
        let dry = plan(&config, &items, &placements, Scope::All, NOW);
        let lookup = |id: &ItemId| items.iter().find(|item| &item.id == id);
        let lines = explain(&dry, &config, &lookup, &placements);
        // Each line as a person would be shown it.
        let said: Vec<String> = lines
            .iter()
            .map(|line| {
                let from = line.from.as_ref().map_or("the desktop", ZoneId::as_str);
                let to = match &line.destination {
                    Destination::Zone { zone, title } => format!("{zone} ({title})"),
                    Destination::Folder { folder } => folder.clone(),
                };
                let held = line.held.as_ref().map(|reason| format!(", held: {reason:?}")).unwrap_or_default();
                format!("{} from {from} to {to} by [{}]{held}", line.name, line.rule)
            })
            .collect();
        assert_eq!(
            said,
            [
                "a.png from papers to pictures (My pictures) by [Pictures]",
                "b.doc from the desktop to /archive by []",
                "d.txt from the desktop to rest (rest) by []",
                "drive from the desktop to /archive by [], held: NotAFile",
                "c.lost from the desktop to nowhere () by [Lost], held: NoSuchZone",
            ]
        );
        assert_eq!(lines[2].decided, Decided::CatchAll);
        // As JSON: the line a command line prints from.
        assert_eq!(
            serde_json::to_value(&lines[0]).unwrap(),
            serde_json::json!({
                "item": "/desk/a.png", "name": "a.png", "from": "papers",
                "destination": {"to": "zone", "zone": "pictures", "title": "My pictures"},
                "decided": {"rule": 0}, "rule": "Pictures"
            })
        );
        let text = serde_json::to_string(&lines).unwrap();
        assert_eq!(serde_json::from_str::<Vec<PlanLine>>(&text).unwrap(), lines);
    }

    #[test]
    fn the_lock_for_all_zones_holds_the_rules_back_as_a_zones_own_does() {
        let mut config = config(vec![assign("pictures", vec![extension(&["png"])], "pictures")]);
        let items = [file("a.png"), file("b.png")];
        let mut placements = Placements::default();
        placements.assign(&items[1].id, &ZoneId::new("papers"), None, None);
        assert_eq!(plan(&config, &items, &placements, Scope::All, NOW).steps.len(), 2);
        config.lock.members = true;
        let locked = plan(&config, &items, &placements, Scope::All, NOW);
        assert!(locked.steps.is_empty());
        assert!(locked.held.iter().all(|held| held.reason == Reason::Locked));
        assert_eq!(locked.held.len(), 2);
        // Where a zone stands is another lock, and none of the rules' concern.
        config.lock = crate::config::Lock { geometry: true, members: false };
        assert_eq!(plan(&config, &items, &placements, Scope::All, NOW).steps.len(), 2);
    }

    #[test]
    fn undoing_leaves_what_the_person_moved_since() {
        let config = config(vec![assign("pictures", vec![extension(&["png"])], "pictures")]);
        let items = [file("a.png"), file("b.png")];
        let mut placements = Placements::default();
        let dry = plan(&config, &items, &placements, Scope::Loose, NOW);
        let applied = apply(&dry, &paths_of(&items), &mut placements, NOW);
        // The person takes one of the two elsewhere.
        placements.assign(&items[0].id, &ZoneId::new("papers"), None, None);
        undo(&applied.undo, &mut placements);
        assert_eq!(placements.zone_of(&items[0].id), Some(&ZoneId::new("papers")));
        assert_eq!(placements.zone_of(&items[1].id), None);
    }
}
