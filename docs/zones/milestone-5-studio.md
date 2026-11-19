# Zones: the studio's page

Written 2026-11-19. The settings program has a page for the zones that
hold the desktop's icons: every key of `desktop.zones` is on it, and the
exemption in the page test is gone. This note says what the page holds,
how it talks to the zones that run, what was verified and what was not.

**Nobody has pressed anything on it.** The page was drawn into pictures
and looked at, and its logic is tested; it was never opened in a window,
and no window manager or shell was on the other end of it.

## Where it is

| | |
|---|---|
| The page | `crates/studio/src/zones_page.rs` |
| Which settings it shows | `crates/studio/src/pages.rs` (`Page::Zones`, the cards `ZONES`) |
| Wired into the window | `crates/studio/src/app.rs` (`Page::Zones` in `page_body`; `Event::ZonesAnswered`, `Event::ZonesState`, and `zones.*` events in `take_in`) |
| A number in a list's later entry | `crates/studio/src/form.rs` (`hint_at`) |

The page is called **"Zones on the desktop"** (key `zones`, under
"Desktop"), and says in its subtitle and in its first card that it is not
the places windows snap to. The page "Snap zones" (under "Behaviour")
says the reverse in its subtitle.

## What it holds

In the order of the page. The cards and their settings are written in
`pages.rs` once; the test `every_setting_is_on_exactly_one_page` holds
them against the schema, now without an exemption.

1. **Zones on the desktop.** The switch (`enabled`). Below it, what
   switching it on does to Explorer's desktop icons on Windows (a warning
   where the desktop is Windows', plain words elsewhere), in two wordings
   by `desktop.platform.windows.hide_desktop_icons`, with a link that goes
   to that setting on "This system". The setting itself stays where the
   catalogue puts it and is not shown twice.
2. **The zones** (`zones`). A line a zone: its title, where its items
   come from (filled by hand, shows a folder, shows what matches), and
   what is other than usual about it. Add, take out, drag by the handle,
   or move with the arrows. The one that is open shows the rest:
   - its id ("Name for rules and commands"), kept until the field is left;
     rules and the catch-all that name the zone follow the change;
   - a portal's folder, chosen with gevrek's own browser;
   - a smart zone's source (`source`, a folder where it is one), its
     query as a list of conditions, and `limit`;
   - everything else the model has for a zone, through the schema form:
     view, sort, sort descending, icon size, lock, rolled up, always
     visible, tab group, `expand_on`, `private`, `reserves_space`,
     `workspaces`;
   - "A look of its own": the zone's look laid over the general one,
     drawn small, and its fields.
   The card says that zones are placed on the desktop itself; there is no
   geometry here, because there is none in the configuration.
3. **Rules that sort the desktop** (`rules`, `catch_all`,
   `rules_on_arrival`, `timed_rules`). A line a rule: on or off, its
   name, and the rule as a sentence ("extension is pdf, docx → into
   “Work”"). Add, take out, drag, arrows. The open one shows its
   conditions and its action. A condition with one value has it beside
   it (extensions as one text parted by commas, kinds as ticks); one with
   several (a date) has them below, through the form. An action is "is
   shown in a zone" with the zone chosen from the zones there are, or "is
   moved to a folder" with a folder and a line that says the file itself
   is moved. Then what no rule matches, and when rules are tried by
   themselves.
4. **What the rules would do.** The dry run; see below.
5. **How every zone looks** (`look`). A zone drawn small with the look,
   on a ground that shows through it, and the fields: colours are chosen
   as colours, the rest is the form's.
6. **Moving and sizing a zone** (`snap`).
7. **How they behave.** Whatever else the section holds:
   `profile_per_workspace`, `loose_icon_size`, `roll_up`, `lock`,
   `privacy`. This card takes every key the others do not name, so a key
   the model gains is on the page at once.
8. **Arrangements kept by name.** Snapshots; see below.

What the configuration's own check says about zones
(`ZonesConfig::problems` as errors, `remarks` as warnings) is shown in
the card it is about, with the line's number ("Rule 3: No zone is called
'gone'"), beside the dot on the page's entry and the Problems page, which
every section has.

Kinds of zone, conditions, actions and sources are read from the schema:
one the model gains is offered without a change here (by its own name
until it is given better words). In a window narrower than 640 units of
room an entry's buttons stand on a line below its fields.

## Edits

Everything is an edit of the studio's document, as on every page: staged,
undone and redone with the rest, applied through the window manager,
written back as JSON with unknown keys kept. The page writes nothing by
itself. JSON has no comments; the order of keys is the model's.

## The dry run, and the zones that run

The page asks the zones through the window manager, with what
`gevrek-link` gained for it (`Request::Zones`, `Request::ZonesState`):

- While the page is shown and a window manager is connected, it asks how
  the zones are every 3 seconds, and at once after any `zones.*` event.
  Where no shell runs them, or they are switched off, the card says so
  in the window manager's words and the buttons are off. Without a window
  manager the buttons are off with "when a window manager is connected".
- **Dry run** sends `preview_rules` (scope: items in no zone, or every
  item) and shows the answer's `rules_previewed` as a table: item, what
  would happen, which rule decided; then the items a rule matched that
  are left alone, and why. It uses the zones' own `lines` where the
  answer has them, else the `plan` with names from the zones' state.
- **Apply the rules** is off until a dry run for the same items has
  shown something to do. It sends `apply_rules` and shows what was done.
- **Undo the last sorting** sends `undo_rules`; off where the zones say
  there is nothing to take back (`can_undo_rules`).
- The zones run the rules that were last applied. Where the document has
  unapplied edits the card says to apply first.
- A refusal is shown as the zones worded it. An answer that never comes
  is said after 20 seconds.

**Arrangements.** The list is the state's `snapshots`. Each can be put
back (`restore_snapshot`), written to a file (`export_layout` with its
name; the page writes the answer's document), or forgotten
(`delete_snapshot`). A name and a button keep the zones as they are now
(`save_snapshot`). A file can be written from the zones as they are now
(`export_layout` without a name) and read in (`import_layout`, applied).
The files are the only thing this page writes to the disk by itself, and
only on those buttons.

## The pictures

- `crates/studio/tests/pictures/zones.png`: the page as the picture test
  keeps it (Linux, gevrek's defaults, no window manager).
- `crates/studio/tests/pictures/snapzones.png`: new. The Snap zones page
  had no picture under its name since it was renamed; the old
  `zones.png` was its picture.
- Every other picture in that folder was drawn again: the list of pages
  on the left has one more entry. Outside that list nothing differs but
  `bar.png` (0.08 % of the page) and `system.png` (0.71 %), which follow
  settings added by other work (a panel setting; `hide_desktop_icons`).
- To look at the page with zones and rules in it:
  `GEVREK_LOOK=zones GEVREK_LOOK_FILE=<a config> GEVREK_LOOK_OUT=<png>
  cargo test -p gevrek-studio a_page_is_drawn_to_be_looked_at -- --ignored`
  (`GEVREK_LOOK_PRESS=x,y` opens an entry).
- To look at the dry run's table and the arrangements:
  `GEVREK_LOOK_OUT=<png> cargo test -p gevrek-studio
  the_answer_to_a_dry_run_is_drawn`.

Looked at while it was made: the page empty, with three zones and three
rules, with a smart zone open, with a rule open, at 760 units wide, and
the table with the arrangements.

## What was verified

- `cargo test -p gevrek-studio`: the page test without the exemption;
  the lists (take out, move, what is open follows); a zone added and
  renamed with its rules following; a rule as a sentence; the plan as a
  table, from the plan and from the zones' lines; asking, the answer,
  refusals, no zones running; the zones' state read; arrangements kept,
  written to a file and read; the check's messages in their cards; that
  what the page writes is a configuration the model reads; that the
  table draws.
- `cargo clippy -p gevrek-studio --all-targets`: nothing in the studio.
- `cargo build`; and on Linux (WSL) `cargo check -p gevrek-studio
  --all-targets` and the tests.

## What was not verified

- **The page in a window, with a pointer.** Dragging a line, the folder
  browser opened from the page, typing: drawn, not pressed.
- **Anything against zones that run.** The asking side was tested
  against answers written by hand in the shapes of
  `core-milestones-3-5.md`. No dry run, apply, undo, snapshot, export or
  import was ever sent to a shell.
- The picture test on Linux ends with a crash of the graphics driver in
  WSL when the test program exits, after its comparisons and after the
  pictures are written. It does so when a single page other than this
  one is drawn too (the overview was tried), so it is the machine's, not
  the page's; whether it did before this work was not tried.

## What has no editor of its own

- **A date's `after` and `before`** are seconds since 1970, typed as a
  number. `newer_than_days` and `older_than_days` are plain.
- **A size** is typed in bytes.
- **`workspaces` of a zone** is a list of names to type, not a choice
  from the workspaces there are.
- **`rename_snapshot` and `copy_arrangement`** have no button; the
  arrangements per set of displays (`profiles`) are not listed.
- **Bindings** for zones are edited on the Key bindings page as any
  command is; this page has no shortcut to them.
- **Per-item settings** (`item_always_visible`) are state, set on the
  desktop.
