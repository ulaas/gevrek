# Zones: milestone 1 (the core)

Written 2026-11-19. What `design.md` calls milestone 1, as far as it is
the portable crate: **`crates/zones` (`gevrek-zones`)**.

**Nothing is drawn, and no program runs the zones.** No backend for
Windows or Linux exists yet and the shell does not know the crate. What
exists is the logic, with tests that run without a display, and the
section of the configuration (`desktop.zones`), which is off by default.

## What exists

| Module | What it is |
|---|---|
| `config` | What the person sets: zones, looks, rules, snapping. The types live in the model (`crates/model/src/zones.rs`) and are `DesktopConfig::zones`; `gevrek_zones::config` names them again. Defaults for everything, unknown keys kept, validated with the rest of the configuration. |
| `ids`, `item` | `ZoneId`, `ItemId`, `DisplayId` (strings that stay the same over restarts) and an item with the facts rules ask about. |
| `placement` | Which item is in which zone, in which order and cell; loose items and their own places. An item is in one place only. |
| `geometry` | A zone's place as display id + anchor (corner, edge, centre) + offset and size in logical units; to a rectangle and back; the free room of a display. |
| `profile` | One arrangement for each set of displays (ids, sizes, scales, optionally the workspace). Choosing one, making one from the nearest, sending zones of a display that went to one that is there. At most 48 are kept. |
| `state`, `store` | What is remembered (placements, arrangements, snapshots, the undo record), and its file `zones.json` in a folder the caller names: version 1 with a migration hook, atomic writes, 20 numbered backups, and a load that puts a corrupt or newer file aside and uses the newest good backup. |
| `snap` | Moving and sizing: the free room, the display's sides, other zones with one gap, a grid; and bringing a zone back to where it can be reached. |
| `layout` | Sorting, the grid or list inside a zone with how far it scrolls, loose items laid out down the columns, the rolled-up size, tab groups. |
| `rules` | Globs and matchers, first match wins, a catch-all, a dry run (`plan`), `apply` (returns the undo record and the file moves to carry out), `undo`. The time is an input. |
| `search` | Names by what was typed, with the launcher's marks (`gevrek_system::apps::score`). |
| `backend` | The trait `ZonesBackend` and what passes between it and the core (`BackendEvent`, `BackendRequest`, `FileOperation`). |
| `core` | The state machine `ZonesCore`: `event(BackendEvent, now)` and `command(Command, now)` give `Vec<Effect>`. `Command`, `ZonesEvent`, `Effect` and `Scene` are serde types. |
| `host` | Carries events and requests between a core and a backend. The one place a backend is called from. |
| `testing` | `FakeBackend`: folders, displays and file operations in memory, behind the `test-support` feature. |

## In the configuration

- `desktop.zones` is a `ZonesConfig`, with `enabled` false: this is the
  feature's switch, off until milestone 3. A core whose configuration is
  off shows an empty scene, refuses every command, sorts nothing by
  itself and asks the backend for nothing; `Host` does not list or watch
  anything until `Host::set_config` switches it on.
- No new schema version: the section reads with its defaults from a file
  that does not have it, as `outputs`, `session`, `wallpaper`,
  `night_light` and `layer_rules` do, which have no step in
  `migrate.rs` either. (`input` once got an empty step, 22; that is the
  one case the other way.) Schema 26, which freed the name, is not
  released yet in any case.
- None of its keys is one that only snap zones have (`behavior`,
  `default_layout`, `monitor_layouts`), so a file gevrek wrote is never
  refused by `Config::from_json`; a test in `migrate.rs` holds that, and
  that the old shape still is refused.
- Validation (`ZonesConfig::problems`, paths `desktop.zones...`): zone ids
  not empty and not twice, icon sizes 16 to 256, opacity 0 to 100, snap
  distances in range, a portal with its folder, an action with its zone or
  folder. A rule that names a zone which is not there is a warning, not
  an error: taking a zone away must not leave a file that cannot be
  saved, and such a rule already does nothing (its items are held back).
- The schema crate has ranges for the section's numbers (`hints.rs`); the
  schema itself is made at run time from the model, there is no file of
  it to regenerate. The golden files were written anew.
- The server lists the capability `zones_v1`.

## Running the tests

```
cargo test -p gevrek-zones
cargo clippy -p gevrek-zones --all-targets
```

On Linux, from Git Bash through WSL:

```
MSYS_NO_PATHCONV=1 wsl.exe -d FedoraLinux-44 -- bash -lc '. ~/.cargo/env; cd /mnt/e/source/private/gevrek && export CARGO_TARGET_DIR=$HOME/gevrek-target && cargo test -p gevrek-zones'
```

The long runs take their numbers from seeds. `GEVREK_RUNS=<n>` makes n
times as many, with other seeds; a run that fails names its seed, and
`GEVREK_SEED=0x...` runs that one alone.

## What was verified

The tests of `gevrek-zones` (83) and of the model (141 on Windows, 142
on Linux; the section's among them) pass on Windows and on Linux (Fedora
in WSL).

- Tables: snapping (move and resize), anchors there and back, sorting,
  layout, every matcher, rule order, catch-all, dry run against apply,
  undo, the store (round trip, corrupt file, newer version, missing file,
  backup rotation, migration steps).
- Scenarios (`core_scenarios`): the core on the fake backend, through the
  host: zones, items, tabs, locks, roll-up, quick-hide and peek, rules with
  files moved and brought back, portals, snapshots, displays going and
  returning, workspaces, a restart from the saved file.
- Long runs (`random_runs`), with these held after every step:
  - rules (48 runs of 160 rounds): a dry run changes nothing and says the
    same twice; each step was decided by the first matching rule; applying
    does exactly the plan; nothing is left to do afterwards; undo gives
    back exactly the placements there were; plan and undo record survive
    JSON.
  - layout (24 runs of 260 steps of displays coming, going, changing
    places, size and scale, returning, mixed with commands): no item in
    two places; no zone lost; every zone wholly within the free room of a
    display that is there; an arrangement that is not in use is never
    changed, and a set of displays that returns shows its zones where they
    were last seen on it; the state survives its text at every step and
    its file at every 32nd (through the same `encode` and `decode`; the
    disk itself is slow, and every step would be mostly waiting for it);
    a core started anew from what was written shows the same scene.
- The runs count what they came by, and fail if that is too little. Five
  deliberate breakages (no rehoming, a move written to every arrangement,
  undo in the wrong order, last match wins, no clamping) were each caught
  by a table or scenario and by a long run, then taken out again.

## What was not verified, and what is not done

- Nothing was seen on a screen. There is no renderer and no real backend;
  the trait has only been implemented in memory, so whether it fits the
  shell namespace or the XDG folders is untried.
- The crate is used by no program. The studio has no page for the
  section (its test that every setting is on a page lets `desktop.zones`
  be on none, for now) and the guide does not mention it: it is off and
  has nothing to show.
- The display ids come from `MonitorInfo::stable_id`
  (`engine/src/outputs_edid.rs`), which already tells two displays of one
  model without serial numbers apart. Nothing hands them to the zones
  yet: that is the backend's part.
- An item can be put into a portal (the file moves there) but not dragged
  out of one into a zone filled by hand: the core does not know the
  desktop's own folder.
- Items that came while gevrek was not running are not sorted by the
  rules at the start; only items that come while it runs are.
- Placements of items that are not there are kept (a drive not plugged
  in) and never cleared out except when the backend says the item went.
- Icons pass through as handles only; open, "open with", the context
  menu and drags are on the trait and the fake, and nothing calls them
  yet (the renderer will).
- Saving is the host's to pace: `Store::save` keeps a backup with every
  save that changes the file, so saving on every pointer movement would
  turn the twenty backups over in a second.
