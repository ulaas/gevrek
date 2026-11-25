//! What `gevrek zones <word> ...` asks of the zones: the words of the
//! command line, and the commands they become.
//!
//! Every word here ends as one or more commands of the zones' core, sent
//! the way `gevrek zones send` sends them; nothing can be done here that
//! a script could not do with `send`, and nothing with the pointer that
//! cannot be done here. The words are turned into commands by a function
//! that is given the zones as they were last told (`asked`), so that a
//! zone can be named by its title and an item by its name, and so that
//! all of it can be tested without a window manager.
//!
//! The commands are written as JSON, not as the core's own type: this
//! program may be older or newer than the shell that reads them, and the
//! shell is the one that says what it does not know.

use clap::{Subcommand, ValueEnum};
use gevrek_zones::ipc::{ItemSnapshot, ZoneSnapshot, ZonesSnapshot};
use serde_json::{json, Map, Value};

/// On, off, or the other of what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Switch {
    On,
    Off,
    Toggle,
}

impl Switch {
    /// As the core takes it: a value, or none for "the other".
    fn value(self) -> Option<bool> {
        match self {
            Switch::On => Some(true),
            Switch::Off => Some(false),
            Switch::Toggle => None,
        }
    }

    /// For a command that has no "the other": the value, from what is.
    fn from(self, now: bool) -> bool {
        self.value().unwrap_or(!now)
    }
}

/// What of a zone is locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LockWhat {
    /// Where it is and how large
    Geometry,
    /// What is in it
    Members,
    /// Both
    All,
    /// Neither (the same as `unlock`)
    None,
}

/// Which items the rules are asked about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum RuleScope {
    /// Only the items that are in no zone
    #[default]
    Loose,
    /// Every item of the desktop
    All,
}

impl RuleScope {
    pub fn word(self) -> &'static str {
        match self {
            RuleScope::Loose => "loose",
            RuleScope::All => "all",
        }
    }
}

/// The kinds of zone `create` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindWord {
    /// Filled by hand and by the rules
    Manual,
    /// A window into a folder (--dir)
    Portal,
    /// Fills itself with what matches (--query, --from)
    Smart,
}

/// What `gevrek zones` does.
#[derive(Subcommand, Debug, PartialEq)]
pub enum ZonesAction {
    /// The zones: where each is, how it is, and how much it holds
    List {
        /// With --json: every item of every zone and of the desktop too
        /// (a desktop of thousands of items makes this large)
        #[arg(long)]
        items: bool,
    },
    /// One zone with its items; without a zone, show the zones again
    /// after `hide`
    #[command(alias = "info")]
    Show {
        /// Its id, its title, or the beginning of either
        zone: Option<String>,
    },
    /// Make a zone
    Create {
        /// What it is called
        #[arg(long)]
        title: Option<String>,
        /// manual, portal or smart (told from --dir and --query where
        /// it is not said)
        #[arg(long, value_enum)]
        kind: Option<KindWord>,
        /// For a portal: the folder it shows
        #[arg(long, value_name = "FOLDER")]
        dir: Option<String>,
        /// For a smart zone: what its items match, as the rules write it
        /// (`'[{"match":"extension","any_of":["pdf"]}]'`), or short:
        /// `ext:pdf,docx`, `name:report*`, `mime:image/*`, `kind:folder`,
        /// `untouched:30`, several joined with `+`
        #[arg(long)]
        query: Option<String>,
        /// For a smart zone: what the query is asked of, where that is
        /// not the desktop: recent, downloads, running-apps, pinned, or
        /// a folder
        #[arg(long, value_name = "SOURCE")]
        from: Option<String>,
        /// The display it is made on: its id, its number (1 is the
        /// first), or `primary`
        #[arg(long)]
        display: Option<String>,
        /// Its top left corner on that display, as X,Y
        #[arg(long, value_name = "X,Y", allow_hyphen_values = true)]
        at: Option<String>,
        /// Its size, as W,H (or WxH)
        #[arg(long, value_name = "W,H")]
        size: Option<String>,
    },
    /// Take a zone away; its items stay on the desktop, loose
    Delete { zone: String },
    /// Give a zone another title
    Rename { zone: String, title: String },
    /// Put a zone's top left corner at X,Y of the desktop, or with
    /// --display of that display
    Move {
        zone: String,
        #[arg(value_name = "X,Y", allow_hyphen_values = true)]
        to: String,
        /// Counted from this display's corner: its id, its number, or
        /// `primary`
        #[arg(long)]
        display: Option<String>,
        /// Exactly there: not drawn to edges, other zones or the grid
        #[arg(long)]
        no_snap: bool,
    },
    /// Make a zone W,H large (its top left corner stays)
    Resize {
        zone: String,
        #[arg(value_name = "W,H")]
        size: String,
        /// Exactly so: not drawn to edges, other zones or the grid
        #[arg(long)]
        no_snap: bool,
    },
    /// Change how a zone shows its items and how it looks, as
    /// `key=value ...`: view=grid|list, sort=manual|name|type|date|size,
    /// descending=on|off, icon-size=16..256, background=#rrggbb[aa],
    /// opacity=0..100, blur=on|off, corner-radius=0..64,
    /// title-colour=#rrggbb, title-on-hover=on|off,
    /// title-bar=full|plain|hidden, labels=on|off, icon-tint=#rrggbb[aa],
    /// icon-saturation=0..100, always-visible=on|off, private=on|off,
    /// reserves-space=on|off. `none` for a value of the look takes the
    /// zone's own away (it looks as the others do)
    Set {
        zone: String,
        #[arg(required = true, value_name = "KEY=VALUE")]
        settings: Vec<String>,
    },
    /// Roll a zone up to its title, or down again
    RollUp {
        zone: String,
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// Lock a zone: where it is, what is in it, or both
    Lock {
        zone: String,
        #[arg(value_enum, default_value = "all")]
        what: LockWhat,
    },
    /// Unlock a zone
    Unlock { zone: String },
    /// Make a zone a tab beside another (it takes that one's place)
    Tab {
        zone: String,
        /// The zone it joins
        #[arg(long, value_name = "ZONE")]
        with: String,
    },
    /// Take a zone out of its tabs
    Untab { zone: String },
    /// Bring a tab to the front
    Front { zone: String },
    /// Bring the next tab of a zone's group to the front
    NextTab {
        zone: String,
        /// The one before
        #[arg(long)]
        backwards: bool,
    },
    /// Lock every zone at once, whatever its own lock says, or stop
    LockAll {
        #[arg(value_enum, default_value = "all")]
        what: LockWhat,
    },
    /// Keep a zone shown when the zones are hidden (`hide`), or not
    KeepZone {
        zone: String,
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// Keep one item shown when the zones are hidden (`hide`), or not
    KeepItem {
        item: String,
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// In a portal, go somewhere without leaving the zone: `up`, `back`,
    /// `forward`, or into a folder it shows (by its name)
    Go {
        zone: String,
        /// up, back, forward, or a folder the portal shows
        to: String,
    },
    /// Ask a zone's folder or source again for what it holds
    Refresh { zone: String },
    /// Make a folder: in the folder a portal stands in, or with --in in
    /// any folder (the desktop's own, to make one on the desktop)
    NewFolder {
        /// A portal; left out with --in
        #[arg(required_unless_present = "folder")]
        zone: Option<String>,
        name: Option<String>,
        /// The folder it is made in, by its path
        #[arg(long = "in", value_name = "FOLDER")]
        folder: Option<String>,
    },
    /// Move files into a folder (or copy them): what dragging an item of
    /// a portal onto a folder does
    MoveToFolder {
        #[arg(required = true)]
        items: Vec<String>,
        /// Where they go, by its path
        #[arg(long, value_name = "FOLDER")]
        folder: String,
        /// Copied, not moved
        #[arg(long)]
        copy: bool,
    },
    /// Bring files from elsewhere into a folder (the desktop's own, a
    /// portal's), and show them in a zone: what dropping files from
    /// another program does
    BringIn {
        /// The files, by their paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// The folder they are brought to, by its path
        #[arg(long, value_name = "FOLDER")]
        folder: String,
        /// The zone they are shown in once they are there
        #[arg(long)]
        zone: Option<String>,
        /// Copied, not moved
        #[arg(long)]
        copy: bool,
    },
    /// Call off a copy or move that is under way, by its number; without
    /// a number, the one there is, or the list of them when there are
    /// several
    Cancel { request: Option<u64> },
    /// Open a rolled-up zone for now, or close it (it stays rolled up)
    Expand {
        zone: String,
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// Put a tab at another place in its row (0 is the first)
    MoveTab { zone: String, index: usize },
    /// Take a tab out of its row and put it at X,Y of the desktop
    SplitTab {
        zone: String,
        #[arg(value_name = "X,Y", allow_hyphen_values = true)]
        to: String,
        /// Exactly there: not drawn to edges, other zones or the grid
        #[arg(long)]
        no_snap: bool,
    },
    /// Change what holds for all zones, as `key=value ...` with the
    /// configuration's own keys under `desktop.zones`, dots for what
    /// lies inside: snap.grid=16 roll_up.expand_on=click
    /// look.opacity=70 privacy.when_shared=everything. `none` puts a key
    /// back to its default
    Settings {
        #[arg(required = true, value_name = "KEY=VALUE")]
        settings: Vec<String>,
    },
    /// Choose items, as pressing them with Ctrl or Shift does
    Select {
        items: Vec<String>,
        /// replace (only these), add, remove, or toggle
        #[arg(long, default_value = "replace")]
        mode: String,
    },
    /// Walk the keyboard through the items of the zone that has it:
    /// left, right, up, down, first, last
    Walk {
        step: String,
        /// Add what is walked to to the chosen items
        #[arg(long)]
        extend: bool,
    },
    /// Open the chosen items, or the one the keyboard is on
    OpenChosen,
    /// Move the chosen items into a zone, onto the bare desktop (`desk`),
    /// or to what lies left, right, up or down
    MoveChosen {
        /// A zone, desk, or left, right, up, down
        target: String,
    },
    /// Step back, as Escape does: stop peeking; else empty the filter;
    /// else choose nothing; else let the keyboard go
    Dismiss,
    /// Show only the items of a zone whose names fit what is typed; with
    /// nothing typed, all of them again
    Filter {
        zone: String,
        #[arg(default_value = "")]
        text: String,
    },
    /// Put items into a zone (into a portal: the files are moved into
    /// its folder)
    Add {
        zone: String,
        /// Each by its id (path), its name, or the beginning of its name
        #[arg(required = true)]
        items: Vec<String>,
        /// The place in the zone's order the first one takes (0 is the
        /// first); without it they are put at the end
        #[arg(long)]
        index: Option<usize>,
    },
    /// Take items out of their zones; they are loose on the desktop
    Remove {
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Put an item on the bare desktop at X,Y of a display
    Place {
        item: String,
        #[arg(value_name = "X,Y", allow_hyphen_values = true)]
        at: String,
        /// Its id, its number, or `primary` (the primary one without it)
        #[arg(long)]
        display: Option<String>,
    },
    /// Open items, as a double click does
    Open {
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Give the keyboard to an item, wherever it is: its zone's tab comes
    /// to the front and a rolled-up zone opens (how a search hit is gone
    /// to)
    GoTo { item: String },
    /// Give an item's file another name (`undo` takes it back)
    RenameItem { item: String, name: String },
    /// Put items into the system's bin (`undo` takes it back)
    Trash {
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Take back the last file operation (a rename, the bin, a move into
    /// a portal)
    Undo,
    /// Hide every zone that is not always visible; with a zone, hide
    /// that one on the displays as they are now
    Hide { zone: Option<String> },
    /// Show the zones again; with a zone, show that one again
    Unhide { zone: Option<String> },
    /// Hide the zones if they are shown, show them if they are hidden
    Toggle,
    /// Bring the zones above the windows, or let them go back
    Peek {
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// Draw every icon of the desktop grey, or in its colours again (the
    /// looks that are set stay as they are: this is laid over them)
    Grey {
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
    },
    /// Make a zone for each type of item on the desktop (folders,
    /// programs, documents, pictures...: `desktop.zones.auto` says which)
    /// and put the items into them; a type that has its zone already
    /// gets what is new of it. `auto-undo` takes it back
    AutoCreate {
        /// Say what would be made, and where, and do nothing
        #[arg(long)]
        dry_run: bool,
        /// Which items are sorted: `loose` leaves what is in a zone where
        /// it is; `all` also sorts what the zones filled by hand hold
        #[arg(long, value_enum, default_value = "loose")]
        scope: RuleScope,
        /// How many items a type needs for a zone of its own (the
        /// configuration's `auto.min_items` without it, which comes as 2)
        #[arg(long, value_name = "N")]
        min_items: Option<u32>,
        /// Add no rules for what comes to the desktop later
        #[arg(long)]
        no_rules: bool,
        /// The display new zones are made on: its id, its number (1 is
        /// the first), or `primary` (the primary one without it)
        #[arg(long)]
        display: Option<String>,
    },
    /// Take back the last `auto-create`: the zones it made, where it put
    /// the items, the rules it added
    AutoUndo,
    /// Cover what the zones hold, or uncover it. With --zone: `on` makes
    /// that zone private (covered until asked for), `off` uncovers it for
    /// now (until the screen is next shared; `set <zone> private=off`
    /// for good)
    Privacy {
        #[arg(value_enum, default_value = "toggle")]
        to: Switch,
        /// Only this zone
        #[arg(long)]
        zone: Option<String>,
    },
    /// The rules that sort items: list them, see what they would do,
    /// apply them, take the last application back
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
    /// Named copies of the whole layout: save, restore (apply), list,
    /// delete, export to a file, import from one
    #[command(alias = "layout")]
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },
    /// The arrangements, one for each set of displays that was seen
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Give a zone the keyboard: by its name, or the next one `left`,
    /// `right`, `up` or `down` of the one that has it; `desk` is the bare
    /// desktop with its loose items; `none` lets the keyboard go
    Focus {
        /// A zone, or left, right, up, down, desk, none
        target: String,
    },
    /// Find items by some of their name, across every zone
    Search { text: String },
    /// Print what happens to the zones, as it happens (with --json one
    /// object a line)
    Events,
    /// Send the zones a command as JSON (`'{"command":"quick_hide"}'`)
    /// and print what they answer: `Done` with what happened, or why not
    Send { command: String },
    /// Put Explorer's own desktop icons back as they were before the
    /// zones took them over (Windows; elsewhere there is nothing to do)
    RestoreExplorer,
}

/// What `gevrek zones rules` does.
#[derive(Subcommand, Debug, PartialEq)]
pub enum RulesAction {
    /// The rules, in the order they are tried
    List,
    /// What the rules would do, and do nothing: a table of item, where
    /// it would go, and by which rule
    #[command(alias = "preview")]
    DryRun {
        /// Which items are asked about: `loose` leaves what was sorted
        /// by hand where it is
        #[arg(long, value_enum, default_value = "loose")]
        scope: RuleScope,
    },
    /// Carry the rules out
    #[command(alias = "run")]
    Apply {
        /// Which items are asked about: `loose` leaves what was sorted
        /// by hand where it is
        #[arg(long, value_enum, default_value = "loose")]
        scope: RuleScope,
    },
    /// Try the rules that ask how old an item is, now (the shell does
    /// this by itself every `timed_rules.every_minutes`)
    Check,
    /// Take back the last time the rules were applied
    Undo,
}

/// What `gevrek zones snapshot` does.
#[derive(Subcommand, Debug, PartialEq)]
pub enum SnapshotAction {
    /// Keep the layout as it is now under a name
    Save { name: String },
    /// Make the layout what it was when the copy was taken
    #[command(alias = "apply")]
    Restore { name: String },
    /// The copies there are
    List,
    /// Throw a copy away (the layout in use is not touched)
    Delete { name: String },
    /// Give a copy another name
    Rename { name: String, to: String },
    /// Write a copy to a file, to carry it to another machine; the name
    /// `@current` writes the layout as it is now
    Export {
        name: String,
        file: std::path::PathBuf,
    },
    /// Take a copy in from a file such as `export` writes. It is kept
    /// under its own name (with a number if that is taken) and nothing
    /// else changes, unless --apply puts it in force at once
    Import {
        file: std::path::PathBuf,
        /// Make it the layout now (what there was is kept as a copy
        /// called "Before import")
        #[arg(long)]
        apply: bool,
    },
}

/// What `gevrek zones profile` does.
#[derive(Subcommand, Debug, PartialEq)]
pub enum ProfileAction {
    /// Every arrangement that is remembered, and which is in use
    List,
    /// The arrangement in use: its displays, and where each zone is
    Show,
    /// Take over, for the displays as they are now, the arrangement of
    /// another set of displays: by its number in `profile list`
    Copy { number: usize },
}

/// What a word of the command line becomes: the commands to send, in
/// their order, and the sentence that says it was done.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub commands: Vec<Value>,
    pub done: String,
    /// For `create`: what the new zone is given once it is there and its
    /// id is known (`modify_zone`'s changes). Making a zone takes a title,
    /// a kind and a place; the rest of what a zone can have follows.
    pub then_changes: Option<Value>,
}

impl Asked {
    fn one(command: Value, done: impl Into<String>) -> Self {
        Self {
            commands: vec![command],
            done: done.into(),
            then_changes: None,
        }
    }

    fn many(commands: Vec<Value>, done: impl Into<String>) -> Self {
        Self {
            commands,
            done: done.into(),
            then_changes: None,
        }
    }
}

/// The word that stands for the zone that has the keyboard, in any
/// place a zone is named. The zones themselves say which that is.
pub const FOCUSED: &str = "@focused";

/// The word for the bare desktop with its loose items, where the
/// keyboard is given to it or items are moved onto it.
pub const DESK: &str = "@desk";

/// A patch for the zones' settings from `key.path=value ...`: each key
/// is set where the dots lead, to the value read as JSON where it is
/// JSON (a number, true, a list) and as text otherwise; `none` is null,
/// which puts the key back to its default.
pub fn patch(pairs: &[String]) -> Result<Value, String> {
    let mut changes = Value::Object(Map::new());
    for written in pairs {
        let Some((path, value)) = written.split_once('=') else {
            return Err(format!(
                "'{written}' is no setting: write key=value, as snap.grid=16"
            ));
        };
        let value = value.trim();
        let value = if value.eq_ignore_ascii_case("none") {
            Value::Null
        } else {
            serde_json::from_str(value).unwrap_or_else(|_| json!(value))
        };
        let keys: Vec<String> = path
            .split('.')
            .map(|key| key.trim().replace('-', "_"))
            .collect();
        if keys.iter().any(String::is_empty) {
            return Err(format!("'{written}' names no key"));
        }
        let mut at = &mut changes;
        for key in &keys[..keys.len() - 1] {
            let inside = at.as_object_mut().expect("an object, made here");
            let next = inside
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !next.is_object() {
                return Err(format!(
                    "'{written}' goes into '{key}', which was given a value before"
                ));
            }
            at = next;
        }
        at.as_object_mut()
            .expect("an object, made here")
            .insert(keys[keys.len() - 1].clone(), value);
    }
    Ok(changes)
}

/// Whether what an action asks names items, so that the zones are to be
/// fetched with theirs.
pub fn names_items(action: &ZonesAction) -> bool {
    matches!(
        action,
        ZonesAction::Add { .. }
            | ZonesAction::Remove { .. }
            | ZonesAction::Place { .. }
            | ZonesAction::Open { .. }
            | ZonesAction::GoTo { .. }
            | ZonesAction::KeepItem { .. }
            | ZonesAction::Go { .. }
            | ZonesAction::MoveToFolder { .. }
            | ZonesAction::Select { .. }
            | ZonesAction::RenameItem { .. }
            | ZonesAction::Trash { .. }
            // (What a plan leaves where it is, is said by its name.)
            | ZonesAction::AutoCreate { dry_run: true, .. }
    )
}

fn zone_label(zone: &ZoneSnapshot) -> String {
    if zone.title.is_empty() || zone.title == zone.id.as_str() {
        zone.id.to_string()
    } else {
        format!("{} ({})", zone.id, zone.title)
    }
}

/// The zone someone means by `text`: the one with that id; else the one
/// with that title; else the one whose id or title begins so. Letters
/// are compared without regard to case except for the id itself. Two
/// that fit equally well are not chosen between.
pub fn resolve_zone<'a>(
    snapshot: &'a ZonesSnapshot,
    text: &str,
) -> Result<&'a ZoneSnapshot, String> {
    if text == FOCUSED {
        return snapshot
            .zones
            .iter()
            .find(|zone| zone.focused)
            .ok_or_else(|| {
                "No zone has the keyboard (`gevrek zones focus <zone>` gives it to one)".to_string()
            });
    }
    if let Some(zone) = snapshot.zone(text) {
        return Ok(zone);
    }
    let wanted = text.trim().to_lowercase();
    let one_of = |found: Vec<&'a ZoneSnapshot>, how: &str| match found.as_slice() {
        [] => Ok(None),
        [zone] => Ok(Some(*zone)),
        several => Err(format!(
            "'{text}' {how} {} zones: {}. Say which by its id",
            several.len(),
            several
                .iter()
                .map(|zone| zone_label(zone))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    };
    if !wanted.is_empty() {
        let same_id: Vec<&ZoneSnapshot> = snapshot
            .zones
            .iter()
            .filter(|zone| zone.id.as_str().to_lowercase() == wanted)
            .collect();
        if let Some(zone) = one_of(same_id, "is the id of")? {
            return Ok(zone);
        }
        let titled: Vec<&ZoneSnapshot> = snapshot
            .zones
            .iter()
            .filter(|zone| zone.title.to_lowercase() == wanted)
            .collect();
        if let Some(zone) = one_of(titled, "is the title of")? {
            return Ok(zone);
        }
        let begun: Vec<&ZoneSnapshot> = snapshot
            .zones
            .iter()
            .filter(|zone| {
                zone.id.as_str().to_lowercase().starts_with(&wanted)
                    || zone.title.to_lowercase().starts_with(&wanted)
            })
            .collect();
        if let Some(zone) = one_of(begun, "is the beginning of")? {
            return Ok(zone);
        }
    }
    let known: Vec<String> = snapshot
        .zones
        .iter()
        .map(|zone| zone.id.to_string())
        .collect();
    Err(if known.is_empty() {
        format!("There is no zone '{text}': there are no zones")
    } else {
        format!(
            "There is no zone '{text}'. The zones are: {}",
            known.join(", ")
        )
    })
}

/// Every item the snapshot tells of, each once, with the zone it was
/// first met in (none for a loose one).
fn every_item(snapshot: &ZonesSnapshot) -> Vec<(&ItemSnapshot, Option<&ZoneSnapshot>)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    for zone in &snapshot.zones {
        for item in &zone.items {
            if seen.insert(item.id.as_str()) {
                items.push((item, Some(zone)));
            }
        }
    }
    for item in &snapshot.loose {
        if seen.insert(item.id.as_str()) {
            items.push((item, None));
        }
    }
    items
}

/// A name without its extension (`report.pdf` is `report`).
fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// The item someone means by `text`: the one with that id or path; else
/// the one with that name; else the one with that name before its
/// extension; else the one whose name begins so. Two that fit equally
/// well are not chosen between: they are listed with where they are.
pub fn resolve_item<'a>(
    snapshot: &'a ZonesSnapshot,
    text: &str,
) -> Result<&'a ItemSnapshot, String> {
    let items = every_item(snapshot);
    if let Some((item, _)) = items
        .iter()
        .find(|(item, _)| item.id.as_str() == text || item.path.as_deref() == Some(text))
    {
        return Ok(item);
    }
    let wanted = text.trim().to_lowercase();
    // The ways a name can fit, the nearest first.
    type Fits<'a> = Box<dyn Fn(&ItemSnapshot) -> bool + 'a>;
    let ways: [(&str, Fits); 3] = [
        (
            "is the name of",
            Box::new(|item| item.name.to_lowercase() == wanted),
        ),
        (
            "is the name of",
            Box::new(|item| stem(&item.name).to_lowercase() == wanted),
        ),
        (
            "is the beginning of the names of",
            Box::new(|item| item.name.to_lowercase().starts_with(&wanted)),
        ),
    ];
    for (how, fits) in ways.iter().filter(|_| !wanted.is_empty()) {
        let found: Vec<&(&ItemSnapshot, Option<&ZoneSnapshot>)> =
            items.iter().filter(|(item, _)| fits(item)).collect();
        match found.as_slice() {
            [] => continue,
            [(item, _)] => return Ok(item),
            several => {
                let shown: Vec<String> = several
                    .iter()
                    .take(6)
                    .map(|(item, zone)| match zone {
                        Some(zone) => format!("  {}  (in {})", item.id, zone.id),
                        None => format!("  {}  (loose)", item.id),
                    })
                    .collect();
                let more = match several.len().saturating_sub(shown.len()) {
                    0 => String::new(),
                    more => format!("\n  and {more} more"),
                };
                return Err(format!(
                    "'{text}' {how} {} items. Say which by its id:\n{}{more}",
                    several.len(),
                    shown.join("\n")
                ));
            }
        }
    }
    Err(format!(
        "There is no item '{text}' on the desktop or in a zone"
    ))
}

/// Two whole numbers written as `X,Y` (or `WxH`).
pub fn pair(text: &str, what: &str) -> Result<(i32, i32), String> {
    let parts: Vec<&str> = text.split([',', 'x', 'X']).map(str::trim).collect();
    match parts.as_slice() {
        [one, other] => match (one.parse(), other.parse()) {
            (Ok(one), Ok(other)) => Ok((one, other)),
            _ => Err(format!(
                "'{text}' is no {what}: two whole numbers, as 40,40"
            )),
        },
        _ => Err(format!(
            "'{text}' is no {what}: two whole numbers, as 40,40"
        )),
    }
}

/// A size: both sides at least one unit.
fn size(text: &str) -> Result<(i32, i32), String> {
    let (width, height) = pair(text, "size")?;
    if width < 1 || height < 1 {
        return Err(format!("'{text}' is no size: both sides are at least 1"));
    }
    Ok((width, height))
}

/// The display someone means: by its id, by its number (1 is the first),
/// or `primary`; without a word, the primary one (the first where none
/// is).
pub fn resolve_display<'a>(
    snapshot: &'a ZonesSnapshot,
    text: Option<&str>,
) -> Result<&'a gevrek_zones::geometry::Display, String> {
    let displays = &snapshot.displays;
    let primary = || {
        displays
            .iter()
            .find(|display| display.primary)
            .or(displays.first())
    };
    let found = match text.map(str::trim) {
        None | Some("primary") => primary(),
        Some(text) => displays
            .iter()
            .find(|display| display.id.as_str() == text)
            .or_else(|| {
                displays
                    .iter()
                    .find(|display| display.id.as_str().eq_ignore_ascii_case(text))
            })
            .or_else(|| {
                text.parse::<usize>()
                    .ok()
                    .and_then(|number| displays.get(number.checked_sub(1)?))
            }),
    };
    found.ok_or_else(|| {
        if displays.is_empty() {
            "The zones know of no display yet".to_string()
        } else {
            format!(
                "There is no display '{}'. The displays are: {}",
                text.unwrap_or("primary"),
                displays
                    .iter()
                    .map(|display| display.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    })
}

fn on_off(key: &str, value: &str) -> Result<bool, String> {
    match value.to_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Ok(true),
        "off" | "no" | "false" | "0" => Ok(false),
        _ => Err(format!("{key} is on or off, not '{value}'")),
    }
}

/// What `set` changes, sorted by the command that carries it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// For `set_view`: view, sort, descending, icon_size.
    pub view: Map<String, Value>,
    /// For `modify_zone`, under `look`: the look's own keys; null takes
    /// the zone's own value away.
    pub look: Map<String, Value>,
    /// For `modify_zone`: what else a zone has (always_visible, private,
    /// reserves_space).
    pub zone: Map<String, Value>,
}

/// Read `key=value ...` as `set` takes them. Keys are written with
/// dashes or underscores; what is not known is refused with the list of
/// what is.
pub fn settings(pairs: &[String]) -> Result<Settings, String> {
    const KEYS: &str = "view, sort, descending, icon-size, background, opacity, blur, corner-radius, \
                        title-colour, title-on-hover, title-bar, labels, icon-tint, icon-saturation, \
                        always-visible, private, reserves-space";
    let mut settings = Settings::default();
    for written in pairs {
        let Some((key, value)) = written.split_once('=') else {
            return Err(format!(
                "'{written}' is no setting: write key=value (the keys: {KEYS})"
            ));
        };
        let key = key.trim().to_lowercase().replace('_', "-");
        let value = value.trim();
        let word = value.to_lowercase();
        let none = word == "none" || word.is_empty();
        let number = |least: u32, most: u32| -> Result<u32, String> {
            value
                .parse::<u32>()
                .ok()
                .filter(|number| (least..=most).contains(number))
                .ok_or_else(|| format!("{key} is a number from {least} to {most}, not '{value}'"))
        };
        let colour = || -> Result<Value, String> {
            let digits = value.strip_prefix('#').unwrap_or("");
            if matches!(digits.len(), 6 | 8)
                && digits.chars().all(|digit| digit.is_ascii_hexdigit())
            {
                Ok(json!(value))
            } else {
                Err(format!(
                    "{key} is a colour as #rrggbb or #rrggbbaa, not '{value}'"
                ))
            }
        };
        match key.as_str() {
            "view" => match word.as_str() {
                "grid" | "list" => {
                    settings.view.insert("view".into(), json!(word));
                }
                _ => return Err(format!("view is grid or list, not '{value}'")),
            },
            "sort" => match word.as_str() {
                "manual" | "name" | "type" | "date" | "size" => {
                    settings.view.insert("sort".into(), json!(word));
                }
                _ => {
                    return Err(format!(
                        "sort is manual, name, type, date or size, not '{value}'"
                    ))
                }
            },
            "descending" => {
                settings
                    .view
                    .insert("descending".into(), json!(on_off(&key, value)?));
            }
            "icon-size" => {
                let (least, most) = gevrek_model::zones::ICON_SIZES;
                settings
                    .view
                    .insert("icon_size".into(), json!(number(least, most)?));
            }
            "background" | "title-colour" | "title-color" | "icon-tint" => {
                let name = match key.as_str() {
                    "background" => "background",
                    "icon-tint" => "icon_tint",
                    _ => "title_colour",
                };
                settings
                    .look
                    .insert(name.into(), if none { Value::Null } else { colour()? });
            }
            "opacity" | "icon-saturation" => {
                settings.look.insert(
                    key.replace('-', "_"),
                    if none {
                        Value::Null
                    } else {
                        json!(number(0, 100)?)
                    },
                );
            }
            "corner-radius" => {
                let most = gevrek_model::zones::MOST_CORNER_RADIUS;
                settings.look.insert(
                    "corner_radius".into(),
                    if none {
                        Value::Null
                    } else {
                        json!(number(0, most)?)
                    },
                );
            }
            "title-bar" => match word.as_str() {
                "full" | "plain" | "hidden" => {
                    settings.look.insert("title_bar".into(), json!(word));
                }
                _ if none => {
                    settings.look.insert("title_bar".into(), Value::Null);
                }
                _ => return Err(format!("title-bar is full, plain or hidden, not '{value}'")),
            },
            "blur" | "title-on-hover" | "labels" => {
                settings.look.insert(
                    key.replace('-', "_"),
                    if none {
                        Value::Null
                    } else {
                        json!(on_off(&key, value)?)
                    },
                );
            }
            "always-visible" | "private" | "reserves-space" => {
                settings
                    .zone
                    .insert(key.replace('-', "_"), json!(on_off(&key, value)?));
            }
            _ => return Err(format!("'{key}' is nothing `set` knows. The keys: {KEYS}")),
        }
    }
    Ok(settings)
}

/// What a smart zone asks of its items, from how it is written on the
/// command line: the rules' own JSON (a list of matchers, or one), or the
/// short form `ext:pdf,docx+name:report*`.
pub fn query(text: &str) -> Result<Value, String> {
    let text = text.trim();
    if text.starts_with('[') || text.starts_with('{') {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("the query is not JSON: {error}"))?;
        return Ok(if value.is_array() {
            value
        } else {
            json!([value])
        });
    }
    const WORDS: &str = "ext:pdf,docx  name:report*  mime:image/*  kind:folder  target:*chrome*  \
                         untouched:30  older:30  newer:7  larger:1000000  smaller:1000000";
    let mut matchers = Vec::new();
    for part in text
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((word, value)) = part.split_once(':') else {
            return Err(format!(
                "'{part}' is no part of a query. A query is made of: {WORDS}"
            ));
        };
        let value = value.trim();
        let list = || -> Vec<String> {
            value
                .split(',')
                .map(|one| one.trim().trim_start_matches('.').to_lowercase())
                .filter(|one| !one.is_empty())
                .collect()
        };
        let number = || -> Result<u64, String> {
            value
                .parse::<u64>()
                .map_err(|_| format!("'{value}' in '{part}' is no number"))
        };
        matchers.push(match word.trim().to_lowercase().as_str() {
            "ext" | "extension" => json!({"match": "extension", "any_of": list()}),
            "name" => json!({"match": "name", "pattern": value}),
            "mime" => json!({"match": "mime", "pattern": value}),
            "kind" => json!({"match": "kind", "any_of": list()}),
            "target" => json!({"match": "shortcut_target", "pattern": value}),
            "untouched" => json!({"match": "untouched", "days": number()?}),
            "older" => json!({"match": "date", "field": "modified", "older_than_days": number()?}),
            "newer" => json!({"match": "date", "field": "modified", "newer_than_days": number()?}),
            "larger" => json!({"match": "size", "at_least": number()?}),
            "smaller" => json!({"match": "size", "at_most": number()?}),
            _ => {
                return Err(format!(
                    "'{part}' is no part of a query. A query is made of: {WORDS}"
                ))
            }
        });
    }
    if matchers.is_empty() {
        return Err(format!(
            "The query asks for nothing. A query is made of: {WORDS}"
        ));
    }
    Ok(Value::Array(matchers))
}

/// What a smart zone looks through, from its word.
fn source(text: &str) -> Value {
    match text.trim().to_lowercase().replace('_', "-").as_str() {
        "desktop" => json!({"from": "desktop"}),
        "recent" => json!({"from": "recent"}),
        "downloads" => json!({"from": "downloads"}),
        "running-apps" | "running" | "apps" => json!({"from": "running_apps"}),
        "pinned" => json!({"from": "pinned"}),
        _ => json!({"from": "folder", "directory": text.trim()}),
    }
}

/// The command that makes a zone, from what `create` was given.
#[allow(clippy::too_many_arguments)]
fn create(
    snapshot: &ZonesSnapshot,
    title: Option<&str>,
    kind: Option<KindWord>,
    dir: Option<&str>,
    asked: Option<&str>,
    from: Option<&str>,
    display: Option<&str>,
    at: Option<&str>,
    wanted_size: Option<&str>,
) -> Result<(Value, Option<Value>), String> {
    let mut then_changes = None;
    let kind = match (kind, dir, asked.is_some() || from.is_some()) {
        (Some(kind), _, _) => kind,
        (None, Some(_), true) => {
            return Err(
                "--dir makes a portal and --query a smart zone: say one, or --kind".to_string(),
            )
        }
        (None, Some(_), false) => KindWord::Portal,
        (None, None, true) => KindWord::Smart,
        (None, None, false) => KindWord::Manual,
    };
    let mut command = Map::new();
    command.insert("command".into(), json!("create_zone"));
    let kind = match kind {
        KindWord::Manual => {
            if dir.is_some() || asked.is_some() || from.is_some() {
                return Err("A manual zone has no --dir, --query or --from".to_string());
            }
            json!({"type": "manual"})
        }
        KindWord::Portal => {
            let Some(dir) = dir.filter(|dir| !dir.trim().is_empty()) else {
                return Err("A portal needs the folder it shows: --dir <folder>".to_string());
            };
            if asked.is_some() || from.is_some() {
                return Err(
                    "A portal shows all of its folder: it has no --query or --from".to_string(),
                );
            }
            json!({"type": "portal", "directory": dir})
        }
        KindWord::Smart => {
            if dir.is_some() {
                return Err(
                    "A smart zone looks into a folder with --from <folder>, not --dir".to_string(),
                );
            }
            let matchers = match asked {
                Some(asked) => query(asked)?,
                // Of the desktop, a zone that asks nothing shows nothing.
                None if from.is_none() => {
                    return Err("A smart zone needs what it shows: --query, or --from".to_string())
                }
                None => json!([]),
            };
            if let Some(from) = from {
                then_changes = Some(json!({"source": source(from)}));
            }
            json!({"type": "smart", "query": matchers})
        }
    };
    let title = match title {
        Some(title) => title.to_string(),
        None => match (&kind["type"], dir) {
            // A portal is called as its folder is.
            (_, Some(dir)) => dir
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(dir)
                .to_string(),
            _ => "Zone".to_string(),
        },
    };
    command.insert("title".into(), json!(title));
    command.insert("kind".into(), kind);
    match (at, wanted_size) {
        // Nothing said of where: the zones put it beside the ones there are.
        (None, None) if display.is_none() => {}
        (at, wanted_size) => {
            let display = resolve_display(snapshot, display)?;
            // Without a place: a little in from the display's corner.
            let (x, y) = at
                .map(|at| pair(at, "place"))
                .transpose()?
                .unwrap_or((40, 40));
            let (width, height) = wanted_size.map(size).transpose()?.unwrap_or((320, 240));
            command.insert(
                "rect".into(),
                json!({"x": display.rect.x + x, "y": display.rect.y + y, "width": width, "height": height}),
            );
        }
    }
    Ok((Value::Object(command), then_changes))
}

/// The commands an action becomes, given the zones as they were last
/// told. `Err` is what is wrong with what was asked (a zone that is not
/// there, a name two items have), said before anything is sent.
///
/// The actions that only read (`list`, `show <zone>`, `rules list`,
/// `events`, ...) and the ones that work with files (`snapshot export`,
/// `import`) are not here: the command line carries those out itself.
pub fn asked(action: &ZonesAction, snapshot: &ZonesSnapshot) -> Result<Asked, String> {
    let zone = |text: &str| resolve_zone(snapshot, text);
    let item = |text: &str| resolve_item(snapshot, text);
    let items_of = |texts: &[String]| -> Result<Vec<&ItemSnapshot>, String> {
        texts.iter().map(|text| item(text)).collect()
    };
    let counted = |count: usize| {
        if count == 1 {
            "1 item".to_string()
        } else {
            format!("{count} items")
        }
    };
    Ok(match action {
        ZonesAction::Create { title, kind, dir, query, from, display, at, size } => {
            let (command, then_changes) = create(
                snapshot,
                title.as_deref(),
                *kind,
                dir.as_deref(),
                query.as_deref(),
                from.as_deref(),
                display.as_deref(),
                at.as_deref(),
                size.as_deref(),
            )?;
            Asked { commands: vec![command], done: "The zone was made".to_string(), then_changes }
        }
        ZonesAction::Delete { zone: text } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "delete_zone", "zone": zone.id}),
                format!("Zone '{}' was taken away; what it held is loose on the desktop", zone.id),
            )
        }
        ZonesAction::Rename { zone: text, title } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "rename_zone", "zone": zone.id, "title": title}),
                format!("Zone '{}' is called \"{title}\" now", zone.id),
            )
        }
        ZonesAction::Move { zone: text, to, display, no_snap } => {
            let zone = zone(text)?;
            let (mut x, mut y) = pair(to, "place")?;
            if display.is_some() {
                let display = resolve_display(snapshot, display.as_deref())?;
                x += display.rect.x;
                y += display.rect.y;
            }
            Asked::one(
                json!({"command": "move_zone", "zone": zone.id, "x": x, "y": y, "snap": !no_snap}),
                format!("Zone '{}' was moved", zone.id),
            )
        }
        ZonesAction::Resize { zone: text, size: wanted, no_snap } => {
            let zone = zone(text)?;
            let (width, height) = size(wanted)?;
            let Some(rect) = zone.rect else {
                return Err(format!(
                    "Zone '{}' has no place on the displays as they are now, so it cannot be resized",
                    zone.id
                ));
            };
            Asked::one(
                json!({
                    "command": "resize_zone",
                    "zone": zone.id,
                    "rect": {"x": rect.x, "y": rect.y, "width": width, "height": height},
                    "edges": {"right": true, "bottom": true},
                    "snap": !no_snap,
                }),
                format!("Zone '{}' was resized", zone.id),
            )
        }
        ZonesAction::Set { zone: text, settings: pairs } => {
            let zone = zone(text)?;
            let settings = settings(pairs)?;
            let mut commands = Vec::new();
            let mut view = settings.view;
            if !view.is_empty() {
                view.insert("command".into(), json!("set_view"));
                view.insert("zone".into(), json!(zone.id));
                commands.push(Value::Object(view));
            }
            // Everything else is the zone's configuration, changed by a
            // patch: what is named is set, null puts it back.
            let mut changes = settings.zone;
            if !settings.look.is_empty() {
                changes.insert("look".into(), Value::Object(settings.look));
            }
            if !changes.is_empty() {
                commands.push(json!({"command": "modify_zone", "zone": zone.id, "changes": changes}));
            }
            Asked::many(commands, format!("Zone '{}' was changed", zone.id))
        }
        ZonesAction::RollUp { zone: text, to } => {
            let zone = zone(text)?;
            let mut command = json!({"command": "roll_up", "zone": zone.id});
            if let Some(value) = to.value() {
                command["rolled_up"] = json!(value);
            }
            let done = match to.from(zone.rolled_up) {
                true => format!("Zone '{}' is rolled up", zone.id),
                false => format!("Zone '{}' is rolled down", zone.id),
            };
            Asked::one(command, done)
        }
        ZonesAction::Lock { zone: text, what } => {
            let zone = zone(text)?;
            let (command, done) = match what {
                LockWhat::Geometry => (json!({"geometry": true}), "its place is locked"),
                LockWhat::Members => (json!({"members": true}), "what it holds is locked"),
                LockWhat::All => (json!({"geometry": true, "members": true}), "it is locked"),
                LockWhat::None => (json!({"geometry": false, "members": false}), "it is unlocked"),
            };
            let mut command = command;
            command["command"] = json!("lock");
            command["zone"] = json!(zone.id);
            Asked::one(command, format!("Zone '{}': {done}", zone.id))
        }
        ZonesAction::Unlock { zone: text } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "lock", "zone": zone.id, "geometry": false, "members": false}),
                format!("Zone '{}': it is unlocked", zone.id),
            )
        }
        ZonesAction::Tab { zone: text, with } => {
            let (zone, with) = (zone(text)?, zone(with)?);
            if zone.id == with.id {
                return Err(format!("Zone '{}' cannot be a tab beside itself", zone.id));
            }
            Asked::one(
                json!({"command": "tab", "zone": zone.id, "with": with.id}),
                format!("Zone '{}' is a tab beside '{}'", zone.id, with.id),
            )
        }
        ZonesAction::Untab { zone: text } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "untab", "zone": zone.id}),
                format!("Zone '{}' stands by itself", zone.id),
            )
        }
        ZonesAction::Front { zone: text } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "activate_tab", "zone": zone.id}),
                format!("Zone '{}' is the tab in front", zone.id),
            )
        }
        ZonesAction::Add { zone: text, items, index } => {
            let zone = zone(text)?;
            let items = items_of(items)?;
            let commands = items
                .iter()
                .enumerate()
                .map(|(place, item)| {
                    let mut command = json!({"command": "add_item", "zone": zone.id, "item": item.id});
                    if let Some(index) = index {
                        command["index"] = json!(index + place);
                    }
                    command
                })
                .collect();
            Asked::many(commands, format!("{} put into zone '{}'", counted(items.len()), zone.id))
        }
        ZonesAction::Remove { items } => {
            let items = items_of(items)?;
            Asked::many(
                items
                    .iter()
                    .map(|item| json!({"command": "remove_item", "item": item.id}))
                    .collect(),
                format!("{} taken out; loose on the desktop", counted(items.len())),
            )
        }
        ZonesAction::Place { item: text, at, display } => {
            let item = item(text)?;
            let (x, y) = pair(at, "place")?;
            let display = resolve_display(snapshot, display.as_deref())?;
            Asked::one(
                json!({"command": "place_loose_item", "item": item.id, "display": display.id, "x": x, "y": y}),
                format!("'{}' was put on the desktop", item.name),
            )
        }
        ZonesAction::Open { items } => {
            let items = items_of(items)?;
            let ids: Vec<&gevrek_zones::ItemId> = items.iter().map(|item| &item.id).collect();
            let done = match items.as_slice() {
                [item] => format!("'{}' was opened", item.name),
                items => format!("{} opened", counted(items.len())),
            };
            Asked::one(json!({"command": "open", "items": ids}), done)
        }
        ZonesAction::GoTo { item: text } => {
            let item = item(text)?;
            Asked::one(
                json!({"command": "focus_item", "item": item.id}),
                format!("'{}' has the keyboard", item.name),
            )
        }
        ZonesAction::NextTab { zone: text, backwards } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "next_tab", "zone": zone.id, "backwards": backwards}),
                format!("The {} tab beside '{}' is in front", if *backwards { "previous" } else { "next" }, zone.id),
            )
        }
        ZonesAction::LockAll { what } => {
            let (command, done) = match what {
                LockWhat::Geometry => (json!({"geometry": true}), "No zone can be moved or resized"),
                LockWhat::Members => (json!({"members": true}), "Nothing can be put into a zone or taken out"),
                LockWhat::All => (json!({"geometry": true, "members": true}), "Every zone is locked"),
                LockWhat::None => (
                    json!({"geometry": false, "members": false}),
                    "The lock on all zones is off (each zone's own lock stays)",
                ),
            };
            let mut command = command;
            command["command"] = json!("lock_all");
            Asked::one(command, done)
        }
        ZonesAction::KeepZone { zone: text, to } => {
            let zone = zone(text)?;
            let mut command = json!({"command": "always_visible", "zone": zone.id});
            if let Some(value) = to.value() {
                command["on"] = json!(value);
            }
            let done = match to.from(zone.always_visible) {
                true => format!("Zone '{}' stays when the zones are hidden", zone.id),
                false => format!("Zone '{}' is hidden with the others", zone.id),
            };
            Asked::one(command, done)
        }
        ZonesAction::KeepItem { item: text, to } => {
            let item = item(text)?;
            let mut command = json!({"command": "item_always_visible", "item": item.id});
            if let Some(value) = to.value() {
                command["on"] = json!(value);
            }
            let done = match to {
                Switch::On => format!("'{}' stays when the zones are hidden", item.name),
                Switch::Off => format!("'{}' is hidden with the others", item.name),
                Switch::Toggle => format!("'{}' stays when the zones are hidden, or no longer does", item.name),
            };
            Asked::one(command, done)
        }
        ZonesAction::Go { zone: text, to } => {
            let zone = zone(text)?;
            if zone.portal.is_none() {
                return Err(format!("Zone '{}' is no portal: only a portal goes into folders", zone.id));
            }
            match to.trim().to_lowercase().as_str() {
                way @ ("up" | "back" | "forward") => Asked::one(
                    json!({"command": format!("navigate_{way}"), "zone": zone.id}),
                    format!("Zone '{}' went {way}", zone.id),
                ),
                _ => {
                    // A folder the portal shows now, by its name or id.
                    let wanted = to.trim().to_lowercase();
                    let folder = zone
                        .items
                        .iter()
                        .find(|item| item.id.as_str() == to || item.path.as_deref() == Some(to.as_str()))
                        .or_else(|| zone.items.iter().find(|item| item.name.to_lowercase() == wanted))
                        .ok_or_else(|| {
                            format!("Zone '{}' shows nothing called '{to}' (or say up, back or forward)", zone.id)
                        })?;
                    Asked::one(
                        json!({"command": "enter_folder", "zone": zone.id, "item": folder.id}),
                        format!("Zone '{}' shows '{}' now", zone.id, folder.name),
                    )
                }
            }
        }
        ZonesAction::Refresh { zone: text } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "refresh_zone", "zone": zone.id}),
                format!("Zone '{}' was asked again for what it holds", zone.id),
            )
        }
        // With --in, the one word that is given is the folder's name.
        ZonesAction::NewFolder { zone: Some(name), name: None, folder: Some(folder) } => Asked::one(
            json!({"command": "new_folder_in", "folder": folder, "name": name}),
            format!("The folder '{name}' was made in {folder} (`gevrek zones undo` takes it back)"),
        ),
        ZonesAction::NewFolder { zone: Some(text), name: Some(name), folder: None } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "new_folder", "zone": zone.id, "name": name}),
                format!("The folder '{name}' was made in zone '{}' (`gevrek zones undo` takes it back)", zone.id),
            )
        }
        ZonesAction::NewFolder { .. } => {
            return Err(
                "Say where and what it is called: `new-folder <portal> <name>`, or `new-folder --in <folder> <name>`"
                    .to_string(),
            )
        }
        ZonesAction::MoveToFolder { items, folder, copy } => {
            let items = items_of(items)?;
            let ids: Vec<&gevrek_zones::ItemId> = items.iter().map(|item| &item.id).collect();
            Asked::one(
                json!({"command": "move_items_to_folder", "items": ids, "folder": folder, "copy": copy}),
                format!(
                    "{} {} to {folder} (`gevrek zones undo` takes it back)",
                    counted(items.len()),
                    if *copy { "copied" } else { "moved" }
                ),
            )
        }
        ZonesAction::BringIn { paths, folder, zone: into, copy } => {
            let mut command = json!({"command": "bring_in", "paths": paths, "folder": folder, "copy": copy});
            if let Some(text) = into {
                command["zone"] = json!(zone(text)?.id);
            }
            Asked::one(
                command,
                format!(
                    "{} {} to {folder}",
                    if paths.len() == 1 { "1 file".to_string() } else { format!("{} files", paths.len()) },
                    if *copy { "copied" } else { "brought" }
                ),
            )
        }
        ZonesAction::Cancel { request: Some(request) } => Asked::one(
            json!({"command": "cancel_operation", "request": request}),
            format!("Operation {request} was called off; what it had done is put back as far as that can be"),
        ),
        ZonesAction::Expand { zone: text, to } => {
            let zone = zone(text)?;
            let mut command = json!({"command": "expand", "zone": zone.id});
            if let Some(value) = to.value() {
                command["expanded"] = json!(value);
            }
            let done = match to.from(zone.expanded) {
                true => format!("Zone '{}' is open for now", zone.id),
                false => format!("Zone '{}' is closed again", zone.id),
            };
            Asked::one(command, done)
        }
        ZonesAction::MoveTab { zone: text, index } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "move_tab", "zone": zone.id, "index": index}),
                format!("Tab '{}' is at place {index} of its row", zone.id),
            )
        }
        ZonesAction::SplitTab { zone: text, to, no_snap } => {
            let zone = zone(text)?;
            let (x, y) = pair(to, "place")?;
            Asked::one(
                json!({"command": "split_tab", "zone": zone.id, "x": x, "y": y, "snap": !no_snap}),
                format!("Zone '{}' stands by itself", zone.id),
            )
        }
        ZonesAction::Settings { settings } => Asked::one(
            json!({"command": "modify_settings", "changes": patch(settings)?}),
            "The zones' settings were changed",
        ),
        ZonesAction::Select { items, mode } => {
            let mode = mode.trim().to_lowercase();
            if !matches!(mode.as_str(), "replace" | "add" | "remove" | "toggle") {
                return Err(format!("--mode is replace, add, remove or toggle, not '{mode}'"));
            }
            let items = items_of(items)?;
            let ids: Vec<&gevrek_zones::ItemId> = items.iter().map(|item| &item.id).collect();
            Asked::one(
                json!({"command": "select", "items": ids, "mode": mode}),
                match (mode.as_str(), items.len()) {
                    ("replace", 0) => "Nothing is chosen".to_string(),
                    ("replace", count) => format!("{} chosen", counted(count)),
                    (_, count) => format!("{}: the choice was changed", counted(count)),
                },
            )
        }
        ZonesAction::Walk { step, extend } => {
            let step = step.trim().to_lowercase();
            if !matches!(step.as_str(), "left" | "right" | "up" | "down" | "first" | "last") {
                return Err(format!("A step is left, right, up, down, first or last, not '{step}'"));
            }
            Asked::one(
                json!({"command": "move_item_focus", "step": step, "extend": extend}),
                "The keyboard went on",
            )
        }
        ZonesAction::OpenChosen => Asked::one(json!({"command": "open_focused"}), "What was chosen was opened"),
        ZonesAction::MoveChosen { target } => match target.trim().to_lowercase().as_str() {
            direction @ ("left" | "right" | "up" | "down") => Asked::one(
                json!({"command": "move_focused_items", "direction": direction}),
                "The chosen items were moved",
            ),
            "desk" | DESK => Asked::one(
                json!({"command": "move_focused_items", "zone": DESK}),
                "The chosen items are loose on the desktop",
            ),
            _ => {
                let zone = zone(target)?;
                Asked::one(
                    json!({"command": "move_focused_items", "zone": zone.id}),
                    format!("The chosen items were moved into zone '{}'", zone.id),
                )
            }
        },
        ZonesAction::Dismiss => Asked::one(json!({"command": "dismiss"}), "Stepped back"),
        ZonesAction::Filter { zone: text, text: typed } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "filter", "zone": zone.id, "typed": typed}),
                if typed.is_empty() {
                    format!("Zone '{}' shows all it holds", zone.id)
                } else {
                    format!("Zone '{}' shows what fits '{typed}'", zone.id)
                },
            )
        }
        ZonesAction::RenameItem { item: text, name } => {
            let item = item(text)?;
            Asked::one(
                json!({"command": "rename_item", "item": item.id, "name": name}),
                format!("'{}' is called '{name}' now", item.name),
            )
        }
        ZonesAction::Trash { items } => {
            let items = items_of(items)?;
            let ids: Vec<&gevrek_zones::ItemId> = items.iter().map(|item| &item.id).collect();
            Asked::one(
                json!({"command": "trash_items", "items": ids}),
                format!("{} put into the bin (`gevrek zones undo` takes it back)", counted(items.len())),
            )
        }
        ZonesAction::Undo => Asked::one(
            json!({"command": "undo_file_operation"}),
            "The last file operation was taken back",
        ),
        ZonesAction::Hide { zone: Some(text) } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "show_zone", "zone": zone.id, "shown": false}),
                format!("Zone '{}' is hidden on these displays", zone.id),
            )
        }
        ZonesAction::Unhide { zone: Some(text) } => {
            let zone = zone(text)?;
            Asked::one(
                json!({"command": "show_zone", "zone": zone.id, "shown": true}),
                format!("Zone '{}' is shown again", zone.id),
            )
        }
        ZonesAction::Hide { zone: None } => {
            Asked::one(json!({"command": "quick_hide", "hidden": true}), "The zones are hidden")
        }
        ZonesAction::Unhide { zone: None } | ZonesAction::Show { zone: None } => {
            Asked::one(json!({"command": "quick_hide", "hidden": false}), "The zones are shown")
        }
        ZonesAction::Toggle => Asked::one(
            json!({"command": "quick_hide"}),
            if snapshot.quick_hidden { "The zones are shown" } else { "The zones are hidden" },
        ),
        ZonesAction::Peek { to } => {
            let on = to.from(snapshot.peeking);
            Asked::one(
                match to.value() {
                    Some(on) => json!({"command": "peek", "on": on}),
                    None => json!({"command": "toggle_peek"}),
                },
                if on { "The zones are above the windows" } else { "The zones are back below the windows" },
            )
        }
        ZonesAction::Grey { to } => {
            let mut command = json!({"command": "grey_icons"});
            if let Some(value) = to.value() {
                command["on"] = json!(value);
            }
            Asked::one(
                command,
                if to.from(snapshot.grey_icons) {
                    "The icons are drawn grey"
                } else {
                    "The icons are drawn in their colours"
                },
            )
        }
        ZonesAction::AutoCreate { dry_run, scope, min_items, no_rules, display } => {
            let mut command = json!({"command": "auto_create", "scope": scope.word(), "dry_run": dry_run});
            if let Some(least) = min_items {
                if *least == 0 {
                    return Err("--min-items is at least 1: a zone is made for something".to_string());
                }
                command["min_items"] = json!(least);
            }
            if *no_rules {
                command["add_rules"] = json!(false);
            }
            if display.is_some() {
                command["display"] = json!(resolve_display(snapshot, display.as_deref())?.id);
            }
            // (What is said of it is made from the zones' answer.)
            Asked::one(command, String::new())
        }
        ZonesAction::AutoUndo => Asked::one(json!({"command": "undo_auto_create"}), String::new()),
        ZonesAction::Privacy { to, zone: Some(text) } => {
            let zone = zone(text)?;
            // Covered: the zone is private and not revealed. A zone that
            // is not private is made so; one that is, is covered again.
            let cover = to.from(zone.masked);
            let command = match (cover, zone.private) {
                (true, false) => json!({"command": "modify_zone", "zone": zone.id, "changes": {"private": true}}),
                (cover, _) => json!({"command": "reveal", "zone": zone.id, "revealed": !cover}),
            };
            Asked::one(
                command,
                format!("Zone '{}': what it holds is {}", zone.id, if cover { "covered" } else { "uncovered" }),
            )
        }
        ZonesAction::Privacy { to, zone: None } => match to.from(snapshot.masked) {
            true => Asked::one(json!({"command": "mask", "masked": true}), "What the zones hold is covered"),
            // (Ends the cover by hand, and shows the private zones too.)
            false => Asked::one(json!({"command": "reveal"}), "What the zones hold is uncovered"),
        },
        ZonesAction::Rules { action } => match action {
            RulesAction::DryRun { scope } => Asked::one(
                json!({"command": "preview_rules", "scope": scope.word()}),
                String::new(),
            ),
            RulesAction::Apply { scope } => Asked::one(
                json!({"command": "apply_rules", "scope": scope.word()}),
                String::new(),
            ),
            RulesAction::Undo => Asked::one(
                json!({"command": "undo_rules"}),
                "The last application of the rules was taken back",
            ),
            RulesAction::Check => {
                Asked::one(json!({"command": "check_timed_rules"}), String::new())
            }
            RulesAction::List => return Err("`rules list` only reads".to_string()),
        },
        ZonesAction::Snapshot { action } => match action {
            SnapshotAction::Save { name } => Asked::one(
                json!({"command": "save_snapshot", "name": name}),
                format!("The layout was saved as '{name}'"),
            ),
            SnapshotAction::Restore { name } => Asked::one(
                json!({"command": "restore_snapshot", "name": name}),
                format!("The layout is what it was in '{name}'"),
            ),
            SnapshotAction::Delete { name } => Asked::one(
                json!({"command": "delete_snapshot", "name": name}),
                format!("The copy '{name}' was deleted"),
            ),
            SnapshotAction::Rename { name, to } => Asked::one(
                json!({"command": "rename_snapshot", "name": name, "to": to}),
                format!("The copy '{name}' is called '{to}' now"),
            ),
            SnapshotAction::Export { name, .. } => Asked::one(
                if name == "@current" {
                    json!({"command": "export_layout"})
                } else {
                    json!({"command": "export_layout", "name": name})
                },
                String::new(),
            ),
            SnapshotAction::List | SnapshotAction::Import { .. } => {
                return Err("not a command of its own".to_string())
            }
        },
        ZonesAction::Focus { target } => match target.trim().to_lowercase().as_str() {
            direction @ ("left" | "right" | "up" | "down") => Asked::one(
                json!({"command": "focus_neighbour", "direction": direction}),
                match direction {
                    "up" => "The zone above has the keyboard".to_string(),
                    "down" => "The zone below has the keyboard".to_string(),
                    side => format!("The zone to the {side} has the keyboard"),
                },
            ),
            "none" => Asked::one(json!({"command": "focus_zone"}), "No zone has the keyboard"),
            "desk" | DESK => Asked::one(
                json!({"command": "focus_zone", "zone": DESK}),
                "The bare desktop has the keyboard",
            ),
            _ => {
                let zone = zone(target)?;
                Asked::one(
                    json!({"command": "focus_zone", "zone": zone.id}),
                    format!("Zone '{}' has the keyboard", zone.id),
                )
            }
        },
        ZonesAction::Profile { action: ProfileAction::Copy { number } } => {
            let profile = number
                .checked_sub(1)
                .and_then(|index| snapshot.profiles.get(index))
                .ok_or_else(|| {
                    format!(
                        "There is no arrangement {number}: `gevrek zones profile list` numbers the {} there are",
                        snapshot.profiles.len()
                    )
                })?;
            if profile.current {
                return Err(format!("Arrangement {number} is the one in use"));
            }
            Asked::one(
                json!({"command": "copy_arrangement", "from": profile.key}),
                format!("The zones are placed as in arrangement {number}"),
            )
        }
        ZonesAction::Search { text } => {
            Asked::one(json!({"command": "search", "typed": text}), String::new())
        }
        ZonesAction::RestoreExplorer => Asked::one(
            json!({"command": gevrek_zones::ipc::RESTORE_SYSTEM_ICONS}),
            "The system's own desktop icons were put back",
        ),
        ZonesAction::List { .. }
        | ZonesAction::Cancel { request: None }
        | ZonesAction::Show { zone: Some(_) }
        | ZonesAction::Profile { .. }
        | ZonesAction::Events
        | ZonesAction::Send { .. } => return Err("not a command of its own".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::geometry::Display;
    use gevrek_zones::ipc::Request;
    use gevrek_zones::{ItemId, Rect, ZoneId};

    /// The command line's words, read as `gevrek zones <words>` reads them.
    #[derive(clap::Parser, Debug)]
    struct Line {
        #[command(subcommand)]
        action: ZonesAction,
    }

    fn read(words: &[&str]) -> Result<ZonesAction, clap::Error> {
        use clap::Parser;
        Line::try_parse_from(std::iter::once("zones").chain(words.iter().copied()))
            .map(|line| line.action)
    }

    fn file(path: &str) -> ItemSnapshot {
        ItemSnapshot {
            id: ItemId::new(path),
            name: path.rsplit('/').next().unwrap().to_string(),
            path: Some(path.to_string()),
            ..ItemSnapshot::default()
        }
    }

    fn zone(id: &str, title: &str, at: (i32, i32), items: &[&str]) -> ZoneSnapshot {
        ZoneSnapshot {
            id: ZoneId::new(id),
            title: title.to_string(),
            rect: Some(Rect::new(at.0, at.1, 320, 240)),
            visible: true,
            front: true,
            item_count: items.len(),
            items: items.iter().map(|path| file(path)).collect(),
            ..ZoneSnapshot::default()
        }
    }

    /// Three zones on two displays, with items in two of them and loose.
    fn desk() -> ZonesSnapshot {
        ZonesSnapshot {
            enabled: true,
            displays: vec![
                Display { primary: true, ..Display::new("DEL-A1B2-77", Rect::new(0, 0, 1920, 1080)) },
                Display::new("LG-0042", Rect::new(1920, 0, 2560, 1440)),
            ],
            zones: vec![
                zone("work", "Work", (40, 40), &["/desk/report.pdf", "/desk/Budget 2026.xlsx"]),
                ZoneSnapshot { focused: true, ..zone("workshop", "Wood", (400, 40), &["/desk/plans.pdf"]) },
                zone("zone-3", "Play", (40, 320), &[]),
                ZoneSnapshot {
                    kind: gevrek_zones::config::ZoneKind::Portal { directory: "/home/ada/Downloads".to_string() },
                    portal: serde_json::from_value(json!({
                        "root": "/home/ada/Downloads", "directory": "/home/ada/Downloads",
                        "crumbs": [{"name": "Downloads", "directory": "/home/ada/Downloads"}],
                        "can_go_up": false, "can_go_back": false, "can_go_forward": false,
                        "listing": {"state": "ready"},
                    }))
                    .unwrap(),
                    ..zone("dl", "Downloads", (800, 40), &["/home/ada/Downloads/2026", "/home/ada/Downloads/setup.exe"])
                },
                ZoneSnapshot { private: true, masked: true, ..zone("bank", "Bank", (800, 320), &[]) },
            ],
            profiles: serde_json::from_value(json!([
                {"key": {"displays": [
                    {"id": "DEL-A1B2-77", "width": 1920, "height": 1080, "scale": 1000},
                    {"id": "LG-0042", "width": 2560, "height": 1440, "scale": 1000}]},
                 "zones": 5, "current": true},
                {"key": {"displays": [{"id": "DEL-A1B2-77", "width": 1920, "height": 1080, "scale": 1000}]},
                 "zones": 3, "current": false},
            ]))
            .unwrap(),
            loose: vec![file("/desk/notes.txt"), file("/desk/report.txt"), file("/desk/photo.png")],
            ..ZonesSnapshot::default()
        }
    }

    /// What the words become, as the commands' JSON.
    fn commands(words: &[&str]) -> Vec<String> {
        let action = read(words).unwrap_or_else(|error| panic!("{words:?}: {error}"));
        asked(&action, &desk())
            .unwrap_or_else(|why| panic!("{words:?}: {why}"))
            .commands
            .iter()
            .map(Value::to_string)
            .collect()
    }

    fn refusal(words: &[&str]) -> String {
        asked(&read(words).unwrap(), &desk()).unwrap_err()
    }

    #[test]
    fn a_zone_is_named_by_its_id_its_title_or_the_beginning_of_either() {
        let desk = desk();
        let found = |text: &str| resolve_zone(&desk, text).map(|zone| zone.id.to_string());
        assert_eq!(
            found("work").unwrap(),
            "work",
            "the id itself, though another begins so"
        );
        assert_eq!(found("WORKSHOP").unwrap(), "workshop");
        assert_eq!(found("wood").unwrap(), "workshop", "by its title");
        assert_eq!(
            found("pl").unwrap(),
            "zone-3",
            "by the beginning of its title"
        );
        assert_eq!(
            found("zone").unwrap(),
            "zone-3",
            "by the beginning of its id"
        );
        let two = found("wo").unwrap_err();
        assert!(
            two.starts_with("'wo' is the beginning of 2 zones: work (Work), workshop (Wood)"),
            "{two}"
        );
        assert_eq!(
            found("games").unwrap_err(),
            "There is no zone 'games'. The zones are: work, workshop, zone-3, dl, bank"
        );
        assert_eq!(
            resolve_zone(&ZonesSnapshot::default(), "work").unwrap_err(),
            "There is no zone 'work': there are no zones"
        );
    }

    #[test]
    fn an_item_is_named_by_its_id_its_name_or_the_beginning_of_its_name() {
        let desk = desk();
        let found = |text: &str| resolve_item(&desk, text).map(|item| item.id.to_string());
        assert_eq!(found("/desk/notes.txt").unwrap(), "/desk/notes.txt");
        assert_eq!(found("Report.PDF").unwrap(), "/desk/report.pdf");
        assert_eq!(
            found("notes").unwrap(),
            "/desk/notes.txt",
            "without its extension"
        );
        assert_eq!(
            found("budget").unwrap(),
            "/desk/Budget 2026.xlsx",
            "by its beginning"
        );
        // `report` is the name of two before their extensions: neither.
        let two = found("report").unwrap_err();
        assert!(
            two.starts_with("'report' is the name of 2 items. Say which by its id:"),
            "{two}"
        );
        assert!(two.contains("  /desk/report.pdf  (in work)"), "{two}");
        assert!(two.contains("  /desk/report.txt  (loose)"), "{two}");
        let two = found("p").unwrap_err();
        assert!(
            two.contains("is the beginning of the names of 2 items"),
            "{two}"
        );
        // What a portal shows is named as everything else is.
        assert_eq!(found("setup").unwrap(), "/home/ada/Downloads/setup.exe");
        assert_eq!(
            found("nothing.bin").unwrap_err(),
            "There is no item 'nothing.bin' on the desktop or in a zone"
        );
    }

    #[test]
    fn a_display_is_named_by_its_id_its_number_or_as_the_primary_one() {
        let desk = desk();
        let found =
            |text: Option<&str>| resolve_display(&desk, text).map(|display| display.id.to_string());
        assert_eq!(found(None).unwrap(), "DEL-A1B2-77");
        assert_eq!(found(Some("primary")).unwrap(), "DEL-A1B2-77");
        assert_eq!(found(Some("2")).unwrap(), "LG-0042");
        assert_eq!(found(Some("lg-0042")).unwrap(), "LG-0042");
        assert_eq!(
            found(Some("3")).unwrap_err(),
            "There is no display '3'. The displays are: DEL-A1B2-77, LG-0042"
        );
        assert_eq!(
            resolve_display(&ZonesSnapshot::default(), None).unwrap_err(),
            "The zones know of no display yet"
        );
    }

    #[test]
    fn places_and_sizes_are_two_whole_numbers() {
        assert_eq!(pair("40,60", "place").unwrap(), (40, 60));
        assert_eq!(pair(" -10 , 20 ", "place").unwrap(), (-10, 20));
        assert_eq!(pair("320x240", "size").unwrap(), (320, 240));
        assert!(pair("40", "place").unwrap_err().contains("is no place"));
        assert!(pair("a,b", "size").unwrap_err().contains("is no size"));
        assert!(size("0,10").unwrap_err().contains("at least 1"));
    }

    #[test]
    fn settings_are_sorted_by_the_command_that_carries_them() {
        let pairs = |words: &[&str]| {
            settings(
                &words
                    .iter()
                    .map(|word| word.to_string())
                    .collect::<Vec<_>>(),
            )
        };
        let read = pairs(&[
            "view=list",
            "sort=name",
            "descending=on",
            "icon-size=32",
            "opacity=80",
            "blur=off",
            "background=#102030",
            "title_colour=#ffffffcc",
            "corner-radius=none",
            "always-visible=yes",
            "private=off",
        ])
        .unwrap();
        assert_eq!(
            Value::Object(read.view).to_string(),
            r#"{"view":"list","sort":"name","descending":true,"icon_size":32}"#
        );
        assert_eq!(
            Value::Object(read.look).to_string(),
            r##"{"opacity":80,"blur":false,"background":"#102030","title_colour":"#ffffffcc","corner_radius":null}"##
        );
        assert_eq!(
            Value::Object(read.zone).to_string(),
            r#"{"always_visible":true,"private":false}"#
        );
        for (wrong, says) in [
            ("view", "write key=value"),
            ("view=tiles", "view is grid or list"),
            ("sort=age", "sort is manual, name, type, date or size"),
            ("icon-size=8", "icon-size is a number from 16 to 256"),
            ("opacity=140", "opacity is a number from 0 to 100"),
            ("background=blue", "a colour as #rrggbb"),
            ("blur=maybe", "blur is on or off"),
            ("glow=on", "'glow' is nothing `set` knows"),
        ] {
            let why = pairs(&[wrong]).unwrap_err();
            assert!(why.contains(says), "{wrong}: {why}");
        }
    }

    #[test]
    fn a_query_is_the_rules_own_json_or_its_short_form() {
        assert_eq!(
            query("ext:pdf,.DOCX + name:report* + untouched:30")
                .unwrap()
                .to_string(),
            r#"[{"match":"extension","any_of":["pdf","docx"]},{"match":"name","pattern":"report*"},{"match":"untouched","days":30}]"#
        );
        assert_eq!(
            query("mime:image/*+larger:1000+newer:7")
                .unwrap()
                .to_string(),
            r#"[{"match":"mime","pattern":"image/*"},{"match":"size","at_least":1000},{"match":"date","field":"modified","newer_than_days":7}]"#
        );
        // The rules' own words, one matcher or a list of them.
        assert_eq!(
            query(r#"{"match":"kind","any_of":["folder"]}"#)
                .unwrap()
                .to_string(),
            r#"[{"match":"kind","any_of":["folder"]}]"#
        );
        // Each is a list of matchers the configuration reads.
        for text in [
            "ext:pdf",
            "kind:folder,shortcut",
            "target:*chrome*",
            "older:30+smaller:5",
        ] {
            let matchers = query(text).unwrap();
            serde_json::from_value::<Vec<gevrek_model::zones::Matcher>>(matchers)
                .unwrap_or_else(|error| panic!("{text}: {error}"));
        }
        assert!(query("pdf").unwrap_err().contains("is no part of a query"));
        assert!(query("older:soon").unwrap_err().contains("is no number"));
        assert!(query(" + ").unwrap_err().contains("asks for nothing"));
        assert!(query("[1").unwrap_err().contains("not JSON"));
    }

    #[test]
    fn every_word_becomes_the_commands_the_zones_read() {
        for (words, expected) in [
            (
                &["delete", "Wood"][..],
                &[r#"{"command":"delete_zone","zone":"workshop"}"#][..],
            ),
            (
                &["rename", "work", "Papers and more"],
                &[r#"{"command":"rename_zone","zone":"work","title":"Papers and more"}"#],
            ),
            (
                &["move", "work", "100,-20"],
                &[r#"{"command":"move_zone","zone":"work","x":100,"y":-20,"snap":true}"#],
            ),
            (
                &["move", "work", "100,50", "--display", "2", "--no-snap"],
                &[r#"{"command":"move_zone","zone":"work","x":2020,"y":50,"snap":false}"#],
            ),
            (
                &["resize", "workshop", "500x300"],
                &[
                    r#"{"command":"resize_zone","zone":"workshop","rect":{"x":400,"y":40,"width":500,"height":300},"edges":{"right":true,"bottom":true},"snap":true}"#,
                ],
            ),
            (
                &["set", "work", "view=list", "icon-size=32"],
                &[r#"{"view":"list","icon_size":32,"command":"set_view","zone":"work"}"#],
            ),
            (
                &["roll-up", "work"],
                &[r#"{"command":"roll_up","zone":"work"}"#],
            ),
            (
                &["roll-up", "work", "on"],
                &[r#"{"command":"roll_up","zone":"work","rolled_up":true}"#],
            ),
            (
                &["roll-up", "work", "off"],
                &[r#"{"command":"roll_up","zone":"work","rolled_up":false}"#],
            ),
            (
                &["lock", "work"],
                &[r#"{"geometry":true,"members":true,"command":"lock","zone":"work"}"#],
            ),
            (
                &["lock", "work", "geometry"],
                &[r#"{"geometry":true,"command":"lock","zone":"work"}"#],
            ),
            (
                &["lock", "work", "members"],
                &[r#"{"members":true,"command":"lock","zone":"work"}"#],
            ),
            (
                &["lock", "work", "none"],
                &[r#"{"geometry":false,"members":false,"command":"lock","zone":"work"}"#],
            ),
            (
                &["unlock", "work"],
                &[r#"{"command":"lock","zone":"work","geometry":false,"members":false}"#],
            ),
            (
                &["tab", "play", "--with", "work"],
                &[r#"{"command":"tab","zone":"zone-3","with":"work"}"#],
            ),
            (
                &["untab", "play"],
                &[r#"{"command":"untab","zone":"zone-3"}"#],
            ),
            (
                &["front", "play"],
                &[r#"{"command":"activate_tab","zone":"zone-3"}"#],
            ),
            (
                &["add", "play", "notes", "photo.png"],
                &[
                    r#"{"command":"add_item","zone":"zone-3","item":"/desk/notes.txt"}"#,
                    r#"{"command":"add_item","zone":"zone-3","item":"/desk/photo.png"}"#,
                ],
            ),
            (
                &["add", "play", "notes", "photo", "--index", "0"],
                &[
                    r#"{"command":"add_item","zone":"zone-3","item":"/desk/notes.txt","index":0}"#,
                    r#"{"command":"add_item","zone":"zone-3","item":"/desk/photo.png","index":1}"#,
                ],
            ),
            (
                &["remove", "plans", "budget"],
                &[
                    r#"{"command":"remove_item","item":"/desk/plans.pdf"}"#,
                    r#"{"command":"remove_item","item":"/desk/Budget 2026.xlsx"}"#,
                ],
            ),
            (
                &["place", "notes", "300,200", "--display", "LG-0042"],
                &[
                    r#"{"command":"place_loose_item","item":"/desk/notes.txt","display":"LG-0042","x":300,"y":200}"#,
                ],
            ),
            (
                &["rename-item", "notes", "todo.txt"],
                &[r#"{"command":"rename_item","item":"/desk/notes.txt","name":"todo.txt"}"#],
            ),
            (
                &["trash", "notes", "photo"],
                &[r#"{"command":"trash_items","items":["/desk/notes.txt","/desk/photo.png"]}"#],
            ),
            (&["undo"], &[r#"{"command":"undo_file_operation"}"#]),
            (&["hide"], &[r#"{"command":"quick_hide","hidden":true}"#]),
            (&["unhide"], &[r#"{"command":"quick_hide","hidden":false}"#]),
            (&["show"], &[r#"{"command":"quick_hide","hidden":false}"#]),
            (&["toggle"], &[r#"{"command":"quick_hide"}"#]),
            (
                &["hide", "work"],
                &[r#"{"command":"show_zone","zone":"work","shown":false}"#],
            ),
            (
                &["unhide", "work"],
                &[r#"{"command":"show_zone","zone":"work","shown":true}"#],
            ),
            (&["peek"], &[r#"{"command":"toggle_peek"}"#]),
            (&["peek", "on"], &[r#"{"command":"peek","on":true}"#]),
            (&["peek", "off"], &[r#"{"command":"peek","on":false}"#]),
            (
                &[
                    "set",
                    "work",
                    "opacity=80",
                    "blur=none",
                    "private=on",
                    "sort=name",
                ],
                &[
                    r#"{"sort":"name","command":"set_view","zone":"work"}"#,
                    r#"{"command":"modify_zone","zone":"work","changes":{"private":true,"look":{"opacity":80,"blur":null}}}"#,
                ],
            ),
            (
                &["open", "notes", "photo"],
                &[r#"{"command":"open","items":["/desk/notes.txt","/desk/photo.png"]}"#],
            ),
            (
                &["go-to", "plans"],
                &[r#"{"command":"focus_item","item":"/desk/plans.pdf"}"#],
            ),
            (
                &["next-tab", "work", "--backwards"],
                &[r#"{"command":"next_tab","zone":"work","backwards":true}"#],
            ),
            (
                &["lock-all"],
                &[r#"{"geometry":true,"members":true,"command":"lock_all"}"#],
            ),
            (
                &["lock-all", "none"],
                &[r#"{"geometry":false,"members":false,"command":"lock_all"}"#],
            ),
            (
                &["keep-zone", "work", "on"],
                &[r#"{"command":"always_visible","zone":"work","on":true}"#],
            ),
            (
                &["keep-item", "notes"],
                &[r#"{"command":"item_always_visible","item":"/desk/notes.txt"}"#],
            ),
            (
                &["refresh", "dl"],
                &[r#"{"command":"refresh_zone","zone":"dl"}"#],
            ),
            (
                &["new-folder", "dl", "Invoices"],
                &[r#"{"command":"new_folder","zone":"dl","name":"Invoices"}"#],
            ),
            (
                &["filter", "dl", "inv"],
                &[r#"{"command":"filter","zone":"dl","typed":"inv"}"#],
            ),
            (
                &["filter", "dl"],
                &[r#"{"command":"filter","zone":"dl","typed":""}"#],
            ),
            (
                &["go", "dl", "up"],
                &[r#"{"command":"navigate_up","zone":"dl"}"#],
            ),
            (
                &["go", "dl", "back"],
                &[r#"{"command":"navigate_back","zone":"dl"}"#],
            ),
            (
                &["go", "dl", "2026"],
                &[r#"{"command":"enter_folder","zone":"dl","item":"/home/ada/Downloads/2026"}"#],
            ),
            (&["privacy"], &[r#"{"command":"mask","masked":true}"#]),
            (&["privacy", "on"], &[r#"{"command":"mask","masked":true}"#]),
            (&["privacy", "off"], &[r#"{"command":"reveal"}"#]),
            // A zone that is not private is made so; one that is, is
            // covered again or uncovered for now.
            (
                &["privacy", "on", "--zone", "work"],
                &[r#"{"command":"modify_zone","zone":"work","changes":{"private":true}}"#],
            ),
            (
                &["privacy", "off", "--zone", "bank"],
                &[r#"{"command":"reveal","zone":"bank","revealed":true}"#],
            ),
            (
                &["privacy", "--zone", "bank"],
                &[r#"{"command":"reveal","zone":"bank","revealed":true}"#],
            ),
            (
                &["focus", "play"],
                &[r#"{"command":"focus_zone","zone":"zone-3"}"#],
            ),
            (
                &["focus", "left"],
                &[r#"{"command":"focus_neighbour","direction":"left"}"#],
            ),
            (&["focus", "none"], &[r#"{"command":"focus_zone"}"#]),
            // The zone that has the keyboard, by its own word.
            (
                &["roll-up", "@focused"],
                &[r#"{"command":"roll_up","zone":"workshop"}"#],
            ),
            (&["rules", "check"], &[r#"{"command":"check_timed_rules"}"#]),
            (
                &["focus", "desk"],
                &[r#"{"command":"focus_zone","zone":"@desk"}"#],
            ),
            (
                &["new-folder", "--in", "/home/ada/Desktop", "Projects"],
                &[r#"{"command":"new_folder_in","folder":"/home/ada/Desktop","name":"Projects"}"#],
            ),
            (
                &[
                    "move-to-folder",
                    "setup",
                    "--folder",
                    "/home/ada/Downloads/2026",
                ],
                &[
                    r#"{"command":"move_items_to_folder","items":["/home/ada/Downloads/setup.exe"],"folder":"/home/ada/Downloads/2026","copy":false}"#,
                ],
            ),
            (
                &[
                    "bring-in",
                    "/media/usb/film.mkv",
                    "--folder",
                    "/home/ada/Desktop",
                    "--zone",
                    "play",
                    "--copy",
                ],
                &[
                    r#"{"command":"bring_in","paths":["/media/usb/film.mkv"],"folder":"/home/ada/Desktop","copy":true,"zone":"zone-3"}"#,
                ],
            ),
            (
                &["cancel", "7"],
                &[r#"{"command":"cancel_operation","request":7}"#],
            ),
            (
                &["expand", "work"],
                &[r#"{"command":"expand","zone":"work"}"#],
            ),
            (
                &["expand", "work", "off"],
                &[r#"{"command":"expand","zone":"work","expanded":false}"#],
            ),
            (
                &["move-tab", "play", "0"],
                &[r#"{"command":"move_tab","zone":"zone-3","index":0}"#],
            ),
            (
                &["split-tab", "play", "640,300"],
                &[r#"{"command":"split_tab","zone":"zone-3","x":640,"y":300,"snap":true}"#],
            ),
            (
                &[
                    "settings",
                    "snap.grid=16",
                    "roll-up.expand_on=click",
                    "look.blur=none",
                    "privacy.everything=true",
                ],
                &[
                    r#"{"command":"modify_settings","changes":{"snap":{"grid":16},"roll_up":{"expand_on":"click"},"look":{"blur":null},"privacy":{"everything":true}}}"#,
                ],
            ),
            (
                &["select", "notes", "photo", "--mode", "toggle"],
                &[
                    r#"{"command":"select","items":["/desk/notes.txt","/desk/photo.png"],"mode":"toggle"}"#,
                ],
            ),
            (
                &["select"],
                &[r#"{"command":"select","items":[],"mode":"replace"}"#],
            ),
            (
                &["walk", "down", "--extend"],
                &[r#"{"command":"move_item_focus","step":"down","extend":true}"#],
            ),
            (&["open-chosen"], &[r#"{"command":"open_focused"}"#]),
            (
                &["move-chosen", "left"],
                &[r#"{"command":"move_focused_items","direction":"left"}"#],
            ),
            (
                &["move-chosen", "play"],
                &[r#"{"command":"move_focused_items","zone":"zone-3"}"#],
            ),
            (
                &["move-chosen", "desk"],
                &[r#"{"command":"move_focused_items","zone":"@desk"}"#],
            ),
            (&["dismiss"], &[r#"{"command":"dismiss"}"#]),
            (
                &["snapshot", "rename", "tidy", "neat"],
                &[r#"{"command":"rename_snapshot","name":"tidy","to":"neat"}"#],
            ),
            (
                &["snapshot", "export", "tidy", "tidy.json"],
                &[r#"{"command":"export_layout","name":"tidy"}"#],
            ),
            (
                &["snapshot", "export", "@current", "now.json"],
                &[r#"{"command":"export_layout"}"#],
            ),
            (
                &["profile", "copy", "2"],
                &[
                    r#"{"command":"copy_arrangement","from":{"displays":[{"id":"DEL-A1B2-77","width":1920,"height":1080,"scale":1000}]}}"#,
                ],
            ),
            (
                &["rules", "dry-run"],
                &[r#"{"command":"preview_rules","scope":"loose"}"#],
            ),
            (
                &["rules", "apply", "--scope", "all"],
                &[r#"{"command":"apply_rules","scope":"all"}"#],
            ),
            (&["rules", "undo"], &[r#"{"command":"undo_rules"}"#]),
            (
                &["snapshot", "save", "tidy"],
                &[r#"{"command":"save_snapshot","name":"tidy"}"#],
            ),
            (
                &["layout", "apply", "tidy"],
                &[r#"{"command":"restore_snapshot","name":"tidy"}"#],
            ),
            (
                &["snapshot", "delete", "tidy"],
                &[r#"{"command":"delete_snapshot","name":"tidy"}"#],
            ),
            (
                &["search", "rep"],
                &[r#"{"command":"search","typed":"rep"}"#],
            ),
            (&["grey"], &[r#"{"command":"grey_icons"}"#]),
            (&["grey", "toggle"], &[r#"{"command":"grey_icons"}"#]),
            (&["grey", "on"], &[r#"{"command":"grey_icons","on":true}"#]),
            (
                &["grey", "off"],
                &[r#"{"command":"grey_icons","on":false}"#],
            ),
            (
                &["auto-create"],
                &[r#"{"command":"auto_create","scope":"loose","dry_run":false}"#],
            ),
            (
                &["auto-create", "--dry-run"],
                &[r#"{"command":"auto_create","scope":"loose","dry_run":true}"#],
            ),
            (
                &[
                    "auto-create",
                    "--dry-run",
                    "--scope",
                    "all",
                    "--min-items",
                    "3",
                    "--no-rules",
                    "--display",
                    "2",
                ],
                &[
                    r#"{"command":"auto_create","scope":"all","dry_run":true,"min_items":3,"add_rules":false,"display":"LG-0042"}"#,
                ],
            ),
            (
                &["auto-create", "--display", "primary"],
                &[
                    r#"{"command":"auto_create","scope":"loose","dry_run":false,"display":"DEL-A1B2-77"}"#,
                ],
            ),
            (&["auto-undo"], &[r#"{"command":"undo_auto_create"}"#]),
        ] {
            let made = commands(words);
            assert_eq!(made, expected, "{words:?}");
            // And the zones read each of them: what is written here is
            // what the core takes.
            for command in &made {
                Request::from_json(command)
                    .unwrap_or_else(|why| panic!("{words:?}: {command}: {why}"));
            }
        }
        assert_eq!(
            commands(&["restore-explorer"]),
            [r#"{"command":"restore_system_icons"}"#]
        );
        assert_eq!(
            Request::from_json(&commands(&["restore-explorer"])[0]).unwrap(),
            Request::RestoreSystemIcons
        );
    }

    #[test]
    fn a_zone_is_made_as_it_is_described() {
        for (words, expected) in [
            (
                &["create"][..],
                r#"{"command":"create_zone","title":"Zone","kind":{"type":"manual"}}"#,
            ),
            (
                &["create", "--title", "Play"],
                r#"{"command":"create_zone","title":"Play","kind":{"type":"manual"}}"#,
            ),
            (
                &["create", "--dir", "/home/ada/Downloads/"],
                r#"{"command":"create_zone","title":"Downloads","kind":{"type":"portal","directory":"/home/ada/Downloads/"}}"#,
            ),
            (
                &["create", "--title", "Papers", "--query", "ext:pdf"],
                r#"{"command":"create_zone","title":"Papers","kind":{"type":"smart","query":[{"match":"extension","any_of":["pdf"]}]}}"#,
            ),
            (
                &["create", "--title", "On the other", "--display", "2"],
                r#"{"command":"create_zone","title":"On the other","kind":{"type":"manual"},"rect":{"x":1960,"y":40,"width":320,"height":240}}"#,
            ),
            (
                &[
                    "create",
                    "--title",
                    "Here",
                    "--display",
                    "2",
                    "--at",
                    "100,50",
                    "--size",
                    "400x300",
                ],
                r#"{"command":"create_zone","title":"Here","kind":{"type":"manual"},"rect":{"x":2020,"y":50,"width":400,"height":300}}"#,
            ),
            (
                &["create", "--title", "There", "--at", "-5,60"],
                r#"{"command":"create_zone","title":"There","kind":{"type":"manual"},"rect":{"x":-5,"y":60,"width":320,"height":240}}"#,
            ),
        ] {
            let made = commands(words);
            assert_eq!(made, [expected], "{words:?}");
            Request::from_json(&made[0]).unwrap_or_else(|why| panic!("{words:?}: {why}"));
        }
        // What making a zone does not take is given to it once it is there.
        let recent = asked(
            &read(&["create", "--title", "Recent", "--from", "recent"]).unwrap(),
            &desk(),
        )
        .unwrap();
        assert_eq!(
            recent.commands[0].to_string(),
            r#"{"command":"create_zone","title":"Recent","kind":{"type":"smart","query":[]}}"#
        );
        assert_eq!(
            recent.then_changes.unwrap().to_string(),
            r#"{"source":{"from":"recent"}}"#
        );
        let folder = asked(
            &read(&["create", "--query", "ext:pdf", "--from", "/home/ada/Papers"]).unwrap(),
            &desk(),
        )
        .unwrap();
        let change = json!({"command": "modify_zone", "zone": "zone-9", "changes": folder.then_changes.unwrap()});
        assert_eq!(
            change["changes"].to_string(),
            r#"{"source":{"from":"folder","directory":"/home/ada/Papers"}}"#
        );
        Request::from_value(change).unwrap();
        for (words, says) in [
            (
                &["create", "--kind", "portal"][..],
                "A portal needs the folder it shows",
            ),
            (
                &["create", "--kind", "smart"],
                "A smart zone needs what it shows",
            ),
            (
                &["create", "--dir", "/a", "--query", "ext:pdf"],
                "say one, or --kind",
            ),
            (
                &["create", "--kind", "manual", "--dir", "/a"],
                "A manual zone has no --dir",
            ),
            (&["create", "--at", "100"], "is no place"),
            (
                &["create", "--display", "9", "--at", "1,1"],
                "There is no display '9'",
            ),
        ] {
            let why = refusal(words);
            assert!(why.contains(says), "{words:?}: {why}");
        }
    }

    #[test]
    fn what_is_wrong_is_said_before_anything_is_sent() {
        for (words, says) in [
            (&["delete", "games"][..], "There is no zone 'games'"),
            (&["delete", "wo"], "is the beginning of 2 zones"),
            (&["add", "work", "report"], "is the name of 2 items"),
            (&["add", "work", "nothing"], "There is no item 'nothing'"),
            (
                &["tab", "work", "--with", "work"],
                "cannot be a tab beside itself",
            ),
            (&["move", "work", "here"], "is no place"),
            (&["resize", "work", "0x0"], "at least 1"),
            (&["set", "work", "colour=red"], "is nothing `set` knows"),
            (&["go", "work", "up"], "is no portal"),
            (&["new-folder", "dl"], "Say where and what it is called"),
            (
                &["walk", "sideways"],
                "A step is left, right, up, down, first or last",
            ),
            (
                &["select", "notes", "--mode", "some"],
                "--mode is replace, add, remove or toggle",
            ),
            (&["settings", "snap"], "write key=value"),
            (
                &["settings", "snap=1", "snap.grid=2"],
                "which was given a value before",
            ),
            (&["go", "dl", "2027"], "shows nothing called '2027'"),
            (&["profile", "copy", "1"], "is the one in use"),
            (&["profile", "copy", "7"], "There is no arrangement 7"),
            (
                &["auto-create", "--display", "9"],
                "There is no display '9'",
            ),
            (
                &["auto-create", "--min-items", "0"],
                "--min-items is at least 1",
            ),
        ] {
            let why = refusal(words);
            assert!(why.contains(says), "{words:?}: {why}");
        }
        // What the command line itself refuses: words that are missing.
        for words in [
            &["delete"][..],
            &["add", "work"],
            &["set", "work"],
            &["tab", "work"],
            &["rules"],
            &["snapshot", "save"],
            &["lock", "work", "everything"],
            &["peek", "maybe"],
            &["grey", "blue"],
            &["auto-create", "--scope", "some"],
            &["auto-create", "--min-items", "few"],
        ] {
            assert!(read(words).is_err(), "{words:?}");
        }
    }

    #[test]
    fn the_keys_of_the_examples_ask_what_the_zones_read() {
        let mut asked = 0;
        for example in gevrek_model::examples::built_in_examples() {
            for binding in &example.config.desktop.bindings {
                if let gevrek_model::command::DispatchCommand::Zones { action } = &binding.command {
                    Request::from_value(action.clone())
                        .unwrap_or_else(|why| panic!("{}: {}: {why}", example.id, binding.name));
                    asked += 1;
                }
            }
        }
        // The standard three in each example that has them, and the mode
        // of the Advanced Binding Lab.
        assert!(asked >= 3 + 20, "{asked}");
    }

    #[test]
    fn every_word_and_every_flag_says_what_it_is_for() {
        use clap::CommandFactory;
        fn told(command: &clap::Command, path: &str) {
            for flag in command
                .get_arguments()
                .filter(|argument| argument.get_long().is_some())
            {
                assert!(
                    flag.get_help().is_some(),
                    "{path} --{} has no help",
                    flag.get_id()
                );
            }
            for word in command
                .get_subcommands()
                .filter(|word| word.get_name() != "help")
            {
                let path = format!("{path} {}", word.get_name());
                assert!(word.get_about().is_some(), "{path} has no help");
                told(word, &path);
            }
        }
        told(&Line::command(), "zones");
    }

    #[test]
    fn what_was_done_is_said_in_a_sentence() {
        let said = |words: &[&str]| asked(&read(words).unwrap(), &desk()).unwrap().done;
        assert_eq!(
            said(&["delete", "work"]),
            "Zone 'work' was taken away; what it held is loose on the desktop"
        );
        assert_eq!(
            said(&["add", "play", "notes"]),
            "1 item put into zone 'zone-3'"
        );
        assert_eq!(
            said(&["remove", "plans", "budget"]),
            "2 items taken out; loose on the desktop"
        );
        assert_eq!(said(&["roll-up", "work"]), "Zone 'work' is rolled up");
        assert_eq!(
            said(&["roll-up", "work", "off"]),
            "Zone 'work' is rolled down"
        );
        assert_eq!(said(&["toggle"]), "The zones are hidden");
        assert_eq!(said(&["peek"]), "The zones are above the windows");
        assert_eq!(said(&["grey"]), "The icons are drawn grey");
        assert_eq!(
            said(&["grey", "off"]),
            "The icons are drawn in their colours"
        );
        let mut hidden = desk();
        hidden.quick_hidden = true;
        hidden.peeking = true;
        hidden.grey_icons = true;
        let said = |words: &[&str]| asked(&read(words).unwrap(), &hidden).unwrap();
        assert_eq!(said(&["toggle"]).done, "The zones are shown");
        assert_eq!(said(&["peek"]).done, "The zones are back below the windows");
        assert_eq!(said(&["grey"]).done, "The icons are drawn in their colours");
        assert_eq!(said(&["grey", "on"]).done, "The icons are drawn grey");
        // No zone has the keyboard: said here, before anything is sent.
        let mut nobody = desk();
        nobody.zones[1].focused = false;
        let why = asked(&read(&["roll-up", "@focused"]).unwrap(), &nobody).unwrap_err();
        assert!(why.starts_with("No zone has the keyboard"), "{why}");
        // A plan names what it leaves: asked with the items. Carried out,
        // it names none.
        assert!(names_items(&read(&["auto-create", "--dry-run"]).unwrap()));
        assert!(!names_items(&read(&["auto-create"]).unwrap()));
        assert!(names_items(&read(&["add", "work", "a"]).unwrap()));
        assert!(!names_items(&read(&["delete", "work"]).unwrap()));
    }
}
