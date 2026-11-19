# Zones: design proposal

Status: **decided, nothing implemented.** Written 2026-11-19 against
`architecture-notes.md`; the decisions below were taken the same day.

## Decided (2026-11-19)

| Question | Decision |
|---|---|
| Explorer's icons (A) | A1: zones draw every item; Explorer's icon view is hidden from outside, with no injection, under a restore journal. Approved. |
| The name (B) | B1: the old feature becomes `desktop.snap_zones` (schema 26, CLI `snap-zone` with the old names as aliases); the new one takes `desktop.zones`. |
| Process and IPC (C) | The shell owns zones and publishes its state to the server. |
| Dependencies | The `notify` and `trash` crates (checked against `deny.toml` first); MIME and thumbnails by hand. |
| Stable display ids | EDID (maker, model, serial) read on both systems in milestone 1. |
| Linux items | The XDG Desktop folder is the source. |
| Loose items | A loose desktop surface lays them out as Explorer did. |
| Keys | Super+Z peek, Super+Shift+Z quick-hide, after a check against the bindings in use. |

The reasoning as it was put to the decision follows.

Three findings change the approach from what the brief assumed:

1. `explorer.exe` runs and owns the desktop icons (decision A).
2. The name "zones" already means snap zones in the configuration, the
   commands and the CLI (decision B).
3. The feature is UI that owns files, and the portable crates may hold
   neither; the shell cannot answer a query today (decision C).

## Decision A: Windows, living with Explorer's icons

Explorer draws the desktop's icons in its own list view. No window of
another program can lie behind those icons by any documented means.

| | A1: draw everything, put Explorer's view away | A2: leave Explorer's icons, move them | A3: behind the icons (`WorkerW`) |
|---|---|---|---|
| How | Zones and a "loose desktop" surface draw every item from the shell namespace. Explorer's icon view is hidden. | Explorer keeps drawing. Zone grounds are drawn by us; icons are placed into them through `IFolderView2` from outside. | A message to `Progman` makes a `WorkerW` behind the icons; our window is made its child. |
| Injection | none | none | none |
| Icon size, list view, tint, scrolling, tabs, roll-up, portals | all ours | not possible: one list view, one icon size, no clipping | as A2 |
| Zone ground behind the icons | yes (ours) | no: our window is above or below the whole list view | yes |
| If gevrek crashes | icons stay hidden until put back | icons stay where we put them | a stray child window |
| Stands on | documented shell interfaces, plus hiding one window of Explorer's | documented interfaces | an undocumented message that changed in Windows 11 24H2 |

**Recommended: A1.** It is the only one that carries the feature list, and
it shares all drawing with Linux. Its cost is the promise of principle 4:

- Before anything of Explorer's is touched, write a journal
  (`state/zones/explorer.json`): that the view was shown, and the icons'
  positions read through `IFolderView`. This follows the pattern of
  `hidden.json` and `windows-wallpaper.json`.
- Put back on exit, on the next start after a crash, by
  `gevrek zones restore-explorer`, and by the uninstaller.
- How the view is hidden (hiding the list view's window from outside, or
  Explorer's own "show desktop icons" switch) is to be tried on Windows 11
  in milestone 2 before it is settled. **This is the step that needs your
  approval**: it changes something of Explorer's, without injection.

Z-order: zone surfaces are shellkit `Below` surfaces (`HWND_BOTTOM`, one
per display), `Overlay` during peek. Risk to check early: "Show desktop"
(Win+D) raises Explorer's desktop above bottom-level windows. Windows'
virtual desktops are not integrated in gevrek; tool windows show on all of
them, and zones follow gevrek's workspaces instead.

## Decision B: the name

| | B1: rename the old feature | B2: another key for the new one | B3: another name |
|---|---|---|---|
| Config | `desktop.zones` → `desktop.snap_zones` in a migration to schema 26; the new feature takes `desktop.zones` | new feature at `desktop.icon_zones` (or similar); "Zones" in the UI only | e.g. "Shelves" everywhere |
| CLI | `zone` → `snap-zone` (old names kept as aliases for a while); `gevrek zones …` is new | `gevrek icon-zones …` | free |
| Cost | touches model, engine, proto names, CLI, studio, schema, guide | the code says one thing and the UI another, for good | not the name asked for |

**Recommended: B1.** The studio already calls the old feature "Snap
zones", and the project is before 1.0 with a migration path. A file written
by hand that is not migrated would be read wrongly, so the migration must
refuse a `desktop.zones` of the old shape under schema 26 rather than guess.

## Decision C: where the code lives

- **`crates/zones` (new, portable):** layout, snapping, rules, profiles,
  persistence, search, and the backend trait. No system API, no egui; added
  to the CI rule that guards `model` and `engine`. Configuration types go
  in `crates/model` with the others.
- **`crates/system` (existing):** the two backends (it already holds icons,
  programs and launching for both systems).
- **`crates/shell` + `crates/shellkit`:** the drawing, as one more shell
  component, and the new surface abilities (drag and drop).
- **Process:** the shell (`gevrek-shell`) owns the zones. It already owns
  egui, the surfaces and AccessKit. The WM stays the IPC endpoint.

What is missing for that: the shell can be told to act
(`DispatchCommand::Shell`) but cannot answer. Proposed: a small pair of
RPCs by which the shell publishes a numbered snapshot of the zones' state
and its events to the server, which serves `list` and the event stream
from it. Alternative: the core runs in the WM's process and the shell only
draws; rejected because every icon and file event would cross processes.

On Linux this makes zones a **layer-shell client of our own compositor**
(Bottom layer; Overlay for peek), not something the compositor draws. One
drawing code for both systems; blur comes from the existing layer rule.
There is no X11 path: gevrek is not an X11 window manager.

## Data model

```
Item        id (stable: shell parsing name / file URI), kind (file, folder,
            shortcut, launcher, virtual), display name, facts for rules
Zone        id, title, kind (Manual | Portal{dir} | Smart{query}),
            view (grid | list), sort, icon size, look, lock (geometry,
            members), rolled up, always visible, tab group
Placement   zone -> ordered item ids (+ manual positions); an item is in
            at most one manual zone; the rest are "loose"
Geometry    per zone and profile: display (stable id), anchor edge,
            offset and size in logical units relative to that anchor
Profile     key = set of displays (ids, sizes, scales) [+ workspace];
            value = geometry and which zones are shown
Rule        ordered; match (extension, MIME, name pattern, date, size,
            shortcut target, untouched for N days); action (assign to
            zone | move to folder); first match wins; optional catch-all
Snapshot    a named copy of zones, placements, geometry and rules
```

- A zone is a view. Assigning an item changes `Placement` only. The file
  system is touched by portals (real file operations) and by rules whose
  action is "move", and those go through the trash and an undo log.
- **Where it is kept.** Zone definitions, looks, rules and bindings: in
  `config.json` (declarative, hot-reloaded, written by the studio as every
  other section). Placements, geometry per profile, snapshots and the undo
  log: in `state/zones/`, versioned, written atomically with rolling
  backups through `model/src/files.rs`. Reason: placements change with
  every drag and can run to thousands of entries; `config.json` is
  rewritten whole and backed up on every save.
- "GUI edits without destroying comments": JSON has none. Unknown keys and
  order of keys are kept already; nothing more can be promised.

## The backend trait (milestone 1)

One trait in `crates/zones`, implemented by a fake in memory, by Windows
and by Linux:

- enumerate the desktop's items and a directory's; watch both (events, no
  polling)
- facts of an item (type, MIME, size, dates, shortcut target)
- icon and thumbnail at a size and scale (asked off the draw thread)
- open, "open with", the native context menu at a point
- file operations for portals (copy, move, rename, new folder, to trash),
  each returning what undoes it
- drag and drop: what a drop carries, and starting a drag of items
- the displays (stable id, rect, scale) and the room kept free
- the Explorer journal (Windows; a no-op elsewhere)

Surfaces and effects stay in shellkit, not in this trait.

## Windows backend

- Items through the shell namespace (`IShellFolder` of the desktop:
  merges the user's and the public folder, includes This PC and the
  Recycle Bin, right names and overlays).
- Icons and thumbnails: `IShellItemImageFactory` (already used).
- Context menu: `IContextMenu` with third-party extensions, shown over our
  surface. Launching: verbs through `ShellExecuteExW`.
- Drag and drop: OLE `IDropTarget` on the surface and `IDataObject` /
  `DoDragDrop` out of it (the `implement` feature of the `windows` crate
  is enabled already). New work in shellkit.
- Changes: `SHChangeNotifyRegister` plus `ReadDirectoryChangesW`.
- Look: `DWMWA_SYSTEMBACKDROP_TYPE` and corner preference on our own
  surfaces where Windows has them, else plain see-through.
- Privacy: Windows offers no reliable sign that the screen is shared.
  Proposed instead: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on
  the zone surfaces as an option (they are simply absent from captures),
  and masking by hotkey.

## Linux backend

- Desktop folder from `user-dirs.dirs`; inotify.
- By the specifications: Desktop Entry with the trust rules for launchers,
  Icon Theme (exists), shared MIME info, the thumbnail cache (read and
  fill), Trash. `.desktop` and icon themes are parsed by hand in this
  repository today; MIME, thumbnails and trash would be new.
- Drag and drop: a `wl_data_device` client in shellkit's layer half
  (`text/uri-list`). With X11 file managers it depends on the compositor's
  XDND bridge, whose X11-to-Wayland direction is to be checked.
- Context menu: ours (open, open with from the MIME associations, rename,
  trash, properties), since there is no shell to ask.
- Privacy: gevrek is the compositor and implements the screencast portal,
  so it knows when the screen is shared; masking can be automatic here.
- Tiling awareness: a zone region claims an exclusive zone, as the panel
  does. On Windows the WM keeps the room, as it does for the panel.

## Feature flag

No mechanism exists. Proposed: `desktop.zones.enabled`, false by default
until milestone 3, and the server's capability string `zones_v1`. No cargo
feature: the code is built and tested on every run of CI.

(As built: `desktop.zones.enabled` was false by default through
milestone 5 and is **true by default since 2026-11-20**. On Windows that
puts Explorer's desktop icons away while the shell runs, unless
`desktop.platform.windows.hide_desktop_icons` is false; a file that says
`"enabled": false` stays off.)

## Milestones

As in the brief, with what this repository adds:

1. **Core.** `crates/zones`: model, persistence, the trait, the fake,
   snapping, rules, profiles. Tests in the house form: tables, and seeded
   long runs for rules and for restoring a layout across changes of
   displays. Decision B's migration, if B1.
2. **Static zones.** Backends, drawing, launching, organizing by hand,
   move/resize/snap, context menu, drag and drop. Decision A's journal and
   its restore are built and tested **before** Explorer's view is first
   hidden. The publish RPCs of decision C.
3. **Roll-up, tabs, quick-hide, peek, lock, look.** The flag turns on.
4. **Portals, rules with dry run and undo, smart zones.**
5. **Profiles, snapshots, IPC/CLI, keyboard, privacy, accessibility,
   speed.**

"Usable on both platforms" has a limit worth stating: the Linux side can be
built and tested here in WSL and headless, but not seen; `udev.rs` has not
run on a physical machine. Each milestone's note will say what was seen and
what was only tested.

## Open questions

None that block milestone 1. To be tried on Windows 11 in milestone 2
before it is settled: how Explorer's icon view is hidden, and what "Show
desktop" (Win+D) does to bottom-level surfaces. To be checked on Linux:
whether a drag from an X11 program reaches a Wayland surface.
