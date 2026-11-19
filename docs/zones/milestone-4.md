# Zones: milestone 4, the shell's part

Written 2026-11-19. Zones that show a folder and walk into it, the
rules looked over before they are carried out, and zones that fill
themselves.

What decides is the core (`core-milestones-3-5.md`), what the systems
supply is in the backends (`milestone-4-backends.md`); this page is what
the shell draws and hears: `crates/shell/src/zones/`.

**Nobody has seen it run.** It was built against the backend in memory
(folders in a tree, a folder that is slow, one that is away, made-up
lists for the files used last and the programs that run), tried in
tests, and drawn into pictures that were looked at.

## What exists

| File | What changed |
|---|---|
| `zones/view.rs` | A portal's way as boxes in its title (`NavBox`: back, up, each folder from its own to where it stands; the first ones give way when they do not fit); `Hit::Nav`; a drop knows the folder among a portal's items it is on (`Target::Zone::into`); `ZoneBox::outside` (what it shows is not of the desktop); `ZoneBox::busy` and `Layout::under_way` (what takes long). |
| `zones/gesture.rs` | A click on a part of the way goes there; a drag by it moves the zone. Items onto a folder a portal shows (`Act::IntoFolder`); out of a portal or a zone that found them elsewhere (`Act::BringOut`); a click in a zone whose folder is away asks again. Ctrl+Z. |
| `zones/draw.rs` | The way in the title; the mark that says what fills a zone; "Looking…", "Not there now", "Nothing here"; a thin line under the title and a chip at the desktop's foot for what takes long; a tile with a letter for a program. |
| `zones/mod.rs` | `make_zone`, `rules_preview`, `new_folder_in`; what is dropped on a portal goes where it stands (`BringIn`, so it can be taken back); `Desk::pinned`/`pin`/`programs`/`properties`; refusals from anywhere shake their zone (`RefusedAbout`). |
| `zones/surfaces.rs` | The sheet of what the rules would do (`rules`, `rules_sheet`); the properties' sheet; the menu of zones that fill themselves; boxes for a folder's path and a new folder's name; the zone's menu split in three (itself, "Shown and ordered…", "Look…"). |
| `zones/backend.rs` | Everything the core asks is passed on (`begin_listing`, `list_source`, `watch_source`, `begin_operation`, `cancel_operation`, `screen_shared`): a test holds that no method of the trait is left to its default. The window manager's state goes to the backend for the running programs (`set_wm_state`); the backend's commands go to the window manager (`set_commander`). On Linux the item's menu has "New folder…", "Properties" and "Open with another program…" (every candidate). |

**Edits outside `crates/shell/src/zones/`** (one, as small as it could
be made): `crates/shell/src/windows.rs`, `zones_configured`: the
`Starter` handed to the zones is no longer `Starter::none` but one whose
`dispatch` sends `Request::Dispatch(command)` over the shell's line (one
`let tell = shared.link.teller();` and the closure). Without it a
running program shown in a zone could not be brought forward on Windows.
`main.rs` already passed such a starter.

## How each thing is done

| What | By the pointer | The command |
|---|---|---|
| Make a zone of a folder | The desktop's menu, "New zone of a folder…": the folder chooser of the design system (`gevrek_ui::browser`, the one the settings use), in which a folder is gone to by pressing or by typing its path | `create_zone` with `kind: portal` |
| Go into a folder | Double click it | `open` (the core goes into one folder of a portal in place) |
| Back, up, to a folder of the way | Press the angle, the arrow, a folder's name in the title | `navigate_back`, `navigate_up`, `navigate_to` |
| Show only what fits | Type, with the keyboard in the zone; Backspace; Escape empties | `filter` |
| Put a file into the folder | Drop it on the portal (into where it stands), or on a folder it shows (into that) | `add_item`; `move_items_to_folder`; from another program `bring_in` |
| Take a file out to the desktop | Drag it out, onto the bare desktop or a zone filled by hand | `bring_in` with the desktop's folder (moved; from a zone that only found it, copied) |
| Rename, bin | F2, Delete, the item's menu | `rename_item`, `trash_items` |
| New folder | The zone's menu, "New folder in it…"; on the desktop itself the desktop's menu, "New folder on the desktop…" | `new_folder`; `new_folder_in` with the desktop's folder (both taken back by Ctrl+Z) |
| Take the last file change back | Ctrl+Z; the desktop's menu | `undo_file_operation` |
| Ask again what is there | Press in a zone that says "Not there now"; the zone's menu, "Look again what is there". Unasked: a minute after it was seen to be away and every minute while it is, and at once when its zone comes back into view (shown again, its workspace shown) | `refresh_zone` |
| Call off a long copy | The desktop's menu, while one is under way | `cancel_operation` |
| See what the rules would do | The desktop's menu, "What the rules would do…" | `preview_rules` (the sheet asks the core directly) |
| Carry the rules out, take it back | The sheet's "Sort now", "Take the last sorting back"; the desktop's menu | `apply_rules`, `undo_rules` |
| Make a zone that fills itself | The desktop's menu, "New zone that fills itself…": the files used last, what was downloaded today, the programs that run, the pinned programs, the pictures on the desktop | `create_zone` with `kind: smart`, then `modify_zone` for `source`, order and view |
| Bring a running program forward | Double click it in its zone | `open`; the backend asks the window manager (`focus_window`) |
| Pin a program, let it go | Its menu (with Shift on Windows) | the backend's `pin_launcher` (the pins are its file) |

## What is drawn

- **The mark at the start of a title says what fills the zone**: a bar
  for one filled by hand, a small folder for one that shows a folder, a
  diamond for one that fills itself.
- **A portal that has gone somewhere shows its way in place of its
  name**: an angle (back), an arrow (up), then the folders from its own
  to where it stands, the last in the title's ink (it is where it
  stands, and no way to go), the others quieter and lit under the
  pointer. In its own folder with no way back it is its name, as any
  zone.
- **The filter** is the chip after the name, with `3 of 5` at the end.
- **A file a cloud keeps elsewhere** (Windows: online-only, by the
  backend's `cloud_state`) has a small cloud at the corner of its icon.
  It is asked once for each file a portal shows, and looking fetches
  nothing.
- **A zone that has nothing to show says why**: "Looking…" while its
  folder is being read (no spinner: nothing moves but roll, fade and
  peek), "Not there now · press to look again" with the system's reason
  under it, "Nothing here", or "Nothing here is called so" under a
  filter. The zone stays where it is in every case.
- **What takes long** (a copy the backend carries out by itself): a thin
  line under the title of the zone it goes into, filled as far as it has
  come (all of it, quietly, where the backend cannot say); for what
  belongs to no zone a chip at the foot of the primary display with the
  words ("Copying film.mkv · 29 %").
- **The sheet of the rules**: every item with where it would go and by
  which rule; a file that would be moved is in the warning colour, since
  it is the one thing here that touches the file system; what is held
  back is quieter and says why (its zone lets nothing in, its rule names
  no zone, the zone fills itself, it is no file). A line in a few words
  before the list, the two scopes, and under the list how often the
  rules that ask a file's age are tried and whether they may move files.

## Pictures

    cargo test -p gevrek-shell milestone_four_is_drawn -- --ignored

writes into `target/zones-pictures/`:

| File | What |
|---|---|
| `m4-portals-and-smart-zones.png` | A portal two folders in with its way, one whose folder is away, one still being read; the files used last as a list, the programs that run, the pinned ones |
| `m4-portal-filter.png` | The same with "inv" typed into the portal |
| `m4-portals-and-smart-zones-1.5x.png` (1920 by 1200), `m4-portals-and-smart-zones-high-contrast.png` | At one and a half; with high contrast and the filter |
| `m4-rules-preview.png`, `m4-rules-preview-high-contrast.png` (600 by 460) | The sheet: four into zones, a file moved, two held back, a timed rule |

All were looked at. Changed after looking: programs were drawn as
papers with an arrow and are tiles with a letter now; the sheet's lines
were centred, twice as high, and had an arrow the font does not have.

## What was verified

- `cargo test -p gevrek-shell` on Windows and in WSL, `cargo clippy -p
  gevrek-shell --all-targets` on both with the two old warnings only,
  `cargo build`. Numbers in the report of the day.
- Tests beside the code: the way of a portal laid out (`view`); a portal
  walked by double click, by the way and back, dragged by its way; files
  into a folder it shows, back with Ctrl+Z, out to the desktop, a file
  of another program into where it stands, a folder made; the rules'
  preview against a desk with rules of every sort, applied and taken
  back; every zone that fills itself made with what it looks through;
  a long copy shown on its zone and at the desktop's foot, and called
  off; a refusal from outside shown on its zone; the menus; that the
  adapter passes every question of the core on.

## What was not verified, and what is not done

- **Nothing on a screen, and nothing against a real folder from the
  shell.** The backends' own tests use real folders; the shell's tests
  use the backend in memory. A slow share, a cloud folder and the
  programs that run were never shown in a zone.
- The sheets (rules, properties) and the boxes are surfaces that were
  never opened by a host; the sheet's drawing was pictured, its buttons
  were not pressed (what they do is three commands and a new preview).
- Away folders are asked again by a wake-up the zones ask for a minute
  ahead, only while a zone that is shown waits for one (nothing looks by
  the clock while nothing is away). Tried in a test with the wait cut
  short; never against a share that comes back.
- The folder chooser is the design system's own widget on a surface of
  its own; never opened by a host from here. It reads the folders it
  shows on the thread that draws, as it does in the settings.
- The cloud mark was never seen: the tests' backend has no cloud.
- Programs cannot be put in order by dragging within "Pinned" (the
  backend has `set_pinned_launchers`; no gesture gives it).
- On Windows the properties, "new folder" and "open with" are the
  system's own menu's; the sheets above are for Linux.
- The line of progress is not animated where the backend cannot say how
  far it is.

## Manual test checklist

**Portals (both systems)**
- [ ] Desktop menu, "New zone of a folder…", type the path of Downloads:
      a zone with a small folder as its mark shows what is in it.
- [ ] Double click a folder in it: its items are replaced in place, the
      title becomes the way (angle, arrow, names). Press a name, the
      arrow, the angle: each goes where it says. Drag the title by a
      name: the zone moves and stays in its folder.
- [ ] Type a few letters with the keyboard in the zone: only what fits
      stays, the chip shows them, the count says `n of m`; Backspace;
      Escape shows all again. Go to another folder: the filter is empty.
- [ ] Drop a desktop file on the portal, on a folder inside it; drag a
      file out to the desktop and into a zone filled by hand; each time
      look in a file manager: the file really moved. Ctrl+Z after each:
      it is back.
- [ ] From Explorer / a file manager, drop a file on the portal while it
      stands in a subfolder: it lands there. With Ctrl: copied.
- [ ] F2 and Delete on a file in a portal; "New folder in it…".
- [ ] Change the folder from another program (add, rename, delete a
      file): the zone follows without being asked.
- [ ] A portal on a share that is switched off: "Not there now" with the
      reason, at once, and the shell stays quick; switch the share on and
      press in the zone: its files.
- [ ] A portal on a folder of thousands of files: "Looking…" and then
      the files; nothing else stalls meanwhile.
- [ ] Windows, a OneDrive folder with online-only files: icons show,
      nothing is downloaded by looking (watch the OneDrive icon).
- [ ] Copy a file of a gigabyte into a portal: the line under the title
      fills; "Call off…" in the desktop's menu stops it.

**Rules**
- [ ] With a few rules configured (one that moves files): "What the
      rules would do…" lists every loose item with its rule; "Every item
      on the desktop" lists more. Nothing has moved yet.
- [ ] "Sort now": the desktop is as the list said; "Take the last
      sorting back": as before, moved files back in their folder.
- [ ] A rule with `untouched: 30`: the sheet says how often such rules
      are tried; set `timed_rules.every_minutes: 1` and wait.
- [ ] Drop a new file on the desktop that a rule fits: it appears in its
      zone at once.

**Zones that fill themselves**
- [ ] "The files used last": open a document, it comes to the top.
- [ ] "What was downloaded today": download something.
- [ ] "The programs that run": start and close programs, the zone
      follows; double click one: its window comes forward (Windows too).
- [ ] "The pinned programs": Shift+other button on one (Windows) or its
      menu (Linux), "No longer pinned"; double click starts it.
- [ ] Drag a file out of "Recent" to the desktop: a copy comes; the
      original stays where it was.

**Linux only**
- [ ] The other button on a file: "Open with another program…" lists
      every program for its type; "Properties" shows name, place, type,
      size, dates, rights; "New folder…" makes one beside it.
