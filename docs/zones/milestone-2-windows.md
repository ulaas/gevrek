# Zones: milestone 2, the Windows backend

Written 2026-11-19. The part of milestone 2 that is the Windows half of
`crates/system`: `gevrek_zones::backend::ZonesBackend` answered by the
shell of Windows.

**Nothing uses it yet, and nothing of it was seen in a zone.** The shell
does not make a `SystemBackend`; no zone shows an item. What exists is the
backend, tests that run against the real shell where that only reads and
against folders of their own where something is written, and an example
that prints what the backend finds on a desktop.

**Explorer's icons were never hidden, moved or changed**, not by a test,
not by the example, not by hand. The code that would do it exists, was
tested against an Explorer in memory, and is waiting for somebody to run
it on a desktop that is theirs (the checklist at the end).

## What exists

`crates/system/src/zones/windows/`:

| Module | What it does | Asked of Windows through |
|---|---|---|
| `mod` | `SystemBackend`: the trait, and what the shell uses beside it | |
| `shell` | What is at a place and the facts of each item | `IShellFolder::EnumObjects`, `IShellItem`, `IShellItem2`, `AssocQueryStringW`, `AssocGetPerceivedType` |
| `facts` | The part of that which is arithmetic: kind, type, times, which things lie on the desktop | |
| `icons` | Icons and small pictures by handle, made off the drawing thread | `IShellItemImageFactory` over `crate::icons::Cache` |
| `launch` | Opening, "open with", the shell's own menu | `ShellExecuteExW`, `IContextMenu`/`2`/`3`, `TrackPopupMenuEx` |
| `fileops` | Copy, move, rename, new folder, the Recycle Bin and back | `IFileOperation`, `IFileOperationProgressSink` |
| `watch` | Being told of changes: folders, the desktop, the displays | `notify` (`ReadDirectoryChangesW`), `SHChangeNotifyRegister`, `WM_DISPLAYCHANGE` |
| `seen` | What is known at a watched place; each change told to the core once | |
| `displays` | The displays by the window manager's ids, in logical units | `EnumDisplayMonitors`, `crate::display_ids` |
| `dnd` | What a drag carries | the shell's `IDataObject` (`BHID_DataObject`) |
| `explorer` | Explorer's own icons: the journal, hiding, putting back | `IShellWindows`, `IShellBrowser`, `IFolderView2` |

Beside it: `crates/system/src/display_ids.rs` (below, "The displays"),
two additions to `crates/system/src/icons.rs` (`Cache::answer`,
`Cache::forget`, and `shell::picture_as` for small pictures), and the
example `crates/system/examples/zones_desktop.rs`.

## How each thing the trait asks is answered

- **`list`.** The shell's folder is enumerated (`SHGetDesktopFolder` for
  the desktop, the folder's own `IShellFolder` for a directory); hidden
  files are there when the person has Explorer show them. An item's id is
  its parsing name (`SIGDN_DESKTOPABSOLUTEPARSING`): the path for a file,
  `::{20D04FE0-...}` for This PC. The name is the one the shell shows.
  Of the desktop only what lies on it as an icon is listed. The shell's
  namespace begins at the desktop, so its folder has much more directly
  under it (Documents, Downloads, Network, Control Panel, Gallery, Home:
  the roots of Explorer's tree). A file or folder counts when it is in one
  of the desktop's two folders; a thing that is no file counts when the
  person's "Desktop icon settings" show it
  (`...\Explorer\HideDesktopIcons\NewStartPanel`, the person's value
  before the machine's; with no value only the Recycle Bin is there). On
  this machine that gives exactly the 25 icons Explorer shows; the
  example prints the difference, which is empty.
- **`facts`.** The ending from the file's real name; the type from the
  registry's content type, else made of Windows' "perceived type" and
  the ending (`image/x-xcf`), `inode/directory` for a folder so that one
  rule fits both systems; size and times from the file system; where a
  shortcut leads from the shell's property `System.Link.TargetParsingPath`
  (`.lnk` and `.url` alike).
- **`watch`, `unwatch`, `drain_events`.** A directory is watched by the
  file system (`notify`). The desktop is watched twice: its two folders
  by the file system, and the shell's desktop by the shell, for what is
  no file. Both tell into one record of what is known there (`seen`), so
  a change told by both is told to the core once, a save that deletes and
  writes anew is a change and not a going and coming, and "look again"
  becomes exactly the differences. The host is woken by the function
  given to `SystemBackend::new`. Nothing looks by the clock: every thread
  sleeps in the system until it is told.
- **`request_icon`, `request_thumbnail`.** Asked of the shell on the icon
  cache's thread, at the display's pixels (size times scale), answered as
  `IconReady` with a handle; `SystemBackend::picture(handle)` gives the
  pixels. A small picture is asked for with `SIIGBF_THUMBNAILONLY`, so
  the answer is none where a file holds no picture. An item that changed,
  went or was renamed loses its pictures; asked for again they are made
  again and have other handles.
- **`open`, `open_with`.** `ShellExecuteExW` by the item's id list, so
  that what is no file opens too; "open with" without a program is the
  shell's verb `openas`. Each on a short-lived thread: the drawing thread
  never waits for a program that answers slowly. The shell shows what
  goes wrong, as Explorer does.
- **`context_menu`.** The shell's menu with everything other programs
  added, on a thread of its own with a window of its own that passes the
  menu's messages on to `IContextMenu2`/`3` (so "Send to" and "Open with"
  fill in). The window belongs to the one given with
  `SystemBackend::set_owner`. `launch::show_menu(owner, items, (x, y))`
  is the same at an exact point of the screen.
- **`perform`.** One `IFileOperation` per call, silent except for a long
  operation's progress and for the one question that must stay (below);
  what the shell made is heard through its progress sink, so the way back
  names the real file. A file is never written over: where one of its
  name is, the shell gives another name and the way back knows it.
  `Trash` is `FOFX_RECYCLEONDELETE`; `Restore` finds the file in the
  Recycle Bin (by where it was deleted from; the newest of several) and
  moves it back. No crate for the trash was needed on Windows.
- **`drop_payload`, `start_drag`.** See "Drag and drop".
- **`displays`, `reserved_areas`.** See "The displays".
- **`journal_system_icons`, `system_icons_to_restore`,
  `restore_system_icons`.** See "Explorer's icons".

## Pixels for the one who draws

`SystemBackend::picture(IconHandle) -> Option<&icons::Picture>`: width,
height, and four bytes a pixel (red, green, blue, solidness, not
multiplied). It is the same `Picture` the panel and the launcher already
turn into textures. `pixels(IconSize)` says the side a request comes to.

The cache keeps every picture it made until the item changes. A desktop
of a few hundred icons at 72 pixels is some megabytes; nothing is thrown
out for age yet.

## Drag and drop: the data half

`gevrek_shellkit` has the window that takes drops and the loop that runs
while something is dragged. This half says what is carried:

| Function | For |
|---|---|
| `dnd::data_object(&[ItemId]) -> Result<IDataObject, BackendError>` | The shell's own object for the items: paths (`CF_HDROP`), the shell's names ("Shell IDList Array", also for what has no path), everything Explorer needs, and gevrek's own kind beside them |
| `SystemBackend::take_drag() -> Option<DragOut>` | After the core's `start_drag`: the object, the items, what is allowed; handed to `DoDragDrop` |
| `dnd::payload_of(&IDataObject, DragActions) -> DropPayload` | What a drag over the surface carries (`DragEnter` has the object): the items when the drag began in a zone of this program, else paths, else text |
| `SystemBackend::set_drop(Option<DropPayload>)` | Keeps it for the core's `drop_payload`; `None` when the drag leaves or ends |
| `dnd::effects_of`, `dnd::actions_of` | Between the core's three actions and Windows' `DROPEFFECT` |
| `dnd::items_of_id_list(&[u8]) -> Vec<ItemId>` | The items named by the bytes of the shell's kind, where only those are at hand |

gevrek's own kind (`gevrek.zones.items`) is the ids and the process that
began the drag, so a zone knows a drag out of another zone as items and
moves them without touching a file.

## The displays

The zones know a display by the id the window manager gives it
(`MonitorInfo::stable_id`, made of the EDID). The shell is told the
displays by the window manager; `displays_of_state` reads them out of its
state, `SystemBackend::set_displays` takes them, and from then on they
are what counts.

Until then (and in the example) the backend reads the displays from
Windows itself. So that both give a display the same id, three things
were moved to where both can reach them:

- The parser of the EDID and the making of the id went from
  `crates/engine/src/outputs_edid.rs` to `crates/model/src/edid.rs` (std
  only). `gevrek_engine::outputs::edid` still names it.
- The rule that tells twins apart (`#1`, `#2`) is written once, there
  (`edid::number_same_ids`); the engine's `number_same_stable_ids` calls
  it.
- The reading of the EDID out of Windows went from
  `crates/backend-win32/src/dwm/monitor.rs` to
  `crates/system/src/display_ids.rs`, with the one function that decides
  an id (`stable_ids`: the EDID, else maker and model, else the device
  path, then the twins, over all displays at once). The window manager
  and the zones both call it. `gevrek-win32` depends on `gevrek-system`
  for that; `gevrek-system` needs only the model.

The window manager's example (`cargo run -p gevrek-win32 --example
displays`) still prints `GSM-5B7F-00034872-003NTYT6B154` and
`GSM-5C4D-00059AA9-307NTWGAT273` on this machine, and this backend
prints the same two.

The core counts in logical units, Windows in pixels with a scale for
each display. A display's rectangle in logical units is its rectangle in
pixels divided by its scale, corner and size alike. Two displays of
different scales can then cover the same units; a point is taken to be on
the display the pointer is on (`displays::to_pixels`).
`SystemBackend::pixels_of(display)` gives the exact rectangle for the
surface.

## Explorer's icons

The rule is: never lose a person's layout.

- `journal_system_icons` reads how Explorer shows the desktop (icons
  shown, auto-arrange, snap to grid, view and icon size, and where each
  icon is) and writes it to `state/zones/explorer.json`, atomically,
  marked *open*. Nothing of Explorer's is changed. While a journal is
  open it is never written over. The one from before is kept as
  `explorer.previous.json`.
- `SystemBackend::hide_system_icons` is the one call that changes
  something of Explorer's, and nothing calls it. It is refused without an
  open journal, and it writes `hidden: true` into the journal before it
  asks Explorer.
- `restore_system_icons` compares the journal with how the desktop is and
  does only what differs: size, arrangement, each icon to its place, and
  the icons shown last. Then the journal is closed and stays as a record.
  If a step fails it stays open and is tried again.
- `system_icons_to_restore` is true while a journal is open, which is
  what a start after a crash finds.
- A journal that cannot be read counts as open and as "the icons may be
  away": putting back then only shows them.
- Icons the person had switched off themselves are left off, and icons
  they switched on while gevrek ran are left on.

The file:

```json
{
  "version": 1,
  "written_at": 1795100000,
  "open": true,
  "hidden": true,
  "desktop": {
    "view": { "icons_shown": true, "auto_arrange": false, "snap_to_grid": false, "view_mode": 1, "icon_size": 32 },
    "icons": [ { "id": "C:\\Users\\ada\\Desktop\\a.txt", "name": "a.txt", "x": 25, "y": 90 } ]
  }
}
```

### How the icons are put away: recommended, and not tried

Two ways were weighed.

| | The view's own flag | Hiding the list's window |
|---|---|---|
| How | `IFolderView2::SetCurrentFolderFlags(FWF_NOICONS, FWF_NOICONS)` on the desktop's view, reached through `IShellWindows::FindWindowSW(SWC_DESKTOP)` | `ShowWindow(SW_HIDE)` on the `SysListView32` under `SHELLDLL_DefView` |
| Documented | yes: `FOLDERFLAGS`, `IFolderView2`, `IShellWindows` | no: the windows' classes and their order are Explorer's own, and changed in Windows 11 |
| What it is to Explorer | its own "Show desktop icons", as the desktop's menu switches it | a window that is unexpectedly not shown |
| The exact reverse | the same call with the flag cleared; or the person's own click on "Show desktop icons" | `ShowWindow(SW_SHOW)`, if the window is still the same one |
| If Explorer restarts | to be seen (checklist); the menu's switch survives it | the icons are back, drawn over by the zones |

**Recommended: the view's flag.** It is the documented way, it is
Explorer's own switch, and the person can always undo it without gevrek.
It is what `explorer::ShellDesktop::show_icons` does. References:
[FOLDERFLAGS](https://learn.microsoft.com/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags),
[IFolderView2::SetCurrentFolderFlags](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifolderview2-setcurrentfolderflags),
[IShellWindows::FindWindowSW](https://learn.microsoft.com/windows/win32/api/exdisp/nf-exdisp-ishellwindows-findwindowsw),
and Raymond Chen, "Manipulating the positions of desktop icons"
(The Old New Thing, 2013-03-18), which is this way to the view.

Whether the flag does on Windows 11 what the documentation says was
**not tried**: that would have hidden the icons of the desktop this was
written on.

## Dependencies

- `notify` 8 (CC0-1.0), without its default features, for Windows as for
  Linux. `serde` for the journal.
- `trash` was not added: the shell's own file operation recycles and
  says where the file lies afterwards.
- Features of the `windows` crate added to `gevrek-system`:
  `Win32_System_Ole`, `Win32_System_SystemServices`,
  `Win32_Devices_Display`, `Win32_Devices_DeviceAndDriverInstallation`,
  `Win32_Devices_Properties`.
- `cargo deny check licenses advisories bans sources`: all four ok.

## Running it

```
cargo test -p gevrek-system
cargo clippy -p gevrek-system --all-targets
cargo run -p gevrek-system --example zones_desktop
```

The example only reads, unless it is given `--really-hide`.

## What was verified on this machine

Windows 11 Pro 26300, two displays at 100 %, Explorer running.

By tests (45 for the zones, of 123 in the crate):

- Pure: kinds, types, paths, which things lie on the desktop, pixels for
  a size, the picture cache by handle, the record of what is known
  against every kind of change, the three actions and effects, gevrek's
  kind there and back, the list of dropped files wide and narrow, the
  places in the shell's kind and garbage, logical units and pixels, the
  window manager's state, the journal's text, the plan for putting back,
  the whole order of journal, hide and restore against the Explorer in
  memory (also after a "crash", with a failing step, with an unreadable
  file, with icons the person had off).
- Against the real shell, reading only: the desktop listed with ids the
  shell takes back; the desktop watched and let go of again; the facts
  of files in a folder of the test's; an icon drawn by the shell at 48
  pixels for 32 at 150 %, none for a file that is not there, no small
  picture for a text file; the shell's data object for files in two
  folders carrying paths, names and items; Explorer's desktop read from
  outside twice with the same result, through an object that refuses
  every change.
- Against folders of the tests' own under `%TEMP%`: the file system's
  telling (come, renamed, changed, a folder, gone; nothing from below;
  nothing after the end); the shell's telling, said to the shell as
  programs say it (create, rename, new folder, update, "look again",
  delete); copy, move, rename, new folder and recycle through
  `IFileOperation`, each taken back by what it answered; a name that is
  taken; a file that is never written over; a file put back from the
  Recycle Bin by a run that remembers nothing, the newest of two first.

By the example: 25 items with their facts, the same 25 Explorer shows
with their positions; six icons fetched at 72 pixels; the shell's menu
for a file read without showing it (30 entries, other programs' among
them); both displays with the ids the window manager prints.

## What was not verified

- **Anything that changes Explorer.** `show_icons`, `arrange`,
  `size_icons`, `place` of the real `ShellDesktop` were never called.
  Unknown with them: whether `FWF_NOICONS` hides the icons from outside
  on this Windows, whether the view still lists its items while they are
  hidden, what a restart of Explorer does to it, whether
  `SelectAndPositionItems` puts an icon back to the pixel with snap to
  grid on.
- **Nothing that shows something on the screen was run:** the menu
  (`TrackPopupMenuEx`, the entries' own drawing, what an entry opens),
  `open`, `open_with` and the "Open with" dialog. Their refusals (what is
  not there, what is no file) are tested; that a menu's entries can be
  read is; that they can be picked is not.
- **The desktop's own changes.** Nothing was put on the desktop, so the
  telling was tried on other folders only. Not seen: This PC switched on
  in "Desktop icon settings" appearing, the Recycle Bin's picture
  changing when it fills.
- **The shell's telling of files.** Asked to tell of a folder's files by
  itself, the shell told nothing here within twenty seconds (it did tell
  what programs said to it). That is why the file system is listened to
  for files; whether the shell tells of the desktop's files, which
  Explorer watches too, was not seen.
- **Displays:** a display plugged in or out, a change of scale, two
  displays of different scales, a display without an EDID.
  `displays_of_state` was tested against a state written by hand, not one
  the window manager sent.
- **Drags:** a real drag in either direction. The object and its reading
  are tested; `DoDragDrop` and a drop target are shellkit's.
- **Large and slow things:** a copy that takes long (the shell's own
  progress window is left on for that, and `perform` returns only when
  it is done), a folder on a network, a drive without a Recycle Bin.
- **A third program's thing on the desktop** that is no file and has no
  entry in "Desktop icon settings" is not listed, where Explorer may show
  it.

## What the trait could use

- `perform` returns when the operation is done. For a long copy that
  holds its caller; the events already have `OperationDone`, so a backend
  could answer later if `Host` did not expect the answer at once.
- `context_menu` takes a point in logical units, which is ambiguous
  between displays of different scales. The backend decides by the
  pointer; a renderer that knows the pixels should call
  `launch::show_menu`.

## Left in the Recycle Bin

The first run of the file tests failed before it put back what it had
recycled. Three files of a few bytes from that run are in the Recycle
Bin of this machine: `a.txt` and two `kept for later.txt`, deleted from
`%TEMP%\gevrek-zones-fileops-26928\archive` and
`%TEMP%\gevrek-zones-recycle-26928`. The tests now put back everything
they recycle, also when they fail, and leave nothing there.

## Checklist: to be tried by hand on Windows

Nothing of this was done. Each line is one thing to look at.

Explorer's icons (on a desktop that is yours; `cargo run -p gevrek-system
--example zones_desktop -- --really-hide`, or `cargo test -p gevrek-system
-- --ignored really_hides`):

- [ ] The icons go when it says "put away", and nothing else of the
      desktop changes (the wallpaper, the desktop's own menu).
- [ ] `state` of the trial holds `zones/explorer.json` with `open` and
      `hidden` true and every icon's place, before the icons went.
- [ ] Enter brings them back, each where it was. With snap to grid on and
      off; with auto-arrange on (then no places are given).
- [ ] While they are away, the desktop's menu shows "Show desktop icons"
      unticked, and ticking it there brings them back.
- [ ] Crash recovery: end the example from the task manager while the
      icons are away; start it again with `--really-hide`; it says a
      journal is open, and after Enter the icons are back.
- [ ] Restart Explorer while the icons are away (task manager, "Restart").
      Are they still away? Does putting back still work?
- [ ] Change the icons' size (Ctrl and the wheel) before putting back:
      size and places come back as written down.
- [ ] Sign out and in while the icons are away; put back afterwards.

The menu (needs the renderer, or a few lines that call
`launch::show_menu` with a window's handle):

- [ ] Right click on a file, a folder, a shortcut, This PC, the Recycle
      Bin: the shell's own menu, with other programs' entries.
- [ ] "Send to", "Open with", "New": the lists inside them fill in and
      their icons are drawn.
- [ ] Shift held: the longer menu ("Copy as path").
- [ ] A press elsewhere closes the menu; Escape closes it.
- [ ] Properties opens and stays open; the panel goes on drawing while
      the menu and the properties are open.
- [ ] Delete from the menu: the file goes to the Recycle Bin and the zone
      is told it went.
- [ ] Several items picked, in both of the desktop's folders: one menu
      for all of them.
- [ ] The menu on a display at another scale opens at the pointer.

Opening:

- [ ] Double click on each kind of item; a shortcut whose target is gone
      (the shell says so).
- [ ] "Open with" without a program shows Windows' own chooser.

Drag and drop (needs shellkit's half):

- [ ] From a zone to an Explorer window: move within a disk, copy to
      another; Ctrl copies, Shift moves, Alt makes a shortcut.
- [ ] From a zone to the Recycle Bin, to a program's window (a text
      editor takes the file), to a browser's upload field.
- [ ] From Explorer to a zone: files arrive as paths; from one zone to
      another: as items, and no file is touched.
- [ ] This PC dragged out of a zone: Explorer makes a shortcut of it.
- [ ] Text dragged from a browser over a zone: seen as text.
- [ ] The picture under the pointer while something is dragged.

Changes:

- [ ] A file saved to the desktop appears; renamed in Explorer it keeps
      its place in its zone; deleted it goes.
- [ ] A program installed "for all users" puts its shortcut on the
      desktop: it appears.
- [ ] This PC switched on and off in "Desktop icon settings".
- [ ] The Recycle Bin's picture when it fills and is emptied.
- [ ] Hidden files switched on and off in Explorer.
- [ ] No use of the processor while nothing happens (task manager, the
      shell's row, for a minute).

Hot-plug:

- [ ] A display unplugged and plugged in again: the zones are told
      once each time, and the display has the id it had.
- [ ] The same display on another port: the same id.
- [ ] The scale changed in Windows' settings; the resolution changed.
- [ ] The panel moved to another side: the room kept follows.
- [ ] Two displays of one model without serial numbers: two ids
      (`...#1`, `...#2`).

Files:

- [ ] A copy of several gigabytes into a portal: the shell's progress
      shows. Does the panel go on drawing meanwhile? (`perform` returns
      only when the copy is done; see "What the trait could use".)
- [ ] Recycle on a drive that has no Recycle Bin (a network share): the
      shell asks before it deletes; "No" leaves the file.
- [ ] Undo of a recycle after gevrek was restarted: the file comes back.
