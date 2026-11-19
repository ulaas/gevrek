# Zones: milestone 5, the shell's part

Written 2026-11-19. Arrangements kept and carried in and out, zones by
workspace and by set of displays, the keyboard, the search, privacy,
room kept clear of tiled windows, displays of different scale, what a
screen reader is told, and what a frame costs.

What decides is the core (`core-milestones-3-5.md`); what the surfaces
can do is shellkit's (`milestone-3-surfaces.md`). This page is what the
shell draws and hears: `crates/shell/src/zones/`.

**Nobody has seen it run.** Built against the backend in memory, tried
in tests, drawn into pictures that were looked at. No screen reader was
ever attached; no screen was ever shared.

## What exists

| File | What changed |
|---|---|
| `zones/access.rs` (new) | What a screen reader is told: every zone that is shown a list, every item that is seen an option of it, in the core's words; what it asks for by a node is done. Nothing is made while none listens. |
| `zones/draw.rs` | Pictures and names are found by the item's id as it stands in the layout (no id is copied to look one up); a picture the backend let go of is asked for again; the cover of what is private; a figure for a zone that keeps windows off. |
| `zones/mod.rs` | `screen_shared`, `kept_from_pictures`, `export_to`, `import_from`, `say`; `laid_out` counts how often everything was laid out. |
| `zones/surfaces.rs` | The surfaces are left out of pictures of the screen while something private is on them; strips that keep room for zones that ask (`rooms_for`, `open_room`); the arrangements' menus; the boxes for an arrangement's name and file; a sheet that says what came of it. |
| `zones/gesture.rs` | A press on a cover asks to see what is under it. |

**Edits outside `crates/shell/src/zones/`** (one more; the first is in
`milestone-4.md`): `crates/shell/src/main.rs`, where the window manager's
events are read: one arm, `Event::Runtime { kind, fields, .. } if kind ==
"screen.shared"`, which hands the event's fields to
`Running::screen_shared` (seven lines, before the `"shell.command"` arm).

## How each thing is done

| What | By the pointer | The command |
|---|---|---|
| Keep this arrangement | Desktop menu, "Arrangements…", "Keep this arrangement as…" | `save_snapshot` |
| Go back to one, rename it, forget it | "Arrangements…", its name, then the line | `restore_snapshot`, `rename_snapshot`, `delete_snapshot` |
| Write one to a file | Its menu, "Write it to a file…"; or "Write this arrangement to a file…" for the one in use | `export_layout`; the shell writes the document (beside its place first, then renamed over it) |
| Take one from a file | "Take an arrangement from a file…" | `import_layout` with `apply` |
| A zone on some workspaces only | (configuration: `workspaces`; `gevrek zones`) | `modify_zone` |
| Each workspace its own arrangement | (configuration: `profile_per_workspace`) | `modify_settings` |
| Make a zone private | The zone's menu, "Private" | `modify_zone` |
| See what a covered zone holds | Press on its cover; the zone's menu, "Show what it holds" | `reveal` |
| Cover it again | The zone's menu, "Cover it again" | `reveal` with `revealed: false` |
| Cover everything | Desktop menu, "Everything covered from view" | `mask` |
| Keep tiled windows off a zone | The zone's menu, "Windows keep off its place" | `modify_zone` (`reserves_space`) |
| The keyboard (all of it) | see `milestone-3.md`, "The keys". The loose items of the bare desktop are walked by the same keys: the core keeps the keyboard there too (`@desk`), Alt+arrows go between zones and the desktop, Alt+Shift+arrows carry items out of a zone onto it and back | `move_item_focus`, `focus_neighbour`, `move_focused_items` |
| Search everywhere | Look at the zones (Super+Z) and type | `peek`; the shell asks `core.search` |

Whatever comes of writing or taking in a file is said on a sheet (the
file's name, or why not), since a file being written is not seen.

## Workspaces and sets of displays

The core chooses the arrangement; the shell tells it the displays (from
every state of the window manager) and the workspace that is shown where
the keyboard is, lays out anew, and opens or closes a surface for each
display that came or went (`surfaces::sync`, at every wake-up).

Another workspace or another set of displays is a change of what is
there, not a hiding: it is drawn at once, nothing fades (fading is for
the one key that hides everything), and a zone that is rolled open or
shut on the way is drawn as it ends up. A test holds that nothing is on
its way after such a change.

## Privacy

- A private zone is covered: its ground with a cloth of slanting lines,
  a struck-through eye, "8 items, covered", "Press to show". **Nothing
  of what it holds reaches the shell while it is covered** (the core
  leaves it out of the scene), so it cannot be drawn, found, or read out.
- The loose items covered: a chip at the top of the display, "4 items,
  covered".
- **Linux**: the compositor's `screen.shared` event reaches
  `Running::screen_shared`, which tells the core
  (`BackendEvent::ScreenShared`) whether any display is shown to others.
  What happens then is the configuration's (`privacy.when_shared`:
  what was asked to be seen is covered again, or everything is, or
  nothing).
- **Windows** gives no sign of a shared screen. So, while a zone is
  private (or everything is) and `when_shared` is not `nothing`, the
  zones' surfaces are left out of every picture taken of the screen
  (`Control::exclude_from_capture`), also while the person has asked to
  see what a zone holds: seen on the display, absent from a recording.
  And everything can be covered by hand (`mask`; a key bound to it).
  With no private zone, the surfaces are in pictures as ever.

## Room kept clear of tiled windows

The core says which room (`reserved_regions`: display, side, depth).

- **Linux**: for each display and side one strip surface with nothing on
  it (`Place::reserved_strip`, the pointer goes through), as deep as the
  deepest zone kept there; resized when a zone is, closed when no zone
  asks any more or the display goes.
- **Windows**: no surface is opened (one that keeps room would be an
  application bar and change every program's work area). The room is in
  the snapshot the zones publish (`reserved`), and the window manager
  keeps it from there (`milestone-3-surfaces.md`, "Room kept clear of
  tiling").

**The zones' own room is never told back to the core as room that is
taken.** On both systems the work area in the window manager's state is
now what is left for the windows, the zones' room taken off; told to the
core as it is, a zone that keeps room would find its own room taken, be
moved out of it, keep other room, and so on. So before the state's room
goes to the backend (`zones/backend.rs`, `System::told`),
`panels_state` gives the zones' part back to each display's work area,
per side and never beyond the display, so that what is then read as
kept at its sides is the panels' alone.

Which part is the zones' is read **from the state itself**:
`monitors[].zones_room` (`left`, `top`, `right`, `bottom`), the room
that was in force when that state was made, in the units of the state's
own work area (pixels on Windows, logical units on Linux): so it is
given back before anything is reckoned into the zones' units, and the
panels' room comes out exactly at every scale. It
is not worked out from what the zones last asked for: the state follows
a request by a round trip (the snapshot to the window manager on
Windows, the strip surface's configure on Linux), and in between the two
differ. A state that names no such room is taken as it is.

The state names it on both systems (`Sides zones_room` of a monitor in
`gevrek.proto`; filled by the engine on Windows from the room it took
off, and by the compositor on Linux from the strips' exclusive zones).
That the shell's reading and their writing meet was not run end to end:
the shell's test plays the window manager, and theirs play the shell.

The test `a_zone_that_keeps_room_stays_where_it_is_whatever_the_window_
manager_makes_of_its_room` plays a window manager that keeps the room it
was last told and names it in its state: a zone at the top under a
panel, at the top without one, at the left with a panel at the top, at
the bottom above a panel, at the right without one; each through ten
states, with the room in force at once, one state late and two states
late. The zone's rectangle and the room it asks for are the same after
every state, on displays at 100, 125, 150, 175 and 200 %. The same run
with the state's room told as it is moves the
zone, which is held too, so that the test is known to hold something.
It runs against each system's own `displays_of_state`.

## Displays of different scale

Everything is laid out in the desktop's own logical units, and each
surface draws in its own points, so a zone or an item dragged from a
display at 100 % to one at 150 % is one movement. A picture is asked for
in the pixels of the surface that shows it (48 units: 48 pixels on the
one, 72 on the other), each surface has its own textures, and a zone
that comes to another display has its pictures asked for anew there,
not stretched. Fractional scales are whatever the surface's scale is.

## What a screen reader is told

Through AccessKit, by way of the toolkit (shellkit already hands the
toolkit's tree to `accesskit_windows` / `accesskit_unix`):

| | Role | Name | State |
|---|---|---|---|
| A zone that is shown (of tabs the one in front) | list box | its title | description in the core's words ("tab 2 of 3, 8 items, locked", "folder Downloads, 2026", "covered"); what is typed into its filter as its value; expanded or not while it is rolled up; which one of how many zones |
| An item that is seen in it | option of that list | its name | selected; which one of how many; the keyboard |
| A display's loose items | list box | "Desktop items" | how many |

- The node that has the keyboard is the zone or item the core says has
  it, while the surface has the keyboard.
- A screen reader's "go to" on an item gives it the keyboard
  (`focus_item`, or it is chosen where it is loose); "press" opens it;
  on a zone, the zone has the keyboard.
- Items of a zone that is rolled up and shut, of a tab that is not in
  front, and of a zone that is covered are not told (the last are not
  even known to the shell).
- Visible focus: an accent rim on the zone and a ring on the item, both
  thicker with high contrast; with the keyboard among the loose items
  and on none of them yet, a rim around the room they lie in. Reduced
  movement: see `milestone-3.md`.

## What a frame costs

`a_frame_of_two_thousand_items_does_nothing_twice` (1500 items in one
zone that is scrolled through, 500 loose, a display of 2560 by 1440,
debug build, no graphics device: laying out and making the shapes, not
filling pixels). Printed on the machine this was written on:

| | Laying out (once per change) | First frame | A frame after that |
|---|---|---|---|
| Windows 11 | 14 ms | 77 ms | 5.9 ms |
| WSL 2 (Fedora) | 15 ms | 84 ms | 5.8 ms |

The test does not hold these (only that a frame is far from a second).
What it holds exactly:

- **Nothing is laid out again unless something changed** (`laid_out`
  stays over five frames; one notch of the wheel is one more).
- **Lazy pictures**: of 2000 items, the 721 that are seen are asked for,
  each once; scrolling asks for the rows that came into sight and no
  others. A picture the backend let go of (it keeps 48 MiB) is asked for
  again rather than taken for "there is none".
- **No name is broken into lines twice**; pictures and names are looked
  up by the id in the layout, with no copy.
- **At rest nothing asks for the next frame**; the only waits are the
  core's (`wake_at_ms`), the state's saving and the timed rules, each one
  wake-up asked for in advance. Nothing looks by the clock.

## Pictures

    cargo test -p gevrek-shell milestone_five_is_drawn -- --ignored

writes into `target/zones-pictures/`:

| File | What |
|---|---|
| `m5-private.png`, `m5-private-high-contrast.png` | Two private zones, one covered, one asked to be seen |
| `m5-everything-covered.png` | Everything covered by hand: every zone, and the loose items as a chip |
| `m5-keyboard.png`, `m5-keyboard-1.5x.png` (1920 by 1200), `m5-keyboard-high-contrast.png` | The keyboard on a zone and on an item, three chosen |
| `m5-zone-keeps-windows-off.png` | A zone that keeps windows off and stays when the rest is hidden, as its title says |

Looked at, all of them, together with the pictures of milestones 3 and
4: one mark, one accent, one set of figures throughout. Changed after
looking: the cover says how to see what is under it; a zone that keeps
windows off had no sign of it.

## What was verified

- `cargo test -p gevrek-shell -p gevrek-zones` on Windows; `cargo test
  -p gevrek-shell` in WSL; clippy on both with the two old warnings
  only; `cargo build`. Numbers in the report of the day.
- Tests beside the code: an arrangement kept, gone back to, written to
  a file and taken from one, a file that is none refused with nothing
  changed; another workspace at once and without fading, an arrangement
  for each; a shared screen covering what is private, a press on the
  cover, everything covered by hand; the surfaces kept from pictures
  while something is private; the room for a zone that asks, one strip a
  side; two displays of different scale; the screen reader's tree read
  back from the toolkit (roles, names, selected, one of how many, the
  keyboard, rolled up, covered, and "go to" carried out); the frame of
  two thousand items; that what the screen reader is told asks for no
  surface's keyboard.

## What was not verified, and what is not done

- **No screen reader was attached.** The tree was read back from the
  toolkit in a test; what Narrator, NVDA or Orca say of it, and whether
  a list box with options is how they read best, is unknown.
- **No screen was shared.** That the compositor's event arrives, and
  that a surface excluded from capture is absent from a recording on
  Windows, is shellkit's and the compositor's, and unseen.
- **A zone that keeps room was never seen keeping it**: the shell takes
  the zones' own room out of what it tells the core by the state's
  `zones_room`, tested against a window manager played in the test; the
  real ones were not run against it, and no window was ever tiled
  against a zone on a real desktop.
- A shell that starts while a display is already shared hears nothing
  until the next change (the compositor's event has no "now").
- Menus are worked with the pointer: a menu has no walk by the arrow
  keys, and no key opens a zone's or an item's menu. Everything a menu
  does is a command, so a key can be bound to it; but the menus
  themselves are not reached by keyboard, and are not told to a screen
  reader as menus (their lines are painted).
- The boxes and sheets are the toolkit's own widgets and are told by it;
  never heard.
- An arrangement is written to a path that is typed; there is no file
  chooser. A file that is there already is written over.
- The frame's cost was measured without a graphics device; what filling
  the pixels of 5000 shapes costs on a real one is not in the number.
- Snapshots published to the window manager still carry every item at
  every change (the line itself no longer sends one that is the same as
  the last). Of a drag's many `zone_placed`, and of a long operation's
  progress, only the last of each run is told (`Zones::take_heard`).

## Manual test checklist

**Arrangements (both systems)**
- [ ] "Keep this arrangement as…" twice with two layouts; go back to
      the first: zones and items are where they were. Under
      "Arrangements…" a "Before restoring" appeared.
- [ ] Write one to a file; delete a zone; take the file in: it is back,
      kept as "name (2)", and a sheet says so. Take in a file that is
      something else: a sheet says why, and nothing changed.

**Workspaces and displays**
- [ ] Give a zone `workspaces: ["2"]`: it shows on workspace 2 only,
      appearing and going at once as the workspace is switched.
- [ ] `profile_per_workspace: true`: move a zone on one workspace; on
      another it is where it was.
- [ ] Unplug the second display: its zones come to the first, reachable;
      plug it back: they return to where they were.
- [ ] Two displays at 100 % and 150 %: icons are sharp on both; drag a
      zone across: it keeps its size in the hand, and its icons are
      sharp again a moment after it arrives. Drag an item across.
- [ ] Change a display's scale while zones show (125 %, 175 %).

**Privacy**
- [ ] Mark a zone "Private": it is covered. Press the cover: shown.
- [ ] Linux: start a screen recording or share the screen in a call:
      the zone is covered again within a few seconds; with
      `privacy.when_shared: everything` every zone and the loose items.
- [ ] Windows: with a private zone, take a screenshot and record the
      screen: the zones are not in the picture (the wallpaper is); with
      no private zone they are.
- [ ] "Everything covered from view" and back.

**Room for a zone**
- [ ] Linux: "Windows keep off its place" on a zone at a display's side:
      tiled windows make room at once, and give it back when the zone is
      hidden, rolled up (its title's room stays) or switched off.
- [ ] Windows: the same; `gevrek zones list --json` shows the room under
      `reserved`.
- [ ] Both: with the zone keeping room, leave it for a minute, switch
      workspaces, open and close a window: the zone must not creep, and
      the room must not grow or flicker. With a panel at the same side
      and without one.

**Keyboard**
- [ ] Press on a zone, then: arrows, Home, End, Shift+arrows; Enter;
      Alt+arrows to the neighbours; Alt+Shift+arrow carries the chosen
      items over; Ctrl+R; Ctrl+Tab on tabs; letters filter, Escape
      empties, Escape again lets go of the choice, again of the keyboard.
- [ ] Super+Z, type, arrows, Enter: without touching the pointer.
- [ ] The rim and the ring are always there while the keys go to the
      zones, and clear to see on a busy wallpaper and with high contrast.

**Screen reader**
- [ ] Windows, Narrator or NVDA; Linux, Orca. Press on a zone and walk
      with the arrows: each item is read with its name and "n of m";
      chosen ones as selected.
- [ ] Alt+arrow to another zone: its title, how many items, "locked"
      where it is; a rolled-up zone as collapsed.
- [ ] A covered zone: its name and "covered", and no item.
- [ ] The reader's own "go to" and "press" on an item.

**Cost**
- [ ] A desktop folder of 2000 files with zones on: dragging a zone and
      scrolling one stay smooth; left alone, `gevrek-shell` uses no
      processor time and its memory does not grow.
