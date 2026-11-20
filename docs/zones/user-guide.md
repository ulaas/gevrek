# Zones

Zones are boxes on the desktop that hold its icons: one for the papers
of a project, one that shows the Downloads folder, one that fills itself
with whatever was changed today. They are drawn by the shell
(`gevrek-shell`), below the windows, on every display. Everything that
can be done to them with the pointer can also be done with a key, in the
configuration, and from the command line.

Zones are not **snap zones**. Snap zones are the places on a display
that a *window* snaps into (`desktop.snap_zones`, `gevrek snap-zone 2`;
see [GUIDE.md](../GUIDE.md)). Zones hold *icons*
(`desktop.zones`, `gevrek zones ...`). A configuration written before
schema version 26 had the snap zones under `desktop.zones`; they are
moved when it is read.

Read [Before you rely on it](#before-you-rely-on-it) first: as of
2026-11-20 most of what is written here was checked by tests and
pictures, and not yet on a real desktop. What is not done is under
[What is not there yet](#what-is-not-there-yet).

The menus are named as the shell names them. "The other button" is the
one that asks for a menu (the right one, unless the buttons are
swapped).

## They are on; turning them off

Zones are **on unless the configuration says otherwise** (since
2026-11-20; before that they were off until switched on). A
configuration that says nothing of them has them, and so does one from
before there were zones. A file that says `"enabled": false` keeps them
off: nothing switches them on behind your back.

**On Windows this means that Explorer's own desktop icons are put away
while gevrek's shell runs**, because the zones draw every item of the
desktop instead (see [Windows: Explorer's icons](#windows-explorers-icons)
for how they are written down first and every way back). To have zones
and leave Explorer's icons alone, set
`desktop.platform.windows.hide_desktop_icons` to `false` (the icons then
lie under the zones: for trying, not for living with).

To switch zones off:

```json
{ "desktop": { "zones": { "enabled": false } } }
```

or in the studio, on the page **Zones**, the switch "Show zones on the
desktop". Explorer's icons are put back as that takes hold, and
`gevrek zones restore-explorer` puts them back at any time.

The shell must be running (`gevrek start all`, or `gevrek start
shell`): the zones live in it. With them off nothing is drawn, nothing
of the system is touched, and every command is refused with
`zones are switched off (desktop.zones.enabled)`.

```
$ gevrek zones list
Zones: 2 · profile: DEL-A1B2-77 1920x1080@1.00 · snapshot 14
  ID    TITLE      KIND    DISPLAY      RECT           ITEMS  FLAGS
  work  Work       manual  DEL-A1B2-77  40,40 320x28       2  rolled-up locked(geometry)
  dl    Downloads  portal  DEL-A1B2-77  72,72 320x240     14
Loose items: 9
```

"Loose" items are the ones in no zone. They are drawn on the bare
desktop where Explorer (or the file manager) would have them.

## What a zone is

| Kind | What it shows | What putting something in does |
|---|---|---|
| `manual` | items of the desktop that you, or a rule, put there | the item is *shown* in the zone; the file stays where it is |
| `portal` | what is in a folder, any folder | the file is *moved* into that folder |
| `smart` | whatever matches a query, by itself | nothing can be put in: it fills itself |

A zone is a view. Putting an icon into a manual zone, taking it out, or
deleting the zone never moves or deletes the file. Only two things touch
files: a portal (it is a folder) and a rule whose action is "move to
folder". Both go through the system's bin and can be taken back.

Every zone has an **id** (`work`, or `zone-3` for one made by hand) and a
**title**. On the command line a zone is named by its id, by its title,
or by the beginning of either, as long as only one zone fits; an item is
named by its id (its path), its name, or the beginning of its name:

```
$ gevrek zones add work "Budget 2026.xlsx" notes
2 items put into zone 'work'
$ gevrek zones add work report
Error: 'report' is the name of 2 items. Say which by its id:
  /home/ada/Desktop/report.pdf  (in papers)
  /home/ada/Desktop/report.txt  (loose)
```

`@focused` stands for the zone that has the keyboard, wherever a zone is
named.

## Every feature, four ways

### The menus, and the buttons of a title

Three menus: the **desktop's** (the other button on the bare desktop),
a **zone's** (the other button anywhere on the zone, or the last button
of its title) and an **item's** (the other button on an icon).

A line with an arrow at its end leads to **a menu of its own, which
opens beside the line**: when the pointer rests on the line for a
moment, at once when the line is pressed, and with the right arrow or
Enter. The menu it opened from stays. Only one way through the menus is
open at a time: resting on another line closes what had opened from
that menu. A line that does something closes them all; so does a press
anywhere else.

| The desktop's menu | |
|---|---|
| New zone ▸ | Empty, here · Of a folder… · That fills itself ▸ (the files used last, what was downloaded today, the programs that run, the pinned programs, the pictures on the desktop) · Make zones by type… · Take the automatic zones back (once some were made) |
| Find an item… · New folder on the desktop… | |
| Rules ▸ | What the rules would do… · Sort the desktop by the rules · Take the sorting back (only where there are rules) |
| Take back the last change to a file | |
| Hide the zones · Every icon grey · Everything covered from view | |
| Held ▸ | Every zone held where it is · Nothing in or out of any zone |
| Look of all zones ▸ | Title ▸ · Ground ▸ · Icons ▸ · Everything as the theme |
| Arrangements ▸ | Keep this arrangement as… · each kept one ▸ (Arrange the zones like this, Rename…, Write it to a file…, Forget it) · Write this arrangement to a file… · Take an arrangement from a file… |

| A zone's menu | |
|---|---|
| Rename… · Rolled up | |
| Shown and ordered ▸ | As a grid · As a list · In my own order · By name, type, date, size · The other way round · Small, medium, large icons |
| Look ▸ | Title ▸ (Title bar, Title alone, No title) · Ground ▸ (The desktop soft behind it, Light or Solid ground, Square or Round corners, the theme's colours) · Icons ▸ (Names under the icons, Grey icons, Icons in the accent colour, Icons in their own colours) · Everything as the others |
| Held ▸ | Held where it is · Nothing in or out |
| Stays when the rest is hidden · Out of its tabs | |
| Private · Show what it holds / Cover it again · Windows keep off its place | |
| New folder in it… · Look again what is there | (zones that show a folder or fill themselves) |
| Take this zone away | |

In an item's menu, "Open with" has every program the item can be opened
with beside it (where gevrek draws the menu; on Windows the menu is the
system's own unless Shift is held).

**By keys:** the key for menus, or Shift+F10, opens the menu of what has
the keyboard (the chosen items; else the zone; else the desktop), with
the keys on its first line. Up and Down walk the lines (Home, End: the
first, the last), Right or Enter goes into a line's menu, Left comes
back, Enter or Space chooses, Escape goes one menu back and closes the
first. A screen reader is told each menu as a menu and its lines as
items, marked or not, with a menu of their own or not.

**The buttons of a title.** What is used most is also at the end of a
zone's title, without a menu. The buttons are shown while the pointer is
on the zone, while the zone has the keyboard, and always with high
contrast; otherwise the end of the title says what the zone is (how many
it holds, held, private...) as before. The name is cut short before a
button would cover it; a narrow zone keeps the last buttons (its menu
and rolling up), a very narrow one none. From the left:

| Button | A press |
|---|---|
| eye | covers what the zone holds (makes it private), shows it, covers it again |
| sort | opens "Shown and ordered" under the button |
| grid / list | shows the zone as a list, or as a grid again |
| lock | one step on each time: held where it is; then nothing in or out as well; then neither |
| angle | rolls the zone up, or down again |
| three dots | the zone's menu |

Each shows how the zone stands (a closed lock, the eye struck through,
the angle up or down). Resting the pointer on one says what a press
would do. By keys, in a zone that has the keyboard: **Tab** goes along
the buttons (Shift+Tab backwards; past the last, back to the items),
**Enter** or **Space** presses, Escape leaves them. A screen reader is
told them as buttons of the zone. With "No title" they appear in the
strip the title would be, while the pointer is on the zone. A drag that
begins on a button moves the zone, as anywhere on its title.

### Make, rename, delete

- **By pointer:** the desktop's menu has "New zone ▸": "Empty, here",
  "Of a folder…" (a portal; the folder is chosen or typed), "That fills
  itself ▸" (the files used last, what was downloaded today, the
  programs that run, the pinned programs, the pictures on the desktop)
  and "Make zones by type…" (see [Zones by type](#zones-by-type-command-line-studio-configuration)).
  A zone's menu has "Rename…" and "Take this zone away". Dragging on
  the bare desktop with the button held draws a band that chooses
  items.
- **Configuration:** an entry in `desktop.zones.zones` (see
  [Configuration](#configuration)).
- **Command line:**

  ```
  gevrek zones create --title Play
  gevrek zones create --title Here --display 2 --at 100,50 --size 400x300
  gevrek zones create --dir ~/Downloads                  # a portal
  gevrek zones create --title Papers --query ext:pdf,docx   # a smart zone
  gevrek zones create --title Recent --from recent
  gevrek zones rename play Games
  gevrek zones delete games
  ```

  `create` prints the new zone's id (`Zone 'zone-1' was made`). Deleting
  a zone leaves what it held loose on the desktop.

### Put items in, take them out, order them

- **By pointer:** drag an icon into a zone, out of it, or to another
  place within it. Files dragged in from another program are brought to
  the desktop (moved; copied with Ctrl held) and shown where they were
  dropped.
- **Command line:**

  ```
  gevrek zones add work report.pdf notes.txt     # at the end
  gevrek zones add work todo.txt --index 0       # first
  gevrek zones remove notes.txt                  # loose again
  gevrek zones place notes.txt 300,200 --display 2
  ```

- **By rules:** see [Rules](#rules).

### Open, rename, bin

- **By pointer:** a double click opens; the other button on an item
  asks for its menu. On Windows that is the system's own for the file
  (Open, Open with, Rename, the bin, Properties and what programs
  added), and the zones' own lines ("Out of its zone", "Stays when the
  rest is hidden") come with **Shift** and the other button. On Linux it
  is gevrek's: Open, "Open with another program…", Rename, the bin, "New
  folder…", Properties, and the zones' lines.
- **Keys**, while the zones have the keyboard: Enter opens, F2 renames,
  Delete puts into the bin, Ctrl+Z takes the last file change back.
- **Command line:**

  ```
  gevrek zones open report.pdf
  gevrek zones rename-item notes.txt todo.txt
  gevrek zones trash old.zip older.zip
  gevrek zones undo            # the last of these, taken back
  ```

  Nothing is ever deleted outright. `undo` takes back the last file
  operation (a rename, the bin, a move into a portal, a new folder), the
  one before that when asked again, 32 deep; it is forgotten when the
  shell restarts.

### Move, resize, snap

- **By pointer:** drag a zone by its title; drag its sides and corners
  to size it. The rim that takes hold is 8 points inside the zone's edge
  and 4 outside it, and a corner reaches 22 points along both of its
  sides. **The pointer shows it before anything is pressed**: the arrows
  of the way that side or corner goes, and the side itself is drawn in
  the accent colour; while the pointer is on a zone, three short lines
  in its lower right corner mark where it is sized by. A zone that is
  held where it is shows none of that (taken by its rim, it shakes and
  says that it is held). A rolled-up zone is only made wider or
  narrower. The zone is drawn to the display's sides, to the room panels
  leave, to other zones (with a gap between), and to a grid if one is
  set. Hold Alt to place it exactly.
- **Configuration:** `desktop.zones.snap`.
- **Command line:**

  ```
  gevrek zones move work 100,60                 # of the desktop
  gevrek zones move work 100,60 --display 2     # of that display
  gevrek zones resize work 500x300 --no-snap
  ```

  A display is named by its id as `gevrek zones list` prints it, by its
  number (1 is the first), or as `primary`.

### View, sort, icon size, look

- **By pointer:** the zone's menu has "Shown and ordered ▸" (as a grid
  or a list; by name, type, date, size or in your own order, and the
  other way round; small, medium or large icons) and "Look ▸" with
  "Title ▸", "Ground ▸" and "Icons ▸" (the ground's colour, solid or
  see-through, the desktop soft behind it, round or square corners, the
  title bar, names under the icons, the icons' colour). The desktop's
  menu has "Look of all zones ▸". Two buttons of the title do the most
  usual of it: grid or list, and "Shown and ordered".
- **Every icon grey, at once:** the middle button twice on the bare
  desktop, or "Every icon grey" in the desktop's menu, draws every icon
  (in zones and loose) without its colours, and the same again gives
  them back. It is laid over what each zone says of its own icons and
  changes none of it; it is remembered over a restart.
  `gevrek zones grey [on|off|toggle]`.
- **Configuration:** `view`, `sort`, `sort_descending`, `icon_size` and
  `look` per zone; `desktop.zones.look` for all.
- **Command line:**

  ```
  gevrek zones set work view=list sort=name descending=on icon-size=32
  gevrek zones set media opacity=70 blur=on corner-radius=10 title-bar=plain
  gevrek zones set media background=#1b1f2acc icon-saturation=60 labels=off
  gevrek zones set media opacity=none      # as the other zones again
  ```

  The keys of `set`: `view`, `sort`, `descending`, `icon-size`,
  `background`, `opacity`, `blur`, `corner-radius`, `title-colour`,
  `title-on-hover`, `title-bar`, `labels`, `icon-tint`,
  `icon-saturation`, `always-visible`, `private`, `reserves-space`.

### Roll up

A rolled-up zone shows only its title. It opens for a moment when the
pointer rests on the title (or on a press, if set so), and closes again
a little after the pointer left. Something dragged over it always opens
it, so that it can be dropped inside.

- **By pointer:** double click on the title, the angle at the end of
  the title, or "Rolled up" in its menu. Where zones open at a press
  (`expand_on: click`), a click on the title of a rolled-up zone opens
  it for now.
- **Keys**, in a zone that has the keyboard: Ctrl+R.
- **Keys:** `{"command": "roll_up", "zone": "@focused"}` (R in the zones
  mode of the example below).
- **Configuration:** `rolled_up` per zone; `desktop.zones.roll_up` for
  how it opens: `expand_on` (`hover` or `click`), `expand_delay_ms`
  (150), `collapse_delay_ms` (400); `expand_on` per zone.
- **Command line:** `gevrek zones roll-up work` (the other of what it
  is), `roll-up work on`, `roll-up work off`.

### Tabs

Zones can share one place and be shown as tabs.

- **By pointer:** drag a zone by its title onto another's title or
  tabs (the other gets a rim and says that letting go makes it a tab
  there; with Alt held the zone is only moved); drag a tab along its row
  to reorder; drag it above or below the row to make it a zone of its
  own again, or "Out of its tabs" in its menu; press a tab to bring it
  to the front.
- **Keys**, in a zone that has the keyboard: Ctrl+Tab and
  Ctrl+Shift+Tab.
- **Configuration:** the same `tab_group` name on each zone; their order
  in the file is the order of the tabs.
- **Command line:**

  ```
  gevrek zones tab play --with work
  gevrek zones front play
  gevrek zones next-tab work [--backwards]
  gevrek zones untab play
  ```

### Hide, show, peek

- **Quick hide** puts every zone away at once, for a clear desktop. It
  is remembered over a restart. A zone, or a single item, can be set to
  stay.
- **Peek** brings the zones above the windows, to fetch something
  without moving a window. The same key, Escape, or opening an item puts
  them back.
- **Hide one zone** on the displays as they are now.

| | |
|---|---|
| By pointer | **the main button twice on the bare desktop** hides and shows (where no zone and no icon is; while they are hidden, anywhere a zone was is bare desktop too); the desktop's menu has "Hide the zones" (and, while they are hidden, only "Show the zones"); a zone's menu and an item's have "Stays when the rest is hidden" |
| Keys | **Super+Z**: peek, and again to end it. **Super+Shift+Z**: hide or show |
| Configuration | `always_visible` per zone |
| Command line | `gevrek zones hide`, `show` (or `unhide`), `toggle`; `hide work`, `unhide work`; `peek`, `peek on`, `peek off`; `keep-zone work on`; `keep-item todo.txt on` |

`gevrek zones show` alone shows the zones again; `gevrek zones show work`
(or `info work`) prints one zone with its items.

**The middle button twice on the bare desktop** makes every icon grey,
or gives the colours back (see [View, sort, icon size, look](#view-sort-icon-size-look)).
What it does can be changed: `desktop.zones.gestures.double_middle_click`
is one of `none`, `quick_hide`, `grey_icons` (as it comes), `peek`.
Twice with the main button hides and shows, always. Twice is what it is
in the system's own settings (the time between the two presses, and how
far apart they may be).

### Lock

A zone's **place** can be locked (no move, no resize, no tabbing), and
what it **holds** can be (nothing in or out, by hand, by a rule, or into
the bin). All zones can be locked at once, whatever each one says.

- **By pointer:** "Held ▸" in the zone's menu has "Held where it is"
  and "Nothing in or out"; "Held ▸" in the desktop's has "Every zone
  held where it is" and "Nothing in or out of any zone". The lock at the
  end of a zone's title goes one step on with each press: held where it
  is; then nothing in or out as well; then neither.
- **Configuration:** `lock: {geometry, members}` per zone;
  `desktop.zones.lock` for all.
- **Command line:**

  ```
  gevrek zones lock work               # both
  gevrek zones lock work geometry
  gevrek zones lock work members
  gevrek zones unlock work
  gevrek zones lock-all                # every zone; `lock-all none` ends it
  ```

### Portals

A portal shows a folder. What is dropped on it is moved into the folder;
what is dragged out of it onto the desktop is moved to the desktop. It
can walk into the folders it shows without leaving the zone, and never
above the folder it was given.

- **By pointer:** double click on a folder goes in; the way in the
  portal's title (an angle for back, an arrow for up, each folder's
  name) goes where it is pressed. Drop a file on the portal to move it
  into the folder it stands in, or on a folder it shows to move it into
  that. The zone's menu has "New folder in it…" and "Look again what is
  there"; the desktop's has "New folder on the desktop…", "Take back the
  last change to a file" and, while a long copy or move is under way,
  "Call off what is being copied or moved". Typing while the zone has
  the keyboard filters by name.
- **Configuration:**

  ```json
  { "id": "dl", "title": "Downloads",
    "kind": { "type": "portal", "directory": "C:\\Users\\ada\\Downloads" },
    "view": "list", "sort": "date", "sort_descending": true }
  ```

- **Command line:**

  ```
  gevrek zones create --dir ~/Downloads --title Downloads
  gevrek zones go dl 2026          # into a folder it shows
  gevrek zones go dl up            # also: back, forward
  gevrek zones new-folder dl Invoices
  gevrek zones new-folder --in ~/Desktop Projects   # anywhere that is no portal
  gevrek zones move-to-folder a.pdf b.pdf --folder ~/Downloads/Invoices
  gevrek zones bring-in /media/usb/film.mkv --folder ~/Desktop --copy
  gevrek zones cancel              # call off the copy or move under way
  gevrek zones filter dl inv       # only what fits; `filter dl` for all again
  gevrek zones refresh dl
  ```

  `cancel` calls off the one operation there is, lists them when there
  are several, and takes a number (`cancel 7`) to name one.

A folder that is slow (a network share) shows as being read, and one
that cannot be reached says so (`gevrek zones show dl` prints `listing
unavailable: ...`); the zone stays where it is and fills again when the
folder is back (it is asked again every minute, and with a press on a
zone that says "Not there now").

### Smart zones

A smart zone shows everything that matches all parts of its query,
without taking anything from where it is. It can look at the desktop
(the default), at any folder, at the downloads, at the files used last,
at the pinned launchers, or at the programs that are running.

```json
{ "id": "today", "title": "Downloaded today",
  "kind": { "type": "smart", "query": [
    { "match": "date", "field": "modified", "newer_than_days": 1 } ] },
  "source": { "from": "downloads" }, "sort": "date", "sort_descending": true },
{ "id": "recent", "title": "Recent",
  "kind": { "type": "smart", "query": [] },
  "source": { "from": "recent" }, "limit": 12 }
```

`source` is `{"from": "desktop"}`, `recent`, `downloads`, `pinned`,
`running_apps`, or `{"from": "folder", "directory": "..."}`. Of the
desktop, a zone with an empty query shows nothing; of every other source
it shows all the source has. `limit` is how many it shows at most.

On the command line the query is the rules' own JSON, or a short form:

```
gevrek zones create --title Papers --query ext:pdf,docx
gevrek zones create --title "Big and old" --query "larger:100000000+untouched:90"
gevrek zones create --title Pictures --query "mime:image/*" --from ~/Pictures
gevrek zones create --title Recent --from recent
```

The parts of the short form, joined with `+`: `ext:pdf,docx`,
`name:report*`, `mime:image/*`, `kind:folder`, `target:*chrome*` (where a
shortcut leads), `untouched:30` (days), `older:30`, `newer:7` (days since
changed), `larger:N`, `smaller:N` (bytes).

### Rules

Rules sort items by themselves. They are tried in order for each item;
the first that matches decides, and `catch_all` says what happens to
what none matches (without it such an item stays where it is). A rule's
matchers must all hold.

```json
{ "desktop": { "zones": {
  "rules_on_arrival": true,
  "rules": [
    { "name": "Papers",
      "matchers": [ { "match": "extension", "any_of": ["pdf", "docx"] } ],
      "action": { "action": "assign_to_zone", "zone": "work" } },

    { "name": "Screenshots",
      "matchers": [ { "match": "name", "pattern": "Screenshot*" },
                    { "match": "mime", "pattern": "image/*" } ],
      "action": { "action": "assign_to_zone", "zone": "pictures" } },

    { "name": "Stale",
      "matchers": [ { "match": "untouched", "days": 30 } ],
      "action": { "action": "assign_to_zone", "zone": "archive" } },

    { "name": "Old installers",
      "matchers": [ { "match": "extension", "any_of": ["exe", "msi"] },
                    { "match": "date", "field": "modified", "older_than_days": 60 } ],
      "action": { "action": "move_to_folder", "folder": "D:\\Archive\\Installers" } }
  ],
  "catch_all": null,
  "timed_rules": { "every_minutes": 60, "move_files": false }
} } }
```

- **Papers** and **Screenshots** show an item in a zone. The file is not
  moved.
- **Stale** is a rule that goes by time: an item nobody made, changed or
  opened for 30 days goes to the zone `archive`. Rules like it
  (`untouched`, and `date` with `older_than_days` or `newer_than_days`)
  come true by themselves, so they are tried again every
  `timed_rules.every_minutes` (60; 0 for never), for every item, also
  the ones sorted by hand.
- **Old installers** moves the file itself, into the folder. A rule that
  moves files is only carried out when you ask for the rules to be
  applied, never on arrival; the timed ones only with
  `timed_rules.move_files`. The move goes through the system and is
  taken back by `gevrek zones rules undo`.

The matchers: `extension {any_of}`, `mime {pattern}`, `name {pattern}`,
`kind {any_of}` (`file`, `folder`, `shortcut`, `launcher`, `virtual`),
`date {field, after, before, newer_than_days, older_than_days}` (`field`
is `created`, `modified` or `accessed`), `size {at_least, at_most}`
(bytes), `shortcut_target {pattern}`, `untouched {days}`. Patterns are
globs (`*`, `?`), without regard to case.

`rules_on_arrival` (on by default) puts an item that comes to the
desktop into the zone its rule names, at once.

**Icons that come by themselves.** A program's installation leaves a
shortcut on the desktop that nobody put there. Such an icon (a shortcut
or a launcher that comes while gevrek runs) goes into a zone at once,
also where no rule decides about it: into the zone made for its type
(`gevrek zones auto-create`), or else into the first zone filled by hand
that is not locked, of those shown on this workspace before the others.
A rule that matches it decides instead, and what you bring to the
desktop yourself lies where you let it go.

Where no zone takes it, it lies loose. On Windows, where Explorer's own
icons are put away (`desktop.platform.windows.hide_desktop_icons`, see
[Windows: Explorer's icons](#windows-explorers-icons)), it is then not
drawn at all: a desktop that was cleared stays clear. It is still an
item of the desktop (`gevrek zones list` names it among the loose ones,
the search finds it, the rules and `auto-create` sort it), and it is
drawn from the moment it is put somewhere: `gevrek zones add` puts it
into a zone, `gevrek zones place` at a place of its own. Files and
folders that come to the desktop are always drawn.

**See first, then do.** A dry run changes nothing:

```
$ gevrek zones rules dry-run --scope all
The rules would sort 3 items (asked of every item of the desktop):
  ITEM       FROM        GOES TO                                       BY
  cat.png    zone inbox  zone pictures (Pictures)                      rule 2 "Screenshots"
  old.exe    loose       folder D:\Archive\Installers (the file is moved)  rule 4 "Old installers"
  a.pdf      loose       zone work (Work)                              rule 1 "Papers"
Left where it is (1):
  ITEM       FROM        BECAUSE                                         BY
  setup.exe  zone tools  its zone, or the one it would go to, is locked  rule 4 "Old installers"
Nothing was changed. `gevrek zones rules apply` carries it out.
```

| | |
|---|---|
| By pointer | the desktop's menu: "What the rules would do…" opens a sheet with the same list, and "Sort now" and "Take the last sorting back" on it; "Sort the desktop by the rules" does it at once for the items in no zone; "Take the sorting back" |
| Command line | `gevrek zones rules list`, `rules dry-run [--scope loose\|all]`, `rules apply [--scope loose\|all]`, `rules check` (the timed ones, now), `rules undo` |

`--scope loose` (the default) asks only about the items in no zone, so
what you sorted by hand stays sorted; `all` asks about every item.
`rules undo` takes back the last application, file moves included.

### Zones by type (command line, studio, configuration)

For a desktop that was never sorted: a zone for each type of item, made
and filled in one go, and taken back in one go.

```
$ gevrek zones auto-create --dry-run
3 zones would be made on display DEL-A1B2-77, and 7 items sorted (asked of the items that are in no zone):
  ZONE       IS   ITEMS  AT      SIZE     RULE
  Folders    new  2      8,48    172x126  a rule for what comes later
  Documents  new  2      188,48  172x126  a rule for what comes later
  Pictures   new  3      368,48  172x212  a rule for what comes later
Left where it is (1; too few of a type for a zone of its own, or its zone is locked): song.mp3
Nothing was changed. `gevrek zones auto-create` carries it out.
$ gevrek zones auto-create
$ gevrek zones auto-undo
```

| | |
|---|---|
| By pointer | the desktop's menu, "New zone ▸ Make zones by type…": a sheet shows which zones would be made and what would go into each, with a small picture of the display and where they would stand (new ones in the accent colour, numbered as in the list). On it: loose items only or also what lies in zones filled by hand, the ticked box "Sort new items of these types too" (a rule for each zone; unticked, none is added), "Make them", "Take the automatic zones back", "Close". The new zones are made on the display the menu was opened on. Once some were made, "New zone ▸ Take the automatic zones back" is in the menu too |
| Command line | `gevrek zones auto-create [--dry-run] [--scope loose\|all] [--min-items N] [--no-rules] [--display NAME]`, `gevrek zones auto-undo` |
| Studio | page **Zones**, card "Zones by type": "Make zones by type…" shows what would be made, "Make them" makes it, "Take the automatic zones back" (while there is something to take back) |
| Configuration | `desktop.zones.auto` (below, under [Configuration](#configuration)) |

- An item is of the first type that names its **kind** (a folder is a
  folder whatever it is called); else of the first that names its
  **extension**; else of the first whose **MIME** beginnings fit; else of
  the type that names nothing (`other`), which takes the rest. This PC
  and the bin are never sorted.
- A type gets a zone when at least `min_items` items are of it (2, as
  it comes); with fewer, its items stay where they are.
- New zones stand in rows from the top left corner of the display's
  free room, the snap gap between them, each as large as what it holds
  (2 to 5 icons wide, at most 4 rows, then it scrolls), never on a zone
  that is there and never on an icon that stays lying loose (what goes
  into no zone keeps its place, and the zones begin beside it). Where
  the room is short a zone is made small; where there is none, the rest
  are laid over what is there, a step apart.
- A zone made so remembers its type. Asking again makes no second one:
  what is new of the type goes into the zone there is, however little,
  and a desktop that is sorted gives "nothing to make". Rename and move
  such a zone as you like.
- `--scope loose` (the default) sorts the items in no zone; `all` also
  what the zones you filled by hand hold (not what is in a zone made by
  type already).
- With each new zone comes a rule, called `Auto: <title>`, that sends
  what comes to the desktop later into it: by the type's extensions, or
  by its kinds where it has no extensions (the folders). The type that
  takes the rest has no rule. `--no-rules`, or `auto.add_rules: false`,
  makes none.
- `auto-undo` takes back the last `auto-create`: its zones go, every
  item goes back where it was (into the zone and the place it had, or
  loose where it lay), its rules are taken out. What you changed since
  stays: an item you put elsewhere, a zone you locked or gave another
  purpose, a rule you renamed.
- A zone locked to what it holds takes nothing; with every zone locked
  (`lock-all`) nothing is made at all.

### Snapshots (saved layouts)

A snapshot is a named copy of everything that makes up the layout: the
zones, the rules, which item is where, and where each zone is on each
set of displays.

```
gevrek zones snapshot save Work
gevrek zones snapshot list
gevrek zones snapshot restore Work        # or: gevrek zones layout apply Work
gevrek zones snapshot rename Work Office
gevrek zones snapshot delete Office
gevrek zones snapshot export Work work.json       # to carry elsewhere
gevrek zones snapshot export @current now.json    # the layout as it is
gevrek zones snapshot import work.json            # kept as a copy
gevrek zones snapshot import work.json --apply    # and put in force
```

By pointer they are called arrangements: the desktop's menu,
"Arrangements…", has "Keep this arrangement as…", each kept one by its
name (go back to it, rename it, forget it, "Write it to a file…"), and
"Take an arrangement from a file…". The file's path is typed.

Whatever replaces the whole layout keeps the one there was first:
restoring and importing save it as a snapshot of their own (`Before
restoring`, `Before import`; the last five of these are kept). Importing
never replaces a snapshot: a name that is taken gets a number. A file
that is not a layout, or holds what a configuration may not, is refused
and changes nothing.

### Profiles (one arrangement for each set of displays)

Where the zones are is remembered for each set of displays: laptop
alone, laptop and the monitor at the desk, the projector. Docking,
undocking and changing a resolution or a scale choose the arrangement
that belongs, and a set never seen before starts from the nearest one.
Positions are kept from a corner or side of a display, so a zone at the
right edge stays at the right edge.

```
$ gevrek zones profile list
Arrangements remembered: 2
  * 1  DEL-A1B2-77 1920x1080@1.00 · 3 zones placed
    2  DEL-A1B2-77 1920x1080@1.00 + LG-0042 2560x1440@1.25 · 3 zones placed
(* is the one in use; `gevrek zones profile copy <number>` takes another over)
$ gevrek zones profile show          # where each zone is, in the one in use
$ gevrek zones profile copy 2        # place the zones as in arrangement 2
```

With `desktop.zones.profile_per_workspace` each workspace has an
arrangement of its own, and with it its own set of hidden zones. A zone
can also be tied to workspaces whatever the displays are:
`"workspaces": ["dev", "2"]`.

### Tiling

A zone lies under the windows. With `"reserves_space": true` tiled
windows keep off its place, as they keep off a panel: "Windows keep off
its place" in the zone's menu, or `gevrek zones set dock
reserves-space=on`. The room is kept on both systems; it has not been
seen on a desktop yet.

### Keyboard

What has the keyboard is one zone, and perhaps one of its items.

```
gevrek zones focus work          # by name
gevrek zones focus left          # the next zone on that side; also right, up, down
gevrek zones focus desk          # the bare desktop and its loose items
gevrek zones focus none
gevrek zones go-to report.pdf    # the item, wherever it is: its tab comes
                                 # to the front, a rolled-up zone opens
gevrek zones walk down [--extend]     # from item to item
gevrek zones select a.txt b.txt --mode add
gevrek zones move-chosen left         # or a zone's name, or desk
gevrek zones open-chosen
gevrek zones dismiss                  # what Escape does
```

**The keys of a zones surface**, once it has the keyboard (after a press
on it, or while peeking):

| Key | What |
|---|---|
| Arrows, Home, End (with Shift: choosing as well) | from item to item in the zone that has the keyboard, or among the loose items of the bare desktop |
| Alt+arrows, Alt+H J K L | the keyboard to the zone (or the bare desktop) that lies that way |
| Alt+Shift+arrows, Alt+Shift+H J K L | what is chosen into the zone that lies that way, or out onto the desktop |
| Enter | open |
| Letters, Backspace | while peeking: search everywhere; in a zone: its filter; on the bare desktop: to the first name that begins so |
| Ctrl+Tab, Ctrl+Shift+Tab | the next tab, the one before |
| Ctrl+R | roll the zone up or down |
| Ctrl+A, F2, Delete, Ctrl+Z | choose all, rename, the bin, take the last file change back |
| Tab, Shift+Tab | along the buttons of the title of the zone that has the keyboard; past the last, back to the items |
| Enter or Space, on a button of the title | press it |
| The key for menus, or Shift+F10 | the menu of what has the keyboard: of the chosen items, else of the zone, else of the desktop |
| In a menu: Up, Down, Home, End; Right or Enter; Left; Escape | walk its lines; into a line's menu, or choose the line; one menu back; one menu back, and out of the first |
| Escape | let go of a drag; else stop peeking, empty what was typed, choose nothing, let the keyboard go |

These are the keys of a surface that already has the keyboard. The keys
that work from anywhere are bindings of the window manager:

**The standard keys** (in `standard_bindings`, so in every showcase
example and in the configuration gevrek starts with):

| Keys | What | The binding's command |
|---|---|---|
| Super+Z | the zones above the windows; again, back below | `{"command":"zones","action":{"command":"toggle_peek"}}` |
| Super+Shift+Z | hide the zones, or show them | `{"command":"zones","action":{"command":"quick_hide"}}` |

Z is in nobody's way: neither niri's default keys, which gevrek's
follow, nor any of gevrek's own use Super+Z.

**Peek only while the key is held**, if you prefer it: two bindings on
Super+Z in place of the standard one, the first on the press and the
second with `"trigger": "release"`:

```json
{ "name": "Zones above the windows (held)", "key": "Z", "super": true,
  "command": { "command": "zones", "action": { "command": "peek", "on": true } } },
{ "name": "Zones back below the windows", "key": "Z", "super": true, "trigger": "release",
  "command": { "command": "zones", "action": { "command": "peek", "on": false } } }
```

It works whichever of the two keys is let go first: a release is
matched with the modifiers that were held when the key went down.

**More keys, proposed.** They are not in the standard set. The example
"Advanced Binding Lab" has them as a mode (a submap named `zones`),
entered with **Super+Ctrl+Z**; inside, no modifier is held:

| Keys in the mode | What | The zones command |
|---|---|---|
| arrows | the zone on that side | `{"command":"focus_neighbour","direction":"left"}` |
| H J K L | the item on that side | `{"command":"move_item_focus","step":"left"}` |
| Shift+arrows | move the chosen items to the zone on that side | `{"command":"move_focused_items","direction":"left"}` |
| Enter | open what is chosen | `{"command":"open_focused"}` |
| R | roll this zone up or down | `{"command":"roll_up","zone":"@focused"}` |
| Tab | the next tab of this zone | `{"command":"next_tab","zone":"@focused"}` |
| Z | the zones above the windows, or back | `{"command":"toggle_peek"}` |
| Space | hide or show the zones | `{"command":"quick_hide"}` |
| M | cover what the zones hold, or uncover | `{"command":"mask"}` |
| Escape | the zones back below the windows, and leave the mode | `{"command":"peek","on":false}` on the press, the mode left on the release |

Any command of the zones can be put on a key the same way: a binding
whose command is `{"command": "zones", "action": { ... }}`, with the
zones command as `action`. In a binding, `"zone": "@focused"` means the
zone that has the keyboard.

### Search

```
$ gevrek zones search inv
1 item fits 'inv':
  NAME         WHERE             ID
  invoice.pdf  zone work (Work)  /home/ada/Desktop/invoice.pdf
```

By pointer and keys: Super+Z and type (a line at the top says "Type to
find anything in the zones"), or "Find an item…" in the desktop's menu.
Everything on the desktop, the folder each portal stands in, and what
every smart zone shows, ranked as the launcher ranks (the whole name,
its beginning, the beginning of a word, any part). `gevrek zones go-to
invoice.pdf` then goes there. What a covered zone holds is not found.

### Privacy

A zone can be **private**: what it holds is covered until you ask to
see it. Everything can be covered at once by hand. When the screen is
shared or recorded, what was uncovered is covered again.

- **Configuration:** `"private": true` per zone;
  `desktop.zones.privacy.everything` (every zone and the loose items);
  `desktop.zones.privacy.when_shared`: `private` (the default: the
  private zones are covered again), `everything`, or `nothing`.
- **By pointer:** "Private" in the zone's menu. A covered zone shows a
  cloth in place of its items ("8 items, covered", "Press to show"):
  press it, or "Show what it holds" in its menu; "Cover it again" puts
  the cloth back. The desktop's menu has "Everything covered from view".
- **Command line:**

  ```
  gevrek zones privacy on              # cover everything
  gevrek zones privacy off             # uncover, the private zones too
  gevrek zones privacy                 # the other of what it is
  gevrek zones privacy on --zone bank  # make that zone private, or cover it again
  gevrek zones privacy off --zone bank # see it for now
  gevrek zones set bank private=off    # not private any more
  ```

Seeing a private zone lasts until the screen is next shared, until
everything is covered by hand, or until the shell restarts.

On **Linux** gevrek is the compositor and knows when the screen is
shared or recorded, so covering is automatic. On **Windows** nothing
tells a program reliably that the screen is shared. So there, while any
zone is private (or everything is) and `when_shared` is not `nothing`,
the zones' surfaces are left out of every picture and recording of the
screen, also while you have asked to see what a zone holds: seen on the
display, absent from what others get. And everything can be covered by
hand (`gevrek zones privacy on`, or a key bound to `{"command":"mask"}`).

A covered zone's items are not drawn, not read out by a screen reader,
not found by the search and cannot get the keyboard. `gevrek zones show`
still names them: you asked.

## Configuration

Everything under `desktop.zones`. Every key has a default; a file
without it reads with that. The studio's page **Zones** edits the same
keys.

```json
{ "desktop": { "zones": {
  "enabled": true,
  "zones": [
    { "id": "work", "title": "Work",
      "kind": { "type": "manual" },
      "view": "grid", "sort": "manual", "sort_descending": false,
      "icon_size": 48,
      "look": { "background": "#1b1f2acc", "opacity": 85 },
      "lock": { "geometry": false, "members": false },
      "rolled_up": false, "expand_on": "hover",
      "always_visible": false, "private": false,
      "reserves_space": false, "workspaces": [],
      "tab_group": null }
  ],
  "rules": [], "catch_all": null,
  "rules_on_arrival": true,
  "timed_rules": { "every_minutes": 60, "move_files": false },
  "profile_per_workspace": false,
  "loose_icon_size": 48,
  "look": { "opacity": 70, "blur": true, "corner_radius": 10 },
  "snap": { "enabled": true, "distance": 12, "gap": 8, "grid": 0 },
  "roll_up": { "expand_on": "hover", "expand_delay_ms": 150, "collapse_delay_ms": 400 },
  "lock": { "geometry": false, "members": false },
  "privacy": { "everything": false, "when_shared": "private" },
  "gestures": { "double_middle_click": "grey_icons" },
  "auto": { "min_items": 2, "add_rules": true,
            "categories": [ { "id": "pictures", "title": "Pictures",
                              "extensions": ["png", "jpg"], "mime_prefixes": ["image/"], "kinds": [] } ] }
} } }
```

| Key | |
|---|---|
| `enabled` | whether zones are shown at all (on unless it says `false`; on Windows Explorer's desktop icons are put away while they are shown, unless `desktop.platform.windows.hide_desktop_icons` is `false`) |
| `zones[]` | the zones; see below |
| `rules[]`, `catch_all`, `rules_on_arrival`, `timed_rules` | [Rules](#rules) |
| `profile_per_workspace` | each workspace has an arrangement of its own |
| `loose_icon_size` | how large the items in no zone are drawn (16 to 256) |
| `look` | the look of every zone that does not set its own |
| `snap` | `enabled`; `distance` from which an edge pulls; `gap` left between two zones; `grid` (0 for none) |
| `roll_up` | how a rolled-up zone opens for a moment |
| `lock` | what no zone lets be changed, whatever its own lock says |
| `privacy` | `everything`, `when_shared` |
| `gestures` | `double_middle_click` (as it comes `grey_icons`): what pressing the bare desktop twice with the middle button does; `none`, `quick_hide`, `grey_icons` or `peek` |
| `auto` | [zones by type](#zones-by-type-command-line-studio-configuration): `min_items` (2; 1 to 1000), `add_rules` (on), `categories[]` |

A zone:

| Key | |
|---|---|
| `id` | its name in the configuration and on the command line; unique |
| `title` | what is shown |
| `kind` | `{"type":"manual"}`, `{"type":"portal","directory":...}`, `{"type":"smart","query":[...]}` |
| `source`, `limit` | for a smart zone: what it looks through, and how many it shows (0 for all) |
| `view` | `grid` or `list` |
| `sort`, `sort_descending` | `manual`, `name`, `type`, `date`, `size` |
| `icon_size` | 16 to 256 |
| `look` | where it differs from the others |
| `lock` | `geometry`, `members` |
| `rolled_up`, `expand_on` | only the title is shown; `hover` or `click` to open it for a moment |
| `always_visible` | stays when the zones are hidden |
| `private` | covered until asked for |
| `reserves_space` | tiled windows keep off it |
| `workspaces` | the workspaces it is shown on (none named: all) |
| `tab_group` | zones with the same name here are tabs of one place |

The types of `auto.categories`, as they come. Each has `id` (what its
zone is known by), `title` (what the zone is called), `extensions`
(small letters, no dot), `mime_prefixes` and `kinds` (`folder`,
`shortcut`, `launcher`, `file`). The order counts: the first that fits
takes the item, kind before extension before MIME, and a type that
names nothing takes the rest. Change, reorder, add or remove them as
you like; a list you write replaces this one whole.

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

A zone that was made for a type has `"auto": "<id>"` among its keys;
take it off and the zone is yours like any other.

A look (each key optional; what a zone leaves out is the look of all
zones, and what that leaves out is the theme's):

| Key | |
|---|---|
| `background` | `#rrggbb` or `#rrggbbaa` |
| `opacity` | hundredths, 0 to 100 |
| `blur` | what is behind is blurred, where the system can |
| `corner_radius` | 0 to 64 |
| `title_bar` | `full`, `plain` (the title alone), `hidden` |
| `title_colour` | a colour |
| `title_on_hover` | the title only while the pointer is over the zone |
| `labels` | the items' names are shown |
| `icon_tint` | a colour laid over the icons |
| `icon_saturation` | hundredths, 0 (grey) to 100 |

**What is not in the configuration.** Which item is in which zone, where
each zone is on each set of displays, the snapshots and whether the
zones are hidden change with every drag. They are the shell's own file
(`state/zones/zones.json` beside the configuration, written with
backups), and `gevrek zones snapshot export` is how to carry them.

Keys this version does not know are kept and written back as they were.

## The command line

```
gevrek zones list [--items]            show [<zone>]         events
gevrek zones create ...                delete <zone>         rename <zone> <title>
gevrek zones move <zone> X,Y           resize <zone> W,H     set <zone> key=value ...
gevrek zones roll-up <zone> [on|off|toggle]
gevrek zones lock <zone> [geometry|members|all|none]         unlock <zone>     lock-all [...]
gevrek zones tab <zone> --with <zone>  untab <zone>          front <zone>      next-tab <zone>
gevrek zones add <zone> <item...>      remove <item...>      place <item> X,Y
gevrek zones open <item...>            rename-item <item> <name>   trash <item...>   undo
gevrek zones hide [<zone>]             unhide [<zone>]       toggle            peek [on|off|toggle]
gevrek zones keep-zone <zone> [on|off|toggle]                keep-item <item> [on|off|toggle]
gevrek zones go <zone> up|back|forward|<folder>              refresh <zone>
gevrek zones new-folder <zone> <name> | --in <folder> <name>  filter <zone> [text]
gevrek zones move-to-folder <item...> --folder <folder> [--copy]
gevrek zones bring-in <path...> --folder <folder> [--zone <zone>] [--copy]
gevrek zones cancel [<number>]         expand <zone> [on|off|toggle]
gevrek zones move-tab <zone> <index>   split-tab <zone> X,Y
gevrek zones settings key.path=value ...
gevrek zones privacy [on|off|toggle] [--zone <zone>]     grey [on|off|toggle]
gevrek zones auto-create [--dry-run] [--scope loose|all] [--min-items N] [--no-rules] [--display NAME]
gevrek zones auto-undo
gevrek zones rules list|dry-run|apply|check|undo [--scope loose|all]
gevrek zones snapshot save|restore|list|delete|rename|export|import ...     (also: layout)
gevrek zones profile list|show|copy <number>
gevrek zones focus <zone>|left|right|up|down|desk|none       go-to <item>
gevrek zones walk <step> [--extend]    select [<item...>] [--mode replace|add|remove|toggle]
gevrek zones move-chosen <zone>|left|right|up|down|desk      open-chosen       dismiss
gevrek zones search <text>
gevrek zones send '<json>'             restore-explorer
```

`gevrek zones <word> --help` says what each takes.

`grey` draws every icon of the desktop grey, or in its colours again.
It is a way of looking at the desktop and is remembered over a restart;
no zone's look is changed by it (`icon_saturation` stays as it is set).

**For scripts.**

- `--json` on any word prints JSON. What lists prints one object
  (`list`: `{running, stale, sequence, version, published_ms, message,
  state}`; `show`: `{running, stale, sequence, zone}`). What asks
  something of the zones prints their answer, `{success, delivered,
  error, events}`, with what the word adds: `zone` (the new id) for
  `create`, `lines` for the rules, `hits` for `search`, `file` for
  `export`, `plan` for `auto-create --dry-run` (`{zones: [{category,
  title, zone?, rect, items, rule}], left, display}`), `made` for
  `auto-create` and `undone` for `auto-undo`.
- `list --json` leaves the items out (each zone has its `item_count`,
  the desktop its `loose_count`): a status bar asks often. `list --json
  --items` has them all; `show <zone> --json` has one zone's.
- `--timeout-ms N` is how long the answer is waited for (5 seconds
  where nothing is said, never more than 30).
- The exit code:

  | | |
  |---|---|
  | 0 | done |
  | 1 | not done: the zones refused (their reason is printed), or what was named is not there, or two things have the name |
  | 2 | the words could not be read |
  | 3 | no shell runs the zones |
  | 4 | the shell was asked and did not answer in time; it may still do it |

  `list --json` ends with 0 also when no shell runs: the JSON says it
  (`"running": false`).
- `send` takes any command of the zones as JSON, as a key binding does:

  ```
  gevrek zones send '{"command":"always_visible","zone":"@focused"}'
  ```

  What holds for all zones is changed with `settings`, by the
  configuration's own keys: `gevrek zones settings snap.grid=16
  roll_up.expand_on=click look.opacity=70`.

  Every command is in [core-milestones-3-5.md](core-milestones-3-5.md).
  The same through the call every client has:
  `gevrek dispatch '{"command":"zones","action":{"command":"quick_hide"}}'`.

### Events, for status bars

```
$ gevrek zones events --json
{"event":"zones.attached","timestamp_ms":1795113838100,"session":1}
{"event":"zones.visibility_changed","timestamp_ms":1795113838803,"sequence":4,"data":{"event":"visibility_changed","quick_hidden":true,"peeking":false}}
{"event":"zones.items_changed","timestamp_ms":1795113840112,"sequence":5,"data":{"event":"items_changed","zone":"work"}}
```

One JSON object a line, each written out at once, until the command is
stopped. `event` is `zones.` and the event's name; `timestamp_ms` is
milliseconds since 1970; `sequence` is the number of the snapshot there
was when it happened (the snapshot that shows the change has a higher
one, so `gevrek zones list --json` after an event is never older than
the event); `data` is the event itself.

| `event` | `data` | When |
|---|---|---|
| `zones.attached`, `zones.detached` | (none; `session`) | a shell begins, or stops, running the zones |
| `zones.zone_created`, `zone_changed`, `zone_deleted` | `zone` | a zone was made; its title, view, lock, roll-up, tabs or look changed; it was taken away |
| `zones.zone_placed` | `zone`, `rect` | a zone was moved or resized |
| `zones.zone_expanded` | `zone`, `expanded` | a rolled-up zone opened for a moment, or closed |
| `zones.items_changed` | `zone` (absent: the loose items) | what a zone holds changed |
| `zones.layout_changed` | | everything may be elsewhere: the displays, the configuration or the arrangement changed |
| `zones.profile_changed` | `made`, `moved` | another arrangement is in use (other displays, another workspace) |
| `zones.visibility_changed` | `quick_hidden`, `peeking` | hidden, shown, peeking |
| `zones.privacy_changed` | `masked`, `screen_shared` | covered or uncovered; the screen is shared or no longer |
| `zones.focus_changed` | `zone`, `item` (each may be absent) | the keyboard is elsewhere |
| `zones.selection_changed` | `items` | other items are chosen |
| `zones.rules_previewed`, `rules_applied` | `plan`, `lines` | a dry run; the rules were carried out |
| `zones.rules_undone` | | the last application was taken back |
| `zones.snapshot_saved`, `snapshot_restored`, `snapshot_deleted` | `name` | |
| `zones.snapshot_renamed` | `name`, `to` | |
| `zones.layout_exported` | `document` | |
| `zones.layout_imported` | `name`, `applied`, `kept_as` | |
| `zones.navigated` | `zone`, `directory` | a portal went into another folder |
| `zones.listing_changed` | `zone`, `listing` (`state`: `loading`, `ready`, `unavailable`; `reason`) | a folder is being read, was read, or cannot be |
| `zones.filter_changed` | `zone`, `typed` | |
| `zones.found` | `typed`, `hits` | a search was answered |
| `zones.open_requested` | `items`, `with` | items are being opened |
| `zones.reserved_changed` | `regions` | the room tiled windows keep off changed |
| `zones.settings_changed` | | something of `desktop.zones` that is no zone's own changed |
| `zones.operation_begun` | `request`, `operation` | a copy or move that takes long began; `request` is the number `gevrek zones cancel` takes |
| `zones.operation_progress` | `request`, `done`, `total` (0: not known) | how far it is |
| `zones.operation_ended` | `request` | it is over (after `operation_failed` if it did not end well) |
| `zones.operation_failed` | `operation`, `error` | the system could not do a file operation, or it was called off |
| `zones.refused` | `reason` | a command was not carried out |
| `events.dropped` | `count` (beside `event`, no `data`) | this listener was too slow and missed some: ask for the state anew |

A status bar that shows, say, whether the zones are hidden reads
`gevrek zones list --json` once (`state.quick_hidden`) and then follows
`zones.visibility_changed`; after `events.dropped`, `zones.attached` or
`zones.detached` it reads the list again. Without `--json` the same is
printed for people (`[zones] {...}`).

## Windows: Explorer's icons

Zones draw every item of the desktop themselves, so while they are shown
Explorer's own icons are put away. Nothing is injected into Explorer and
nothing of the desktop is lost:

1. Before anything is touched, how the desktop is (that the icons were
   shown, and where each was) is written down
   (`state/zones/explorer.json`). Only if that was written are the icons
   put away. If putting them away fails, no zone is shown and
   `gevrek zones list` says why (`Not shown: ...`).
2. They are put back when zones are switched off
   (`desktop.zones.enabled: false`) and when the shell ends in the
   ordinary way.
3. If the shell was killed, they are put back when it next starts.

**The ways back**, any of them, as often as you like:

- `gevrek zones restore-explorer`. It works with zones switched off too.
- Switch the zones off.
- In the desktop's own menu: View, "Show desktop icons". It is the same
  switch.
- The uninstaller puts them back.

`desktop.platform.windows.hide_desktop_icons: false` shows zones without
touching Explorer at all. Its icons then lie under the zones and cannot
be pressed, and every icon is seen twice; it is for trying zones, not
for living with them.

Windows' own virtual desktops are not gevrek's workspaces: the zones
follow gevrek's.

## Linux

- The items are those of the XDG Desktop folder (`user-dirs.dirs`),
  watched for changes. There is nothing of the system's to put away, and
  `restore-explorer` does nothing.
- Launchers are `.desktop` entries, shown and started by the Desktop
  Entry rules (one that is not trusted is not started). Icons come from
  the icon theme, thumbnails from the shared thumbnail cache, "Move to
  the bin" is the XDG Trash.
- The item's menu is gevrek's own (Open, Open with from the MIME
  associations, Rename, Move to the bin): there is no shell to ask.
- The zones are layer-shell surfaces of gevrek's own compositor: below
  the windows, above them while peeking. There is no X11 path.
- A drag from an X11 program reaches the zones only as far as the
  compositor bridges it.
- Privacy is automatic: the compositor knows when the screen is shared.

## If something does not react

When a press, a drag or a double press does nothing on your desktop,
the zones can write down what they heard, so that it can be seen where
it was lost. Start the shell with the variable `GEVREK_ZONES_TRACE=1`
set (in PowerShell: `$env:GEVREK_ZONES_TRACE = "1"`, then start gevrek
from that window; on Linux: `GEVREK_ZONES_TRACE=1` before the command
that starts the session), do the one thing that does not react, once or
twice, and stop.

The trace is in the zones' own folder, begun anew with each start:

- Windows: `%LOCALAPPDATA%\gevrek\state\zones\trace.log`
- Linux: `~/.local/state/gevrek/zones/trace.log`

(and in the shell's log at the level of information, `RUST_LOG=info`).
Every line begins with the time since the zones started. It has:

- every button that went down: which, where on the desktop, the
  system's own time of the press, whether it was a first press or **the
  second of a double press**, and what a double press is on this system
  (the longest time between the two, and how far apart they may be);
- for every press, release and menu press: **what was hit** (`Edge` is
  the rim a zone is sized by, `Head` its title, `Button` a button of the
  title, `Body` the zone beside its items, `Item`, `Desk` the bare
  desktop) and **what was decided** (the commands, or nothing), and what
  the pointer is in the middle of afterwards (`Pressed`, `Moving`,
  `Sizing`, `Carrying`, `Banding`, `Nothing`);
- every button let go, every time the pointer was told gone from the
  surface, and every time the surface got or lost the keyboard (became
  the active window, or stopped being it).

How to read it. A press that is not in the file never reached the
surface (another window took it). `hit Some(Body(..))` or `Desk` where
you meant the rim: the press was beside the rim (it is 8 points inside
and 4 outside the edge; the pointer has arrows where it would take
hold). Two presses that are both "a first press": they were farther
apart in time or place than the system's double press, and the numbers
are on the line. "has lost the keyboard" between a press and its
release: another window was made the active one in the middle of the
press. A line of movement alone is not written (a drag would fill the
file), so the trace costs nothing to leave on for a trial; it is off
unless the variable is set.

## Before you rely on it

As of 2026-11-20:

- **The first days on a real desktop** (Windows 11, two displays)
  brought six things, and what was made of them (menus that open beside
  their lines, the buttons of a title, the pointer's shape and a wider
  rim for sizing, the middle button twice for grey icons, zones by
  type) has again been verified **by tests and pictures only**. Hiding
  and showing by two presses on the bare desktop worked there and is as
  it was. One thing did not react there although its tests passed:
  sizing a zone by its rim. No fault was found in how a press on the
  rim is decided; the rim was six points wide with nothing to show
  where it is, and it now is wider, drawn, and shown by the pointer.
  Whether that was all is what the trace above is for.

- **Milestone 2** (zones that are drawn, filled by hand, moved and
  sized, with Explorer's icons put away and brought back) is the part
  that was meant to be tried on a real desktop first; `milestone-2.md`
  says in which order, beginning with Explorer's icons alone.
- **Everything past it** (rolling up, tabs, quick hide and peek, locks,
  looks, portals, rules with a dry run, smart zones, arrangements for
  sets of displays, snapshots, the keyboard, search, privacy, tiling
  awareness, what a screen reader is told) has been verified **by tests
  and by pictures only, never on a real desktop**. The tests are many
  and run on Windows and on Linux (in WSL): the core against a backend
  in memory, the backends against real folders in a scratch place, the
  shell against headless surfaces, the command line against a real
  server with a scripted shell. No zone of these milestones was ever
  opened on a screen with a pointer; no drag was made with a mouse, no
  screen was shared, no screen reader was attached, no network folder
  was shown.
- **Zones are on by default** all the same, as of 2026-11-20: that was
  decided with this in view. Switching them off is one line
  (`desktop.zones.enabled: false`) or one switch in the studio.
- So: where losing an afternoon would cost something, switch them off; keep
  `gevrek zones restore-explorer` at hand on Windows, save a snapshot
  before trying the rules (`gevrek zones snapshot save before`), and try
  a rule that moves files with `gevrek zones rules dry-run` first.
  Nothing here deletes a file outright, and every file operation can be
  taken back, but that too was only tested.

## What is not there yet

What the notes of the three milestones (`milestone-3.md`,
`milestone-4.md`, `milestone-5.md`) list as not done, checked against
the tree on 2026-11-20:

- **A menu's line that does something closes every menu**, also a line
  that only marks something (a look, a lock): to mark a second one, the
  menu is opened again.
- **What the middle button does twice, and the types of "zones by
  type",** are set in the configuration file; the studio has no fields
  of its own for `gestures` and `auto.categories`.
- **On Linux the menus beside their lines, the pointer's shapes and the
  key for menus were compiled and tested without a screen**, as
  everything of the zones on Linux was. A compositor that knows no
  cursor shapes by name (`wp_cursor_shape_v1`) shows its usual pointer
  everywhere.
- **What a button of a title does is said only under the pointer**:
  reached by Tab, a button says it to a screen reader, not on the
  screen.
- **An arrangement's file is typed**, by its path; there is no file
  chooser, and a file that is there already is written over.
  (`gevrek zones snapshot export` writes over one as well.)
- **Pinned programs cannot be put in order by dragging** within a zone
  of the pinned ones.
- **The search line is on the primary display only** while the zones
  are above the windows.
- **A shell that starts while the screen is already shared hears
  nothing** until sharing next changes: nothing is covered until then.
- **On Windows the zones' own lines for an item need Shift** with the
  other button; the plain one is the system's menu, which has no place
  for them. Properties, "new folder" and "open with" are the system's
  there; gevrek's own sheets for them are for Linux.
- A zone that is open for now shuts after its moment while its menu is
  still open; the pointer's shape does not change over a title (it does
  over a rim and over a title's buttons); the line of progress does not
  move where the system cannot say how far a copy is.
- **Dropping from an X11 program** onto a zone on Linux was not tried.
