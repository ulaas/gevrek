# Zones: what the core holds for milestones 3 to 5

Written 2026-11-19. This is the contract between the portable core
(`crates/zones`, with its configuration in `crates/model/src/zones.rs`)
and everything that is built on it: the shell's drawing, the system
backends, the compositor, the command line and the studio. For each
feature it names the commands, the events, the fields of the scene and
the keys of the configuration, with one example each.

Every `json` block here is read by a test of the crate
(`docs::every_example_in_the_contract_is_read`), so an example that no
longer fits the code fails the build. Blocks marked `jsonc` are parts of
larger things and are not read.

Nothing of this is drawn by the core, and nothing here was seen on a
screen: it is logic, with tests that need no display.

## How the core is talked to

Unchanged from milestone 1: `ZonesCore::command(Command, now)` and
`ZonesCore::event(BackendEvent, now)` give `Vec<Effect>`; `Host` carries
`Effect::Ask` to the backend. What is new:

- **No new kind of effect.** Everything new that a host has to act on
  comes as a `ZonesEvent` inside `Effect::Notify`: `open_requested`
  (start these items), `reserved_changed` (hand this room to the window
  manager), `layout_exported` (write this document somewhere).
- **Two kinds of time.** `now` is seconds since 1970, as before. What is
  measured in parts of a second comes as `at_ms` in the commands that
  need it (`pointer_over`, `tick`): any count of thousandths of a second
  that only goes forward. After every call the host asks
  `ZonesCore::wake_at_ms()`; if it gives a time, the host sends
  `{"command": "tick", "at_ms": ...}` then (or a little later). The core
  owns no timer.
- **What is so for the moment only** (which rolled-up zone is open, where
  a portal stands, what is typed into a filter, what has the keyboard,
  what was asked to be seen) is in the core but in neither file. After a
  restart it is gone. It is all in the scene and in the snapshot.
- **`"@focused"`** may stand for a zone's id in any command (the fields
  `zone` and `with`): the zone that has the keyboard. With no zone
  focused the command is refused (`no zone has the keyboard`). This is
  how one key binding acts on whichever zone the person is in.

```json
{"command": "roll_up", "zone": "@focused"}
```

- **Events told by the way.** After any command or event the core adds
  `focus_changed`, `selection_changed` and `reserved_changed` when those
  changed as a consequence (a zone that had the keyboard was deleted, a
  chosen file went away).

New accessors on `ZonesCore`: `wake_at_ms()`, `timed_rules_every()`,
`focus()`, `selection()`, `is_expanded(zone)`, `is_masked(zone)`,
`loose_masked()`, `lock_of(zone)`, `zone_holds(zone, now)` (everything a
zone holds; `zone_items` is what its filter leaves), `reserved_regions()`,
`explain(plan)`, `current_layout(name, now)`, `workspace()`.
`search(typed, now)` takes the time now (smart zones ask it).

## Milestone 3

### Roll-up

A zone's `rolled_up` is configuration, as before (`roll_up`). A rolled-up
zone can be **open for now**: that is view state, changes no file, and
ends by itself.

| | |
|---|---|
| Config | `desktop.zones.roll_up.expand_on` (`hover`, the default, or `click`), `.expand_delay_ms` (150), `.collapse_delay_ms` (400, each 0 to 10000); per zone `expand_on` (absent: as all) |
| Commands | `pointer_over {zone?, at_ms, dragging}`, `expand {zone, expanded?}`, `tick {at_ms}`; `roll_up` as before |
| Events | `zone_expanded {zone, expanded}` |
| Scene | `ZoneScene.rolled_up` (configured), `.expanded` (open for now; `rect` is the whole zone's then), `.expand_on` (resolved), `.whole` (the whole zone's rectangle always: what `rect` grows to). The delays are read from the configuration |

```json
{"roll_up": {"expand_on": "click", "expand_delay_ms": 0, "collapse_delay_ms": 600},
 "zones": [{"id": "tools", "title": "Tools", "rolled_up": true, "expand_on": "hover"}]}
```

The shell sends `pointer_over` whenever the zone under the pointer
changes (with no zone when it is over none), and `tick` when
`wake_at_ms()` says:

```json
{"command": "pointer_over", "zone": "tools", "at_ms": 51200}
```
```json
{"command": "tick", "at_ms": 51350}
```
```json
{"event": "zone_expanded", "zone": "tools", "expanded": true}
```

- Hover: the zone opens once the pointer has rested on it for
  `expand_delay_ms`. Passing over it opens nothing.
- `"dragging": true` opens it after the same delay whatever `expand_on`
  says: something carried over a rolled-up zone must be droppable inside.
- Click: the shell sends `expand` for a press on the title of a rolled-up
  zone (without `expanded`: the other of what it is).
- Either way it closes `collapse_delay_ms` after the pointer left (a
  `pointer_over` for something else). Coming back in time keeps it open.
- The keyboard holds a zone open as the pointer does: walking into its
  items opens it, and it closes when the keyboard leaves.
- Tabs open and close together. `roll_up` (the real one) ends any
  "open for now".

```json
{"command": "expand", "zone": "tools"}
```

### Tabs

`tab`, `untab` and `activate_tab` are as they were. Added: the order of
the tabs, splitting by dragging, and going round with a key.

| | |
|---|---|
| Config | `tab_group` per zone, as before. The order of the tabs is the order of the zones in `desktop.zones.zones`; the first one's place is the group's |
| Commands | `tab {zone, with}` (a zone dropped onto another), `untab {zone}`, `activate_tab {zone}`, **`move_tab {zone, index}`**, **`split_tab {zone, x, y, snap}`** (a tab dragged out), **`next_tab {zone, backwards}`** |
| Events | `zone_changed` for each tab concerned; `zone_placed` after a split |
| Scene | `ZoneScene.tab_group`, `.front`, **`.tab: {index, count}`** |

```json
{"command": "move_tab", "zone": "zone-3", "index": 0}
```
```json
{"command": "split_tab", "zone": "zone-3", "x": 640, "y": 300}
```
```json
{"command": "next_tab", "zone": "@focused", "backwards": true}
```

A group whose geometry is locked (any of its tabs, or all zones) lets no
tab in and none be split off. `next_tab` takes the keyboard along.

### Quick-hide

`quick_hide` is as it was, with two differences: it is **remembered**
(`ZonesState.quick_hidden`: a desktop the person cleared is still clear
after a restart), and there are exclusions per zone and per item.

| | |
|---|---|
| Config | `always_visible` per zone, as before |
| State | `quick_hidden`, `always_visible_items` |
| Commands | `quick_hide {hidden?}`, **`always_visible {zone, on?}`**, **`item_always_visible {item, on?}`** |
| Events | `visibility_changed {quick_hidden, peeking}`, `zone_changed`, `items_changed` |
| Scene | `Scene.quick_hidden`, `ZoneScene.shown`, **`ZoneScene.here`** (not hidden on these displays and of the shown workspace: `here` and not `shown` is "put away with the one key"), `ZoneScene.always_visible`, `PlacedItem.always_visible` |

```json
{"command": "item_always_visible", "item": "/home/ada/Desktop/todo.txt", "on": true}
```

While `quick_hidden` and not `peeking`, the shell draws: zones with
`shown`, and of everything else only the items whose `always_visible` is
set (bare, without their zone). An item's flag follows it through a
rename and is dropped when the item goes.

### Peek

| | |
|---|---|
| Commands | `peek {on}` (a held key), **`toggle_peek`** (a pressed key), **`dismiss`** (Escape), **`open {items, with?, in_place}`** |
| Events | `visibility_changed`, **`open_requested {items, with?}`** |
| Scene | `Scene.peeking` |

The core says when peeking ends: with `peek {on: false}`, `toggle_peek`,
`dismiss`, and **after an `open` while peeking** (the thing was found).
So that the core can know, items are opened through it:

```json
{"command": "open", "items": ["/home/ada/Desktop/report.pdf"]}
```
```json
{"event": "open_requested", "items": ["/home/ada/Desktop/report.pdf"]}
```

The host starts the items when it hears `open_requested` (the shell: by
the backend, or on Linux by the window manager, as today). One folder of
a portal is gone into instead, in place, unless `"in_place": false`.

`dismiss` does the nearest of: stop peeking; empty the filter of the
focused zone; choose nothing; let the keyboard go.

```json
{"command": "dismiss"}
```

Peeking shows what quick-hide put away. It does not show zones hidden on
these displays (`show_zone`) or zones of other workspaces.

### Lock

| | |
|---|---|
| Config | `lock: {geometry, members}` per zone as before; **`desktop.zones.lock`** for all zones |
| Commands | `lock {zone, geometry?, members?}`, **`lock_all {geometry?, members?}`** (neither given: both to the other of what they are) |
| Events | `zone_changed`; `settings_changed` for `lock_all`; **`refused_by_lock {zone, geometry, members}`** right after the `refused` of a command a lock refused |
| Scene | **`ZoneScene.lock`** (the zone's or the general one, whichever is locked), **`Scene.lock`** (the general one) |

```json
{"command": "lock_all", "geometry": true, "members": true}
```

```json
{"event": "refused_by_lock", "zone": "papers", "geometry": false, "members": true}
```

**Every refusal says what it was about.** After the `refused` of any
command that names a zone or an item comes `refused_about {reason, zone?,
item?, locked}`: the zone and item of the command (with `"@focused"`
already looked up; for a lock's refusal the zone that is locked; of
several items the first), and the reason again. So a refusal that came
by a key or from outside can be shown on the right zone.

```json
{"event": "refused_about", "reason": "papers is locked: nothing goes in or out",
 "zone": "papers", "item": "/desk/a.txt", "locked": true}
```

Locked geometry: no move, no resize, no tabbing, no arrangement copied
over. Locked members: nothing in or out by hand, by rules (held with
`locked`), or by the bin.

### Appearance

`ZoneLook`, for all zones (`desktop.zones.look`) and per zone (`look`).
Every value is optional; an absent one is taken from the look of all
zones, and one absent there follows the theme.

| Key | |
|---|---|
| `background` | `#rrggbb` or `#rrggbbaa` |
| `opacity` | hundredths, 0 to 100 |
| `blur` | what is behind is blurred, where the system can |
| `corner_radius` | 0 to 64 |
| `title_bar` | **new**: `full`, `plain` (the title alone), `hidden` |
| `title_colour` | a colour |
| `title_on_hover` | the title only while the pointer is over the zone |
| `labels` | **new**: the items' names are shown |
| `icon_tint` | **new**: a colour laid over the icons |
| `icon_saturation` | **new**: hundredths, 0 (grey) to 100 |

```json
{"look": {"opacity": 70, "blur": true, "corner_radius": 10},
 "zones": [{"id": "media", "title": "Media",
            "look": {"background": "#1b1f2acc", "title_bar": "plain", "labels": false,
                     "icon_tint": "#ffffff20", "icon_saturation": 60}}]}
```

`ZoneScene.look` is the zone's look laid over the general one
(`ZoneLook::over`); what is still absent there is the theme's to decide.
Colours that are no colours and numbers out of range are problems of the
configuration (`ZonesConfig::problems`, `ZoneLook::problems`). The look
is changed at run time with `modify_zone` / `modify_settings` (below).

### Changing the configuration at run time

Two commands change anything the configuration holds, by a JSON merge
patch (RFC 7386: objects are gone into key by key, `null` puts a key
back to its default). The result is held to the same rules as a
configuration file; a patch that cannot be right is refused with the key
and the reason, and changes nothing.

```json
{"command": "modify_zone", "zone": "media",
 "changes": {"look": {"opacity": 85, "blur": null}, "private": true, "workspaces": ["2", "3"]}}
```
```json
{"command": "modify_settings", "changes": {"snap": {"grid": 16}, "roll_up": {"expand_on": "click"}}}
```

`modify_zone` cannot change the `id`; `modify_settings` does not take
`zones`. Events: `zone_changed` + `items_changed` + `layout_changed`, or
`settings_changed` + `layout_changed`; both `Effect::SaveConfig`.

### Snapping

Verified, unchanged: `desktop.zones.snap` (`enabled`, `distance`, `gap`,
`grid`). A moved or sized zone is drawn to the free room's sides (the
room panels leave: `Displays.reserved`), the display's sides, the other
zones with `gap` between, and where nothing else pulls to multiples of
`grid` counted from the free room's corner.

```json
{"snap": {"enabled": true, "distance": 12, "gap": 8, "grid": 16}}
```

## Milestone 4

### Folder portals

A portal (`kind: {type: "portal", directory}`) mirrors a folder. It can
now walk into the folders it shows without leaving the zone. Where it
stands is view state; the configured folder is its root and nothing
above the root is ever shown.

| | |
|---|---|
| Commands | **`enter_folder {zone, item}`**, **`navigate_to {zone, directory}`** (a part of the way), **`navigate_up`**, **`navigate_back`**, **`navigate_forward`** `{zone}`, **`refresh_zone {zone}`**, **`new_folder {zone, name}`**, **`move_items_to_folder {items, folder, copy}`**, **`filter {zone?, typed}`**; `add_item`, `rename_item`, `trash_items`, `bring_in`, `undo_file_operation` as before |
| Events | **`navigated {zone, directory}`**, **`listing_changed {zone, listing}`**, **`filter_changed {zone, typed}`**, `items_changed`, `operation_failed` |
| Scene | **`ZoneScene.portal`**: `root`, `directory`, `crumbs: [{name, directory}]`, `can_go_up`, `can_go_back`, `can_go_forward`, `listing` |
| Backend | `Watch`/`Unwatch` follow the folder the portal stands in; **`Relist {directory}`**; **`BackendEvent::ListingFailed {location, error}`**; **`ZonesBackend::begin_listing`** |

```json
{"command": "enter_folder", "zone": "downloads", "item": "/home/ada/Downloads/2026"}
```
```json
{"event": "navigated", "zone": "downloads", "directory": "/home/ada/Downloads/2026"}
```
```jsonc
// ZoneScene.portal
{"root": "/home/ada/Downloads", "directory": "/home/ada/Downloads/2026",
 "crumbs": [{"name": "Downloads", "directory": "/home/ada/Downloads"},
            {"name": "2026", "directory": "/home/ada/Downloads/2026"}],
 "can_go_up": true, "can_go_back": true, "can_go_forward": false,
 "listing": {"state": "loading"}}
```

**Slow and missing folders.** `listing` is `loading` from the moment the
folder is asked for until the backend answers, `ready` then, and
`unavailable` with a reason when it could not be listed:

```json
{"event": "listing_changed", "zone": "films", "listing": {"state": "unavailable", "reason": "the system refused: //nas/films does not answer"}}
```

The zone stays where it is in every case, and shows no items while its
folder is not `ready`.

What backends do for that:

- `fn begin_listing(&mut self, location) -> bool` (default `false`): a
  backend that lists off-thread returns `true` and later puts `Listed` or
  `ListingFailed` into `drain_events`. Returning `false` means the host
  calls `list` at once, as before. **Use `begin_listing` for anything
  that may be a network or cloud folder**, or the shell waits.
- A folder that goes away while watched: send
  `BackendEvent::ListingFailed`. When it is back: `Listed`.
- `list` returning `Err` is now told to the core as `ListingFailed`, not
  as an empty folder.

```json
{"event": "listing_failed", "location": {"at": "directory", "path": "//nas/films"},
 "error": {"error": "not_found", "what": "//nas/films"}}
```

`refresh_zone` asks again (`BackendRequest::Relist`); the host may send
it on a schedule for zones that are `unavailable`.

**Filter.** Search-as-you-type inside one zone; it works for every kind
of zone, and is view state.

```json
{"command": "filter", "zone": "downloads", "typed": "inv"}
```

`ZoneScene.filter` is what is typed, `ZoneScene.items` what it leaves,
`ZoneScene.item_count` how many the zone holds without it. Walking to
another folder empties the filter. Without `zone` the focused zone is
meant.

**File operations.** All go through the backend (`BackendRequest::
Perform`), never delete (the bin), and can be taken back one by one with
`undo_file_operation`, the last first (32 deep, not kept over a restart).

```json
{"command": "new_folder", "zone": "downloads", "name": "Invoices"}
```
```json
{"command": "move_items_to_folder", "items": ["/home/ada/Downloads/a.pdf"], "folder": "/home/ada/Downloads/Invoices"}
```

A folder on the desktop itself (or anywhere that is no portal) is made
with `new_folder_in`, by the folder's path (`ZonesBackend::desktop_folder`
for the desktop), and is taken back the same way:

```json
{"command": "new_folder_in", "folder": "/home/ada/Desktop", "name": "Projects"}
```

- An item dropped on a portal (`add_item`) is moved into the folder the
  portal **stands in**.
- An item of a portal dropped on a folder inside it, or dragged out onto
  the desktop: `move_items_to_folder` (with the desktop's own folder,
  `ZonesBackend::desktop_folder`, for the latter).
- Files from another program dropped on a portal: `bring_in` with
  `folder` = `ZoneScene.portal.directory`.

**Operations that take long.** A large copy must not hold the caller
(the shell draws on that thread). A backend may carry an operation out by
itself and say later how it ended:

| | |
|---|---|
| Backend | **`ZonesBackend::begin_operation(request, &operation) -> bool`** (default `false`: `perform` is called, as before); **`cancel_operation(request)`**; **`BackendEvent::OperationProgress {request, done, total}`**; the end is the existing `OperationDone {request, undo?, error?}`; **`BackendError::Cancelled`** |
| Requests | `Perform` as before; **`Cancel {request}`** |
| Commands | **`cancel_operation {request}`** |
| Events | **`operation_begun {request, operation}`**, **`operation_progress {request, done, total}`**, **`operation_ended {request}`** (after an `operation_failed` if it did not end well) |
| Scene | **`Scene.operations`**: `[{request, operation, done, total, zones, item?}]`; **`ZoneScene.busy`** |

```json
{"event": "operation_begun", "request": 7,
 "operation": {"operation": "copy", "from": "/media/usb/film.mkv", "to_folder": "/home/ada/Desktop"}}
```
```json
{"event": "operation_progress", "request": 7, "done": 314572800, "total": 1073741824}
```
```json
{"command": "cancel_operation", "request": 7}
```
```json
{"event": "operation_done", "request": 7, "error": {"error": "cancelled"}}
```

- `Host` asks `begin_operation` first; on `true` it tells the core
  `BackendEvent::OperationBegun {request}` and returns at once. The
  backend then puts `OperationProgress` (as often as it likes; `total` 0
  for "not known"; any unit) and at last `OperationDone` into
  `drain_events`, exactly as `perform` would have answered: with the
  operation that takes it back, or with the error.
- Until then the core keeps what the operation is for: where a file
  brought in goes once it is there, which rule moved a file. All of that
  happens when `OperationDone` comes, as it did when it came at once; the
  undo is recorded then.
- Called off: the backend answers `OperationDone` with `cancelled`
  (having put back what it could), or with its result if it was too late.
  The core says `operation_failed` with that error and `operation_ended`;
  a file a rule was moving is shown in its zone again.
- If the program ends while one is under way, nothing of it was written
  anywhere: the next start shows what the backend lists, no more. A copy
  that the system finishes all the same arrives as any file does.
- Request numbers begin anew with every run of the program. A backend
  answers only operations of its own run (it ends with the program, and
  its threads with it).
- An operation the backend does at once is never in the scene.

### Rules

First match wins, catch-all, every matcher (`extension`, `mime`, `name`,
`kind`, `date`, `size`, `shortcut_target`, `untouched`), `preview_rules`,
`apply_rules`, `undo_rules` and rules that move the file itself
(`move_to_folder`, through the backend, taken back by `undo_rules`) are
as in milestone 1. Added:

**A dry run a person can read.** `rules_previewed` and `rules_applied`
carry `lines` beside the `plan`: one line for each step, then one for
each item held back.

```json
{"command": "preview_rules", "scope": "all"}
```
```json
{"event": "rules_previewed",
 "plan": {"steps": [{"item": "/desk/cat.png", "decided": {"rule": 0},
                     "action": {"action": "assign_to_zone", "zone": "pictures"}}],
          "held": []},
 "lines": [{"item": "/desk/cat.png", "name": "cat.png", "from": "inbox",
            "destination": {"to": "zone", "zone": "pictures", "title": "Pictures"},
            "decided": {"rule": 0}, "rule": "Pictures"},
           {"item": "/desk/old.doc", "name": "old.doc",
            "destination": {"to": "folder", "folder": "/home/ada/Archive"},
            "decided": "catch_all", "rule": "", "held": "not_a_file"}]}
```

`held` is one of `no_such_zone`, `zone_fills_itself`, `locked`,
`not_a_file`. A dry run changes nothing; applying does exactly the plan.

**Rules that ask the time.** A rule is *timed* when one of its matchers
is `untouched` or a `date` with `newer_than_days` / `older_than_days`:
it comes true by itself as time passes.

```json
{"rules": [{"name": "Stale", "matchers": [{"match": "untouched", "days": 30}],
            "action": {"action": "assign_to_zone", "zone": "archive"}}],
 "timed_rules": {"every_minutes": 60, "move_files": false},
 "zones": [{"id": "archive", "title": "Archive"}]}
```

The core owns no timer. The host asks `ZonesCore::timed_rules_every()`
(seconds; `None` while there is no timed rule or `every_minutes` is 0;
also in the snapshot as `timed_rules_every`) and sends on that schedule:

```json
{"command": "check_timed_rules"}
```

It looks at **every** item of the desktop, also those sorted by hand (an
item put into a zone a month ago and not touched since is what such a
rule is for), and carries out only what a timed rule decides. Rules that
move the file itself are carried out unasked only with
`timed_rules.move_files`. When nothing has come of age it says nothing
and changes nothing; otherwise `rules_applied`, and the application can
be taken back with `undo_rules` like any other.

### Smart zones

A smart zone (`kind: {type: "smart", query: [matchers]}`) shows what its
query holds for, without taking anything from where it is. New: what the
query is asked **of** (`source`), and how many it shows (`limit`).

| `source` | | Who tells the core |
|---|---|---|
| `{"from": "desktop"}` | the desktop's items (the default; an empty query shows nothing) | as before |
| `{"from": "folder", "directory": ...}` | any folder | `Watch`, as for a portal |
| `{"from": "downloads"}` | the person's downloads folder | the backend: `watch_source`, `list_source` |
| `{"from": "recent"}` | the files used last, the newest first | the backend |
| `{"from": "pinned"}` | the pinned launchers | the backend, or the host |
| `{"from": "running_apps"}` | the programs that have a window | the host (it knows the windows) |

```json
{"zones": [
  {"id": "today", "title": "Downloaded today", "kind": {"type": "smart", "query": [
      {"match": "date", "field": "modified", "newer_than_days": 1}]},
   "source": {"from": "downloads"}, "sort": "date", "sort_descending": true},
  {"id": "recent", "title": "Recent", "kind": {"type": "smart", "query": []},
   "source": {"from": "recent"}, "limit": 12},
  {"id": "running", "title": "Running", "kind": {"type": "smart", "query": []},
   "source": {"from": "running_apps"}},
  {"id": "pins", "title": "Pinned", "kind": {"type": "smart", "query": []},
   "source": {"from": "pinned"}, "view": "list"}]}
```

What passes between core and backend:

```json
{"ask": "watch_source", "source": {"from": "recent"}}
```
```json
{"event": "source_listed", "source": {"from": "recent"}, "items": [
  {"id": "/home/ada/notes.txt", "kind": "file", "name": "notes.txt", "path": "/home/ada/notes.txt",
   "facts": {"extension": "txt", "modified": 1800000000}}]}
```

- `BackendRequest::WatchSource {source}`: the host calls
  `ZonesBackend::watch_source` and `list_source` (both default to
  `Unsupported`) and tells the core `SourceListed` or `SourceFailed`.
  `UnwatchSource` when no zone looks through the source any more;
  `ListSource` on `refresh_zone`.
- A source is always told **whole**, in its own order
  (`BackendEvent::SourceListed {source, items}`), whenever it changes.
  With `sort: "manual"` that order is shown; any other sort re-orders.
- **The host may send `SourceListed` itself** (`core.event(...)`) for
  what it knows better than the backend: `running_apps` (one `Item` of
  kind `launcher` per program, its id whatever starts or raises it) and
  `pinned`. A backend that answers `Unsupported` does not overrule that.
- `ZoneScene.source` is the listing state of a source that is not the
  desktop (`loading`, `ready`, `unavailable`).

**Refresh.** Membership is computed every time the scene is, against
`now`: "today" moves on by itself. The scene should be made again at
least when the day changes; nothing else needs a timer.

**Never persisted.** What a smart zone shows is in no file: its items are
not placements, `add_item` into it is refused (`fills itself`), and the
items of sources are held by the core in memory only.

## Milestone 5

### Context-aware layouts

Arrangements keyed by the set of displays (stable ids, sizes, scales;
optionally the workspace), chosen automatically on dock, undock,
resolution and scale change, with positions relative to a display's
corner or edge: as in milestone 1 (`profile.rs`). Added:

**Zone sets per workspace**, in two ways that work together:

- Declared: `workspaces` per zone, the names of the workspaces the zone
  is shown on (empty: all). This holds whatever the displays are.
- Per arrangement: `show_zone {zone, shown}` hides a zone in the
  arrangement in use; with `desktop.zones.profile_per_workspace` each
  workspace has an arrangement of its own, and with it its own set. A
  new arrangement for a workspace takes over which zones that workspace
  hides from the workspace's other arrangements, not from another
  workspace's.

```json
{"profile_per_workspace": true,
 "zones": [{"id": "code", "title": "Code", "workspaces": ["dev", "2"]}]}
```

The host tells the workspace with `BackendEvent::Workspace {name}`
(unchanged). A workspace that was never told hides nothing.

**An arrangement taken over** from another set of displays:

```json
{"command": "copy_arrangement", "from": {"displays": [{"id": "edid:DEL-A1", "width": 2560, "height": 1440, "scale": 1000}]}}
```

The keys of the arrangements that are kept are in the snapshot
(`profiles: [{key, zones, current}]`).

### Layout snapshots, export and import

`save_snapshot`, `restore_snapshot`, `delete_snapshot` as before. Added:
`rename_snapshot`, export and import, and one guarantee: **whatever
replaces the whole layout keeps the one there was first**, as a snapshot
the zones take themselves (`automatic: true`, named `Before restoring` or
`Before import`), unless a snapshot already holds exactly that layout.
The last 5 automatic ones are kept; snapshots a person saved are never
dropped by the core.

```json
{"command": "export_layout", "name": "Work"}
```
```json
{"event": "layout_exported", "document": {
  "format": "gevrek-zones-layout", "version": 1, "exported_at": 1800000000,
  "layout": {"name": "Work", "taken_at": 1799990000,
             "zones": [{"id": "papers", "title": "Papers"}], "rules": [],
             "placements": {"zones": {"papers": {"items": ["/desk/a.pdf"]}}, "loose": {}},
             "profiles": [], "active_tabs": {}}}}
```

Without `name` the layout as it is now is exported. The host (the
command line) writes `document` to a file; the core touches no file.

```json
{"command": "import_layout", "apply": true, "document": {
  "format": "gevrek-zones-layout", "version": 1,
  "layout": {"name": "Work", "zones": [{"id": "papers", "title": "Papers"}]}}}
```
```json
{"event": "layout_imported", "name": "Work (2)", "applied": true, "kept_as": "Before import"}
```

Import looks the document over before anything is touched: `format`
must be `gevrek-zones-layout`, `version` at most 1, the layout named,
its zones and rules free of what a configuration may not hold (ids
twice, sizes out of range...), no item in two zones. A document that
fails is refused with the reason and changes nothing. One that passes is
kept as a snapshot under its own name (with a number if that name is
taken: importing never replaces a snapshot) and, unless `"apply": false`,
put in force. Arrangements in it for displays that are not here are kept
for when they are; the zones come to the displays that are here as on
any change of displays.

### Tiling awareness

A zone lies under the windows unless it says `reserves_space`. Then the
core says which room tiled windows are to keep off:

```json
{"zones": [{"id": "dock", "title": "Dock", "reserves_space": true}]}
```
```json
{"event": "reserved_changed", "regions": [
  {"display": "edid:DEL-A1", "rect": {"x": 0, "y": 40, "width": 1920, "height": 120},
   "edge": "top", "thickness": 120, "zones": ["dock"]}]}
```

- One region for each place (tabs are one). `rect` is where the zone is
  drawn; `edge` and `thickness` say the same as a strip at a side of the
  display's **free room** (what panels leave), which is how window
  managers keep room: the side from which the zone reaches in least.
- Zones that are not shown here (hidden on these displays, of another
  workspace, put away by quick-hide) keep no room. A rolled-up zone keeps
  its title's room, also while open for a moment; peeking changes
  nothing.
- `reserved_changed` is told whenever the regions are other than last
  told, after any command or event. They are also `Scene.reserved` and
  in the snapshot (`reserved`). `reserve::left_over(area, display,
  regions)` gives what is left of a free room.
- **The host must not feed this back** as `Displays.reserved`: the room
  told to the core is the room panels keep, without the zones' own.
  Otherwise each zone would be pushed out of the room it reserved.

### Keyboard

What has the keyboard is kept by the core, so that keys, `gevrek zones`
and a screen reader mean the same thing. The keyboard is on a *place*
and perhaps on one item of it. A place is a zone (a shown one; of tabs
the one in front) or **the bare desktop of a display**: its loose items,
while there are some and they are to be seen (not put away by
quick-hide, not covered). The desktop is named **`"@desk"`** wherever a
zone is named for the keyboard.

| | |
|---|---|
| Commands | **`focus_zone {zone?}`**, **`focus_neighbour {direction}`**, **`focus_item {item?}`**, **`move_item_focus {step, extend}`**, **`select {items, mode}`**, **`open_focused`**, **`move_focused_items {direction?, zone?}`**, `filter {typed}` (the focused zone), `dismiss`, and any zone command with `"@focused"` |
| Events | **`focus_changed {zone?, item?}`**, **`selection_changed {items}`** |
| Scene | **`Scene.focus: {zone, item?}`**, **`Scene.selection`**, **`ZoneScene.focused`**, **`PlacedItem.focused`**, **`PlacedItem.selected`** |

`direction` is `left`, `right`, `up`, `down`; `step` is those and
`first`, `last`; `mode` is `replace`, `add`, `remove`, `toggle`.

```json
{"command": "focus_neighbour", "direction": "right"}
```
```json
{"command": "move_item_focus", "step": "down", "extend": true}
```
```json
{"command": "move_focused_items", "direction": "left"}
```
```json
{"command": "select", "items": ["/desk/a.txt", "/desk/b.txt"], "mode": "toggle"}
```
```json
{"event": "focus_changed", "zone": "papers", "item": "/desk/a.txt"}
```

**The bare desktop.**

```json
{"command": "focus_zone", "zone": "@desk"}
```
```json
{"event": "focus_changed", "zone": "@desk", "item": "/desk/loose.txt"}
```
```json
{"command": "move_focused_items", "zone": "@desk"}
```
```jsonc
// Scene.focus (and the snapshot's `focus`) on the desktop of a display
{"zone": "@desk", "desk": "edid:DEL-A1", "item": "/desk/loose.txt"}
```

- `Focus` has a third field, `desk`: the display whose loose items have
  the keyboard; `zone` is `"@desk"` then. `LooseScene.focused` says it per
  display, `PlacedItem.focused` / `.selected` per loose item.
- `focus_neighbour` goes between zones and desktops alike, by where they
  are drawn; a display's desktop counts as the smallest rectangle that
  holds its loose items. `focus_zone {"zone": "@desk"}` goes to the first
  display that has loose items.
- `move_item_focus` on the desktop goes to the nearest loose item in the
  direction (they lie where they were put: no rows, so no running on
  from the end of one); `first` / `last` are by name.
- `focus_item` of a loose item goes to its display's desktop.
- `move_focused_items`: from the desktop into a zone (named, or lying
  that way) is `add_item`; from a zone onto the desktop
  (`"zone": "@desk"`, or a desktop lying that way) takes the items out of
  their zone, to lie with the other loose items. The keyboard stays where
  it was, on what is left.
- **`"@focused"` means a zone.** While the desktop has the keyboard, a
  command that names `"@focused"`, and `filter` without a zone, are
  refused with `the desktop has the keyboard, not a zone`. `"@desk"` is
  understood by `focus_zone` and `move_focused_items` only; any other
  command that names it is refused as naming no zone.
- The desktop loses the keyboard when its last loose item goes, when it
  is put away (quick-hide; peeking gives it back for as long as it lasts)
  and when the loose items are covered.

- `focus_neighbour`: the zone nearest in that direction by where the
  zones are drawn (straight ahead before aside); from no focus, the zone
  nearest the top left. At the desktop's side the focus stays.
- `move_item_focus`: through the zone's items as they are laid out; left
  and right run on over the ends of rows as reading does. The item walked
  to is the chosen one; with `extend` it is added to the chosen ones.
  From no focus at all it begins at the first zone's first item.
- `focus_item {item}`: finds the zone that shows the item, brings its tab
  to the front, opens it if rolled up, empties a filter that hides the
  item. This is how a search hit is gone to.
- `open_focused`: `open` for the chosen items, or the focused one.
- `move_focused_items`: `add_item` for each into the named zone or the
  neighbour in a direction; the keyboard stays in the zone it was in, on
  what comes to stand where it was.
- Roll up, lock, hide the focused zone: the ordinary commands with
  `"zone": "@focused"`.
- Type-to-filter: `{"command": "filter", "typed": "..."}`.

The focus is always on something that exists: when its zone goes or is
hidden the focus is dropped, when its item goes the zone keeps it, and
`focus_changed` says so.

### Global search

```json
{"command": "search", "typed": "inv"}
```
```json
{"event": "found", "typed": "inv", "hits": [
  {"item": "/home/ada/Downloads/invoice.pdf", "name": "invoice.pdf", "zone": "downloads",
   "score": 889, "also_in": ["today"], "kind": "file", "path": "/home/ada/Downloads/invoice.pdf"}]}
```

Searched: every item of the desktop (loose or in a zone), the folder
each portal **stands in**, and what every smart zone shows (whatever its
source). Ranked as the launcher ranks (whole name, beginning, beginning
of a word, any part, letters in order), equal ones by name. Each hit
says the zone it is in (`zone`; absent for a loose item) and the smart
zones that show it as well (`also_in`). What a covered zone holds is not
found; a filter typed into a zone hides nothing from the search.

### Privacy

| | |
|---|---|
| Config | per zone **`private`**; **`desktop.zones.privacy.everything`** (all zones and the loose items are private); **`.when_shared`**: `nothing`, `private` (the default), `everything` |
| Commands | **`mask {masked?}`** (cover everything by hand, or stop), **`reveal {zone?, revealed?}`** |
| Events | **`privacy_changed {masked, screen_shared}`**, `zone_changed`, `layout_changed` |
| Scene | **`ZoneScene.masked`**, `.private`, **`LooseScene.masked`**, **`Scene.masked`** (by hand), **`Scene.screen_shared`** |
| Backend | **`BackendEvent::ScreenShared {shared}`**; **`ZonesBackend::screen_shared() -> Option<bool>`** (asked once at the start; default `None`) |

```json
{"privacy": {"everything": false, "when_shared": "everything"},
 "zones": [{"id": "bank", "title": "Bank", "private": true}]}
```
```json
{"event": "screen_shared", "shared": true}
```
```json
{"command": "reveal", "zone": "bank"}
```
```json
{"command": "mask", "masked": true}
```

- A private zone is covered until it is revealed. Revealing lasts until
  the screen is next shared, everything is covered by hand, or the shell
  restarts.
- When the host says the screen is shared: what was revealed is covered
  again (`private`), or everything is (`everything`), or nothing changes
  (`nothing`). On Linux the compositor knows (it implements the
  screencast portal) and tells the shell, which sends the event; on
  Windows nothing reliable says so, and `mask` by a key is the way.
- **A covered zone's items are not in the scene**: `ZoneScene.items` is
  empty and `item_count` says how many there are. So nothing that is
  covered can be drawn, read out by a screen reader, found by the search
  or focused by mistake. The snapshot for `gevrek zones list` does name
  them (the person asked) and says `masked`.
- `reveal` without a zone reveals everything and ends a mask by hand.

### IPC

`ipc::VERSION` stays 1: fields were added, none changed meaning. Every
command above is a `Command` and so a `Request`; every event is a
`ZonesEvent`. New in `ZonesSnapshot`: `masked`, `screen_shared`, `locked`,
`workspace`, `focus`, `selection`, `snapshots: [{name, taken_at, zones,
automatic}]`, `profiles: [{key, zones, current}]`, `reserved`,
`can_undo_rules`, `timed_rules_every`, `operations` (as `Scene.operations`;
left out while none is under way). New in each `ZoneSnapshot`:
`expanded`, `masked`, `private`, `reserves_space`, `focused`,
`workspaces`, `filter`, `portal`, `source`, `listing`; `locked` now
includes the lock for all zones, and `item_count` is what the zone holds
while `items` is what its filter leaves.

What `gevrek zones` needs, and the command for it:

| | |
|---|---|
| list | the snapshot |
| create, delete, rename | `create_zone`, `delete_zone`, `rename_zone` |
| modify | `modify_zone`, `modify_settings`, `set_view`, `lock`, `lock_all` |
| show, hide | `show_zone`, `quick_hide`, `always_visible`, `peek`, `toggle_peek` |
| roll up | `roll_up`, `expand` |
| apply a layout | `restore_snapshot` (a saved one), `copy_arrangement` (another display set's), `import_layout` (a file) |
| save, export | `save_snapshot`, `rename_snapshot`, `delete_snapshot`, `export_layout` (the answer's `layout_exported.document` is the file) |
| rules, dry run | `preview_rules` (the answer's `rules_previewed.lines`) |
| rules, for real | `apply_rules`, `check_timed_rules`, `undo_rules` |
| zones by type | `auto_create` (with `dry_run`: the answer's `auto_planned.plan`), `undo_auto_create` |
| grey icons | `grey_icons` |
| subscribe | the event stream; names by `ipc::event_name` |

### Accessibility

The scene now carries what a screen reader says, and `access::tree(&scene)`
turns it into a tree of nodes (`AccessNode`: `id`, `role`, `name`,
`description`, `value`, `rect`, `focused`, `selected`, `locked`,
`expanded`, `masked`, `hidden`, `position`, `of`, `children`) for the
shell to hand to AccessKit.

- Each `PlacedItem` has `name`, `kind`, `position` and `of` (which one of
  how many, from 1), `selected`, `focused`, `always_visible`.
- Each `ZoneScene` has `title`, `item_count`, `focused`, `lock`,
  `rolled_up`, `expanded`, `masked`, `tab: {index, count}`, `filter`,
  `portal` (the way, for "folder Downloads, 2026"), and `content`: the
  rectangle on the desktop its items are laid out in, so an item's place
  on the desktop is `content` plus its own `rect` (minus the scroll,
  which is the renderer's).
- Roles: `desktop` (root), `zone`, `loose` (a display's loose items),
  `item`. Ids stay the same from scene to scene (`zone:<id>`,
  `zone:<id>/item:<item id>`, `loose:<display>`).
- A zone's `expanded` is absent unless it is rolled up; then it says
  whether its items are to be seen. Items of a closed or hidden zone are
  `hidden`. A zone's `position`/`of` count the zones that can be walked
  to. Its `description` is words: "tab 2 of 3, 12 items, locked".

```jsonc
// one node of access::tree
{"id": "zone:papers", "role": "zone", "name": "Papers", "description": "3 items, locked",
 "rect": {"x": 300, "y": 100, "width": 400, "height": 300}, "focused": true, "locked": true,
 "position": 1, "of": 4, "children": [
   {"id": "zone:papers/item:/desk/a.pdf", "role": "item", "name": "a.pdf",
    "rect": {"x": 308, "y": 136, "width": 76, "height": 82}, "selected": true, "position": 1, "of": 3}]}
```

### Persistence

The state file stays **version 1**. What was added to it
(`quick_hidden`, `always_visible_items`, a snapshot's `automatic`) are
new keys with defaults, each left out of the file while it is at its
default: a file from before reads as it is, and a gevrek from before
keeps the new keys untouched (unknown keys are kept and written back).
A bump would make the older gevrek set the whole file aside as "from a
newer version" and start from a backup, which loses more than it
protects. The migration hook (`store::migrate`, `STEPS`) is unchanged
and takes the first step that really changes a shape.

Never losing a layout, as far as the core goes: restoring and importing
keep what there was (above); a document or a patch that cannot be right
changes nothing; smart membership, filters, focus, navigation and
"open for now" are in no file.

## After milestone 5

Three additions, all of the same making as what is above: commands in,
events out, nothing drawn here.

### Icons drawn grey

One switch for the whole desktop, remembered like quick-hide.

| | |
|---|---|
| State | `ZonesState.grey_icons` (left out of the file while off) |
| Commands | **`grey_icons {on?}`** (without a value: the other of what it is) |
| Events | **`grey_icons_changed {on}`** + `Effect::SaveState`; nothing at all when it already is that |
| Scene | **`Scene.grey_icons`** |
| Snapshot | **`grey_icons`** |
| Core | `ZonesCore::grey_icons()` |

```json
{"command": "grey_icons", "on": true}
```
```json
{"event": "grey_icons_changed", "on": true}
```

It changes **no look**: `look.icon_saturation` of every zone and of all
zones stays as the person set it. The renderer lays grey over whatever
the looks say while `Scene.grey_icons` is set, for the items of every
zone and the loose ones.

### The middle button twice on the bare desktop, and whether loose items are away

Configuration only; the gesture itself is the shell's.

| | |
|---|---|
| Config | **`desktop.zones.gestures.double_middle_click`** (`grey_icons`): `none`, `quick_hide`, `grey_icons` or `peek` |
| Scene | **`Scene.loose_hidden`** = `quick_hidden` and not `peeking` |
| Core | `ZonesCore::loose_hidden()` |

```json
{"gestures": {"double_middle_click": "none"}}
```

Quick hide is as it was: the loose items go with the zones, and twice
with the main button on the bare desktop is `quick_hide`, always (it is
not a setting). `Scene.loose_hidden` only says in one word what the
shell reckoned itself before: it draws the loose items unless it is set,
and while it is set only those with `PlacedItem.always_visible`. The
core goes by the same where it decides by itself: the bare desktop can
have the keyboard (`"@desk"`) exactly while its items are to be seen,
and `access::tree` marks loose items `hidden` by it.

The core does nothing with `gestures`: the shell reads it from
`ZonesCore::config()` and sends `quick_hide`, `grey_icons` or
`toggle_peek`.

### Zones made by type

A zone for each type of item on the desktop, with a plan first and one
command that takes it back. The reckoning is `crate::auto` (pure, with
tables); the core carries it out (`core/auto.rs`).

| | |
|---|---|
| Config | **`desktop.zones.auto`**: `min_items` (2; 1 to 1000), `add_rules` (true), `categories: [{id, title, extensions, mime_prefixes, kinds}]`; per zone **`auto`** (the id of the type it was made for; left out for a zone made by hand) |
| State | `ZonesState.auto_undo` (`AutoUndo {made_at, zones: [{zone, category}], entries: [{item, before, zone}], rules}`; left out while there is none) |
| Commands | **`auto_create {scope, min_items?, display?, add_rules?, dry_run}`**, **`undo_auto_create`** |
| Events | **`auto_planned {plan}`** (the dry run), **`auto_created {zones, items, rules}`**, **`auto_undone {zones, items, rules}`**; `zone_created`, `zone_deleted`, `items_changed`, `layout_changed` as usual |
| Snapshot | **`can_undo_auto`**; per zone **`auto`** |
| Core | `ZonesCore::auto_plan(scope, min_items, display, now) -> AutoPlan`, `can_undo_auto()` |

```json
{"auto": {"min_items": 3, "add_rules": false, "categories": [
   {"id": "notes", "title": "Notes", "extensions": ["md", "txt"], "mime_prefixes": ["text/"], "kinds": []},
   {"id": "links", "title": "Links", "extensions": [], "mime_prefixes": [], "kinds": ["shortcut", "launcher"]},
   {"id": "rest", "title": "Everything else", "extensions": [], "mime_prefixes": [], "kinds": []}]},
 "zones": [{"id": "zone-4", "title": "Notes", "auto": "notes"}]}
```

**The types.** An item is of the first type that names its kind; if
none does, of the first that names its extension (small letters, no
dot); if none does, of the first one of whose `mime_prefixes` its MIME
type begins with; and at last of the first type that names nothing at
all, which takes the rest. `ItemKind::Virtual` (This PC, the bin) is of
no type and never sorted. As they come (`AutoCategory::built_in()`):

| `id` | `title` | `kinds` | `extensions` | `mime_prefixes` |
|---|---|---|---|---|
| `folders` | Folders | folder | | |
| `programs` | Programs | shortcut, launcher | lnk url exe msi bat cmd desktop appimage | |
| `documents` | Documents | | pdf doc docx odt rtf txt md xls xlsx ods csv ppt pptx odp epub | text/ application/pdf application/msword application/vnd. |
| `pictures` | Pictures | | png jpg jpeg gif webp bmp svg tif tiff heic ico | image/ |
| `video` | Video | | mp4 mkv mov avi webm wmv m4v | video/ |
| `audio` | Audio | | mp3 flac wav ogg m4a aac opus wma | audio/ |
| `archives` | Archives | | zip 7z rar tar gz bz2 xz zst iso cab | |
| `code` | Code | | rs py js ts c h cpp hpp cs java go rb php sh ps1 json toml yaml yml xml html css sql | |
| `other` | Other | | | |

**The plan.**

```json
{"command": "auto_create", "scope": "loose", "dry_run": true}
```
```json
{"event": "auto_planned", "plan": {
  "zones": [
    {"category": "folders", "title": "Folders", "rect": {"x": 8, "y": 48, "width": 172, "height": 126},
     "items": ["/desk/Archive", "/desk/Projects"], "rule": true},
    {"category": "pictures", "title": "Photos", "zone": "zone-4",
     "rect": {"x": 900, "y": 600, "width": 412, "height": 384},
     "items": ["/desk/one.gif"], "rule": false}],
  "left": ["/desk/song.mp3"],
  "display": "edid:DEL-A1B2"}}
```

- `scope`: `loose` (the default) asks about the items in no zone;
  `all` also about the items of manual zones that have no `auto` (what
  is in a zone made by type stays there).
- A type whose zone exists (the first manual zone with its id in
  `auto`) gets what is new of it whatever the number: `zone` names it,
  `rect` is where it is drawn, `rule` is false. Any other type gets a
  zone when it has at least `min_items` items (`auto.min_items` where
  the command gives none; 0 counts as 1): `zone` is absent, `rect` is
  where it would be made. With fewer, its items are in `left`.
- `left` also holds items of no type (no type takes the rest) and of a
  type whose zone is locked to what it holds. Items in a locked zone are
  not asked about at all.
- `zones` empty is a plan like any other (there is nothing to do), not
  a refusal. Asking again after making gives exactly that.
- `display`: where new zones are made. As given; else the primary
  display, else the one with the least id (`geometry::fallback_display`).
- `rule`: a rule would be added (`add_rules`, `auto.add_rules` where
  the command gives none, and the type has a matcher that says it).
- A dry run changes nothing and asks for nothing to be saved. Through
  IPC the plan is in the answer's `events`, like `rules_previewed`.
- `ZonesCore::auto_plan` gives the same plan without a command (with
  `rule` by `auto.add_rules`). While no display is known, or the one
  named is not there, it gives an empty plan with no `display`; under
  the lock for all zones, one with everything in `left`.

**Where new zones go** (`auto::place`, `auto::size_for`). Each is sized
for what it holds at the default icon size: about as many columns as
rows, 2 to 5 columns, at most 4 rows to be seen (more is scrolled to).
They stand in rows from the top left corner of the display's free room,
`snap.gap` between them and to the room's sides, the tops of a row in
one line, past every zone that belongs to what is shown on that display
(a rolled-up one counts whole, one put away by quick-hide too), and past
every loose item of that display that the plan leaves loose: each such
item's cell, where the loose items will lie once the others have gone
into their zones, is kept clear as a zone's rectangle is
(`ZonesCore::auto_plan` plans twice for it: once to see what goes, then
around what stays). Where a
zone has no room at its size it is made as small as a zone for two
icons; where there is no room for that either, the zones that are left
are laid over what is there, each 24 units down and right of the last,
still inside the room. So every planned zone has a place and the
command never fails for want of room.

**Making.**

```json
{"command": "auto_create", "scope": "all", "min_items": 3, "display": "edid:DEL-A1B2", "add_rules": false}
```
```json
{"event": "auto_created", "zones": ["zone-5", "zone-6"], "items": 9, "rules": 2}
```

Exactly the plan: each new zone is a manual one at its `rect`, titled as
its type, with `auto` set; the items are put at the end of their zones
in the order of their names. `zones` are the new ones, `items` all that
were moved (into new zones and into ones that were there), `rules` the
rules added. `Effect::SaveConfig` when a zone was made, `SaveState`
always. With nothing to do: `auto_created` with no zones and 0 items,
nothing saved, and what could be taken back before still can be.

**The rules it adds**, one for each new zone, at the end of
`desktop.zones.rules`, named `Auto: <title>`, `assign_to_zone`. A rule
holds when all its matchers do and a type is any of several things, so
the rule says one of them (`auto::rule_for`): the type's `extensions`
where it has some; else its `kinds`; else its MIME beginning where it
has exactly one. A type that names nothing (`other`) gets no rule: it
would match everything. What such a rule does not catch (a shortcut
without one of the extensions, a picture known by MIME alone) is caught
by `auto_create` again.

**Taking back.**

```json
{"command": "undo_auto_create"}
```
```json
{"event": "auto_undone", "zones": 2, "items": 9, "rules": 2}
```

One record is kept (`ZonesState.auto_undo`), of the last making that did
something. Every item it moved that is still in the zone it was put into
goes back where it was (its zone and place there, or loose at its own
place); the zones it made are deleted where they still carry that
`auto`; the rules it added are removed where one of that name and action
is still there. What the person changed since stays: an item put
elsewhere, a zone whose `auto` was taken off or that is locked to what
it holds (with its items), a rule that was renamed. Refused when there
is nothing to take back. Putting a whole layout in force (a snapshot
restored, a layout imported) drops the record.

**Locks and the switch.** With `desktop.zones.lock.members` both
commands are refused (`every zone is locked: nothing goes in or out`),
the dry run too. Switched off, they are refused like every command.

**Refusals:** `no display is known yet`, `no display is called <id>`,
`no zones were made by type, or it was taken back already`.

## Testing against the core

`testing::FakeBackend` (feature `test-support`) can be told what the new
requests are answered with:

| | |
|---|---|
| `add_folder_in(parent, name)`, `add_file_in(folder, name)` | a tree of folders for a portal to walk |
| `set_slow(folder, bool)`, `answer()`, `awaited` | a folder whose listing is only begun (`begin_listing`), and the moment its answers arrive |
| `set_reachable(folder, bool)`, `is_reachable(folder)` | a share that goes away (tells `ListingFailed` to whoever watches) and comes back (tells nothing: `refresh_zone` finds out) |
| `slow_operations`, `under_way`, `finish_operations()`, `progress(request, done, total)`, `abandon_operations(carried_out)` | file operations that are only begun, and how they end (or do not, when the program that began them is gone) |
| `set_source(source, items)` | what a smart zone's source has; a source never set is `Unsupported` |
| `share_screen(bool)`, `shared` | the system saying the screen is shared (`shared` is what `screen_shared()` answers at the start) |
| `is_watched(folder)`, `watches()` | what is watched now (folders and sources) |

The tests of the crate itself:

- tables beside each piece (`portal`, `focus`, `reserve`, `export`,
  `rules`, `core/layouts`, the model's `zones`);
- `later_scenarios`: each feature above, at work through the host on the
  backend in memory;
- `later_runs`: 36 seeded runs of 240 steps of all of it mixed (portals
  in a tree that changes, is slow and goes away; a source that changes;
  the pointer, the keyboard, tabs, hiding, covering, rules, layouts
  carried out and in), with these held after every step: nothing broken;
  no zone lost but by deleting; whatever replaced the layout kept the one
  there was; the keyboard on something shown or on nothing; a listed
  portal shows exactly its folder; nothing a zone shows by itself is in
  the state; a dry run changes nothing and equals applying, and undo
  gives the placements there were; what is covered is neither in the
  scene nor found; what is for the moment only asks for no save; the room
  kept is what was last told; everything told survives its JSON; a core
  started anew has the zones where they are and nothing of the moment;
  with file operations that take long begun, ended, failed and called
  off between, and the program started anew in the middle of the run
  while some are under way: every operation begun is under way until it
  ends and not after, the desktop the core knows is the desktop there is
  (no item more, none less), and a file brought in and taken back leaves
  the desktop as it was; the keyboard on the bare desktop only where
  loose items lie and are to be seen;
- `auto` (tables: of which type an item is, the rule of a type, sizes,
  places that never overlap and never leave the room, plans) and
  `auto_scenarios` (icons drawn grey, what the one key takes with it,
  zones by type planned, made, made again, taken back, under locks, on
  another display); `later_runs` also draws `auto_create` (dry, real,
  and mostly taken back), `undo_auto_create` and `grey_icons`, and holds:
  a plan changes nothing and is what making does; after making there is
  nothing left to make; taking back gives the zones, rules and
  placements there were; no type has two zones;
- `docs`: the examples of this page.

Two things of the core that tests of other crates may meet:

- Word of single items of a folder (`ItemAdded` and the like for a
  `Directory`) is taken only once the folder was listed whole
  (`listing` is `ready`). Before that the listing that comes will have
  them; after a `ListingFailed` only a new `Listed` counts.
- `ZonesEvent::LayoutExported.document` is a `Box<LayoutDocument>`.

## What each layer has to do

**Shell (drawing, pointer, keys).**
- Send `pointer_over` on every change of the zone under the pointer
  (`dragging` while something is carried), poll `wake_at_ms()` after each
  call and send `tick`. Send `expand` for a press on a rolled-up title.
- Open items with `Command::Open` (not by calling the backend directly),
  and start them on `open_requested`.
- Draw from the new scene fields: `expanded`, `tab`, `lock`, `masked`
  (a cover, and `item_count`), `portal` (the way, back/up, `listing`),
  `filter`, `focused`/`selected`, `look.title_bar`/`labels`/`icon_tint`/
  `icon_saturation`, `PlacedItem.always_visible` under quick-hide.
- Keys: Escape = `dismiss`; arrows = `move_item_focus` /
  `focus_neighbour`; letters = `filter`; Enter = `open_focused`.
- Hand `reserved_changed` to the window manager; never back to the core.
- Send `check_timed_rules` every `timed_rules_every()` seconds.
- Feed `SourceListed` for `running_apps` (and `pinned`, if the panel
  knows them better than the backend); send `BackendEvent::ScreenShared`
  when the compositor says so.
- Build the AccessKit tree from `access::tree(&scene)`.

**Backends (`crates/system/src/zones`).** All optional, all defaulted:
- `begin_listing` for folders that may be slow; `ListingFailed` when a
  watched folder goes away.
- `list_source` / `watch_source` for `recent`, `downloads`, `pinned`.
- `screen_shared` where the system can say.

**Compositor / window manager.** Keep the room of `reserved_changed`
free of tiled windows (per display: `edge` + `thickness` from the free
room's side, or `rect`); tell the shell when the screen is shared.

**Command line and studio.** The table under IPC. The studio's page
writes `desktop.zones` as any other section; the new keys are in the
schema with hints for their ranges.

**Bindings** (`crates/model/src/examples.rs`): every action has a
command; the ones that act on "this zone" take `"zone": "@focused"`.
