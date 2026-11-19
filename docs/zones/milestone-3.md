# Zones: milestone 3, the shell's part

Written 2026-11-19. Roll-up, tabs made and undone by dragging, hiding
everything at once, looking at the zones above the windows, locks that
say so, the look of a zone, and the little that moves.

What decides is in the core (`core-milestones-3-5.md`), what the surfaces
can do is in shellkit (`milestone-3-surfaces.md`); this page is what the
shell draws and hears: `crates/shell/src/zones/`.

**Nobody has seen it run.** Everything here was built against the
backend in memory, tried in tests, and drawn into pictures that were
looked at. No surface was opened by a real host for any of it.

## What exists

| File | What changed |
|---|---|
| `zones/motion.rs` (new) | The three things that move by themselves (a zone rolling open or shut, everything fading, the zones coming above the windows) and the short shake of a zone that refused; each a number on its way, asked for at a given time. |
| `zones/gesture.rs` | Which items are chosen and what has the keyboard is the core's: a press says so with a command. New: a zone let go on another's title (`Tab`), a tab dragged along its row (`MoveTab`) and off it (`SplitTab`), a click on a rolled-up title (`Expand`), `Act::Refuse` for a zone that is held. The keys for zones, tabs and rolling up. |
| `zones/view.rs` | The layout carries what the scene now says: open for now, locks (the zone's or the one for all), covered, the filter, what stays when the rest is put away (`DeskBox::stays`), each item's name and whether it is chosen. A zone that is open for now lies above the others. |
| `zones/draw.rs` | The drawing of all of it; `Tone` (icons greyed and tinted); the line that is typed into while the zones are looked at. `draw` gives back the grounds behind which the desktop is to be soft. |
| `zones/mod.rs` | The core is told what the pointer is over (`PointerOver`) and the time when it asks (`Tick`, by `wake_at_ms`); items are opened through the core (`Open`, started on `OpenRequested`); what is typed goes to the search, a zone's filter, or a name's beginning; the timed rules' schedule. |
| `zones/surfaces.rs` | `stands`: the surfaces go above the windows with the keyboard while the zones are looked at. The soft parts are told to the host when they change. The look's menu; more lines in the zone's, the desktop's and an item's menu. |

Nothing outside `crates/shell/src/zones/` was changed for this milestone
(`main.rs` and `windows.rs` are as they were).

## How each thing is done, by the pointer and by a command

Everything the pointer does is a `gevrek_zones::Command`, so a key bound
to the same command, or `gevrek zones send`, does the same.

| What | By the pointer | The command |
|---|---|---|
| Roll up, roll down | Double click the title; the zone's menu | `roll_up` |
| Open a rolled-up zone for now | Rest on its title (150 ms by default); where zones open at a press (`roll_up.expand_on: click`), click the title; carrying something over the title opens it either way | `pointer_over` + `tick`; `expand` |
| Shut it again | Leave it (400 ms by default); coming back in time keeps it open | `pointer_over` + `tick` |
| Make a zone a tab of another | Drag it by its title onto the other's title or tabs: the other gets an accent rim and its title says "Let go: “Code” becomes a tab here"; the dragged zone is seen through meanwhile. Alt held: it is only moved | `tab` |
| Bring a tab to the front | Press it | `activate_tab` |
| Reorder tabs | Drag a tab along its row | `move_tab` |
| Split a tab off | Drag it more than 14 units above or below its row: it is a zone of its own and goes on with the pointer | `split_tab`, then `move_zone`; the menu's "Out of its tabs" is `untab` |
| Hide everything, bring it back | Double click the bare desktop; the desktop's menu | `quick_hide` |
| A zone stays when the rest is hidden | The zone's menu | `always_visible` |
| An item stays | The item's menu (Linux: ours; Windows: the other button **with Shift** gives the zones' own lines, since the plain one is the system's menu) | `item_always_visible` |
| Look at the zones above the windows | (a key: Super+Z by default) | `peek`, `toggle_peek` |
| Stop looking | Escape (first empties what was typed), the key again, opening something | `dismiss`, `peek`, `open` |
| Lock a zone, or all | The zone's menu; the desktop's menu ("Every zone held where it is", "Nothing in or out of any zone") | `lock`, `lock_all` |
| The look of a zone, or of all | The zone's menu, "Look…"; the desktop's menu, "Look of all zones…" | `modify_zone`, `modify_settings` |
| Move and size with snapping | As before; Alt: nothing pulls | `move_zone`, `resize_zone` |

### The keys, while a zones surface has the keyboard

| Key | What |
|---|---|
| Arrows, Home, End (Shift: choosing as well) | From item to item in the zone that has the keyboard, or among the loose items of the bare desktop (`move_item_focus`; the core walks both) |
| Alt+arrows, Alt+H/J/K/L | The keyboard to the zone that lies that way (`focus_neighbour`) |
| Alt+Shift+arrows, Alt+Shift+H/J/K/L | What is chosen into the zone that lies that way (`move_focused_items`) |
| Enter | Open (`open_focused`, or `open` for chosen loose items) |
| Escape | Let go of a drag; else `dismiss` |
| Letters, Backspace | Looked at above the windows: looked for everywhere. In a zone with the keyboard: its filter (`filter`). On the bare desktop: to the first name that begins so |
| Ctrl+Tab, Ctrl+Shift+Tab | The next tab, the one before (`next_tab` with `@focused`) |
| Ctrl+R | Roll the zone up or down (`roll_up` with `@focused`) |
| Ctrl+A, Delete, F2 | As before |

The window manager's own Super+arrows never reach a surface, so the keys
between zones are with Alt. Bindings that work from anywhere are the
window manager's to give (`cli-345`'s examples); these are the keys of a
surface that already has the keyboard.

### The keyboard

Unchanged: a display's surface has the keyboard only when it is pressed.
One exception, and it is a command: while the zones are looked at
(`core.peeking()`), `surfaces::sync` puts every display's surface at
`Level::Overlay` with `Keyboard::Exclusive`, and back to `Level::Below`
with `Keyboard::OnDemand` when that ends. The test
`the_keyboard_comes_with_a_press_and_is_never_asked_for` holds that this
is the only place, and that it goes by what the core says of peeking and
nothing else.

## What is drawn

- **One meaning for the accent colour**: where what you do goes (chosen
  items, the zone with the keyboard, where a drop would land, the tab in
  front, the lines a moved zone meets).
- **What a zone is besides, at the end of its title**, always in this
  order from the right: an angle (rolled up; pointing up while open),
  how many it holds (`3 of 8` under a filter), a padlock (nothing in or
  out), a pin (held where it is), a ringed dot (stays when the rest is
  hidden), a struck-through eye (private). The filter stands after the
  name as a small chip with a lens.
- **A zone that refuses** (held where it is and dragged or sized; lets
  nothing in and something is let go on it) shakes its head for 0.4 s
  and its lock is marked in the warning colour. With movement reduced it
  is marked and does not shake.
- **Title bar**: `full` (mark, title, hairline), `plain` (the title
  alone, quieter), `hidden` (nothing; three dots under the pointer show
  where to take hold; rolled up and shut, the title is shown all the
  same). The geometry is the same in all three.
- **Look**: ground colour, opacity, corner radius, title colour, names
  under the icons or not, icons greyed (`icon_saturation`) and tinted
  (`icon_tint`: baked into the texture when it is made, and the same for
  the figures we draw ourselves). The menu offers the theme's own hues
  laid thinly over its ground colour, so a coloured zone still belongs
  to the desktop it is on.
- **Soft behind** (`look.blur`): the zone's rectangle and radius are
  given to the host (`Control::set_blur_regions`), again only when one
  changed. Where the host shows it the ground is lighter (52 in a
  hundred); where it cannot (`blur_available()` false) more solid (84),
  so words are read either way. With high contrast: no blur, an opaque
  ground, a strong rim, and thicker focus rings.
- **What lies above other things** (a zone open for now, one being
  moved) is nearly solid and has a shade under it.
- **Hidden at once**: what goes fades out and what stays or comes fades
  in (0.18 s). Items that stay of zones that are put away are drawn
  bare where they were, with the ringed dot.
- **Looked at**: the windows are dimmed; a line to type into in the
  upper part of the primary display, and under it up to 8 of what was
  found, each with the zone it is in; items that do not fit are dimmed
  where they lie.
- **Snapping**: dashed lines where a moved or sized zone meets the free
  room's sides, the display's, other zones and the gap beside them; with
  `snap.grid`, a small cross at each corner that rests on the grid.

### What moves, and when nothing does

Three things, each a fraction of a second: rolling open or shut (0.16 s;
while it rolls shut the zone as it was is drawn, cut short), fading
(0.18 s), coming above the windows (0.14 s). With
`desktop.animations.reduce_motion` every one of them is at its end at
once (`Motion::still`).

A frame is asked for only while a drag is under way or something is on
its way (`Motion::busy`). The core's waits (open after a moment, shut
after a moment) are no polling: after every command the zones read
`wake_at_ms()` and ask for one wake-up at that time. At rest nothing is
drawn and nothing wakes.

## Pictures

    cargo test -p gevrek-shell the_zones_are_drawn_to_be_looked_at -- --ignored
    cargo test -p gevrek-shell milestone_three_is_drawn -- --ignored

write into `target/zones-pictures/` (1280 by 800 unless said):

| File | What |
|---|---|
| `zones-1x.png`, `zones-1.5x.png`, `zones-high-contrast.png` | The working desktop, as in milestone 2 |
| `m3-rolled-up-open.png` | "For later" open for now, over the zone below it |
| `m3-tab-drop.png` | "Code" dragged onto the title of "Pictures" |
| `m3-tab-split.png` | "Home" pulled off its row, with the lines it meets |
| `m3-hidden.png` | Everything put away; a zone, an item of another zone and a loose item stay |
| `m3-peek-search.png`, `m3-peek-search-high-contrast.png` | Looked at, with "no" typed and the second hit chosen |
| `m3-looks.png` (soft shown), `m3-looks-1.5x.png` (1920 by 1200), `m3-looks-high-contrast.png` | A filter, the keyboard on a zone and an item, a lock, plain and hidden title bars, no names, grey and tinted icons, a coloured ground, square and round corners |

All were looked at. Two things were changed after looking: a zone open
for now let the zone under it show through its ground (now nearly solid),
and a zone dragged onto another hid the very title it was aimed at (now
seen through, with the words above it).

## What was verified

- `cargo test -p gevrek-shell -p gevrek-zones` on Windows; `cargo test
  -p gevrek-shell` in WSL; `cargo clippy -p gevrek-shell --all-targets`
  with the two warnings it had before (`control.rs`, `tutoring.rs`);
  `cargo build`. The numbers are in the report of the day.
- New tests beside the code: `motion` (a part's way and its end,
  turning round on the way, reduced movement, a refusal); `gesture` (a
  zone onto another's title, with Alt, onto a held zone; a tab along its
  row and off it; a held zone says so, by title, side and by the lock
  for all; choosing through the core; a click on a rolled-up title;
  every key); `mod` (a rolled-up zone under the pointer: the wait, a
  pass, in and out, leaving the surface, something carried; refusals and
  opening through the core; a zone's filter typed and taken back; what
  stays when the rest is put away); `surfaces` (above the windows and
  back, with what is typed and found; the look's menu against the
  configuration it makes; the keyboard rule); `draw` (fading painted and
  over; nothing moving afterwards).

## What was not verified, and what is not done

- **Nothing on a screen.** In particular: that the host's level change
  and exclusive keyboard really bring the surfaces above the windows and
  back (shellkit's part is itself only compiled for the compositor's
  half); that blur regions are shown; how the roll and fade look at a
  real frame rate; whether a wake-up from another thread reaches the
  shell promptly enough for 150 ms to feel like 150 ms.
- A menu of a zone that is open for now takes the pointer off the
  zone's surface, so the zone shuts after its moment while its menu is
  still open.
- While the zones are looked at, the line that is typed into is on the
  primary display only.
- The item's own lines (stays, out of its zone) need Shift with the
  other button on Windows; the system's menu has no place for them.
- The look's menu closes after each choice.
- The pointer's shape does not change over a rim or a title.
- (Since milestone 4 a refusal that comes by a key or from outside
  shakes its zone too: the core says which zone it was about.)

## Manual test checklist

For both systems unless said. Start from a desktop with four or five
zones, one of them rolled up, two as tabs.

**Roll-up**
- [ ] Rest on a rolled-up title: it opens after a blink, over what is
      below it, with a shade. Pass over it quickly: nothing.
- [ ] Move from the title into the items and back; out over the rim and
      back within a moment: it never flickers shut.
- [ ] Leave it: it rolls shut after a moment, its items cut short on
      the way, not vanishing first.
- [ ] `roll_up.expand_on: click`: resting does nothing, a click on the
      title opens, another shuts; dragging the title still moves it.
- [ ] Drag an item over a rolled-up title and hold: it opens; let go
      inside: the item is in it.
- [ ] `desktop.animations.reduce_motion: true`: opens and shuts at once.
- [ ] Leave the desktop alone for a minute with a process monitor open:
      `gevrek-shell` uses no processor time.

**Tabs**
- [ ] Drag a zone by its title onto another's title: accent rim, the
      words, the dragged zone seen through. Let go: two tabs.
- [ ] The same with Alt held: only moved, no tab.
- [ ] Drag a tab along its row: the order follows. Up or down off the
      row: it comes off and follows the pointer.
- [ ] Lock one tab's place: no tab comes off, nothing is let in, and
      the zone shakes when tried.

**Hide and look**
- [ ] Double click the bare desktop: everything fades out; again: back.
      A slow double click that Explorer would take also counts (Windows).
- [ ] Mark a zone and an item (Shift+other button on Windows) as
      staying; hide: they stay where they were, the item bare with a
      ringed dot.
- [ ] Super+Z: the zones come above the open windows, dimmed behind;
      type a few letters: results under the line; arrows, Enter opens
      and the zones go back below; Escape once empties, twice ends.
- [ ] While looked at, the window that had the keyboard gets it back
      afterwards.
- [ ] Hidden, then Super+Z: everything shows while held, and is hidden
      again after.

**Lock and look**
- [ ] "Held where it is": dragging the title or a side shakes the zone
      and marks the pin; "Nothing in or out": dropping an item on it
      shakes it and marks the padlock, and the item stays where it was.
- [ ] The desktop's menu, "Every zone held where it is": every zone
      shows the pin.
- [ ] "Look…": each line changes the zone at once and survives a
      restart (it is in `config.json`); "Everything as the others" takes
      it all back. "Look of all zones…" changes those without their own.
- [ ] "The desktop soft behind it": Linux, the wallpaper is blurred
      behind that zone only, with round corners; Windows, note what is
      seen (never seen so far).
- [ ] `desktop.appearance.high_contrast`: opaque grounds, strong rims,
      no blur, thick focus rings.

**Snapping**
- [ ] Move a zone near the display's side, the panel's side, another
      zone: dashed lines where it rests; with `snap.grid: 16` and nothing
      else near, small crosses at its corners.
