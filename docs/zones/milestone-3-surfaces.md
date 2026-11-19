# Zones: milestone 3, what the surfaces can do

Written 2026-11-19. The part of milestone 3 that is `crates/shellkit` and
`crates/backend-smithay`: a surface that changes its level while it is
shown (peek), the desktop soft behind parts of it, being left out of
pictures of the screen, room kept clear of tiling, and the four things
the compositor lacked for drags (`milestone-2-dnd.md`).

**Nothing of it was seen on a screen.** The Linux half ran against a
headless compositor with real clients; the Windows half was built, and
its tables tested. What Windows shows behind a zone has never been looked
at by anyone: see "Only compiled, never seen" and the checklist.

## The calls

All on `gevrek_shellkit::host::Control`, the same on both systems, queued
like its other calls and carried out when the drawing returns.

```rust
// Peek
pub fn set_level(&mut self, id: SurfaceId, level: Level)
pub fn set_keyboard(&mut self, id: SurfaceId, keyboard: Keyboard)

// Look
pub fn set_blur_regions(&mut self, id: SurfaceId, regions: &[BlurRegion])
pub fn blur_available(&self) -> bool
// gevrek_shellkit::look
pub struct BlurRegion { pub rect: egui::Rect, pub radius: f32 }   // BlurRegion::new(rect, radius)

// Privacy
pub fn exclude_from_capture(&mut self, id: SurfaceId, exclude: bool)   // Windows; nothing on Linux

// Room kept clear of tiling
impl Place { pub fn reserved_strip(name: &str, edge: Edge, depth: u32, output: Option<String>) -> Place }
```

How the zones would use them, in a display's drawing:

```rust
// Peek: the same surface, raised and put back. Calling with the level it
// has does nothing, so this can be said every frame from the state.
control.set_level(id, if peeking { Level::Overlay } else { Level::Below });
control.set_keyboard(id, if peeking { Keyboard::Exclusive } else { Keyboard::OnDemand });

// Look: where the grounds are, said again only when they change.
let grounds: Vec<BlurRegion> = layout.zones.iter()
    .filter(|zone| zone.look.blur)
    .map(|zone| BlurRegion::new(zone.rect, zone.look.radius))
    .collect();
if last_told != grounds { control.set_blur_regions(id, &grounds); last_told = grounds; }
// Nothing will be shown behind a ground: make it solid enough by itself.
let opacity = if control.blur_available() { zone.look.opacity } else { zone.look.opacity.max(0.85) };

// Privacy (Windows): the option of the configuration.
control.exclude_from_capture(id, config.privacy.hide_from_captures);

// Room (Linux): one more surface for each display that reserves, nothing on it.
let room = control.open(Place::reserved_strip("gevrek-zones-room", Edge::Right, 280, Some(output)), |_, _| Flow::Continue);
control.reach(room, Vec::new());          // the pointer goes through it
control.resize(room, 320, 0);             // another depth, later
```

What to know when using them:

- **`set_level` moves between `Background`, `Below`, `Above` and
  `Overlay`.** Not to or from `Marks`: a surface is marks from the start
  or never, and such a change is left undone and said in the log.
- **Place, size and picture stay** through a change of level. On Windows
  the surface keeps its rectangle; on Linux the compositor is told the
  new layer with a commit and nothing else.
- **The keyboard is its own call.** A surface raised for a look that
  should take keys asks for `Exclusive` and gives it back with `OnDemand`
  when it is put down; the keyboard then goes back to the window that had
  it (both systems). A surface that was given the keyboard by a press
  keeps it through a change of level.
- **Blur regions are in the surface's own points**, as egui's, and hold
  until others are said. An empty slice ends it. The surface has to draw
  something see-through there; what it draws solid hides it.
- **`blur_available`** is known from the start on Windows, and on Linux
  after the compositor's first answers (`false` in the very first frame;
  the surface is drawn again when the answer comes).
- **A press never goes to what shows the blur**: on both systems it is
  the surface's own.

## Peek: a surface's level while it is shown (A)

- **Windows** (`win/mod.rs`, `win/place.rs`): the level is a place below
  or above the windows (`place::order_of`), set with `SetWindowPos`
  (`HWND_BOTTOM`, `HWND_TOPMOST`; a window put at the bottom stops being
  topmost by that call). The window's procedure keeps a `Below` surface
  at the bottom whatever raises it; that is switched with the level. A
  bar (`Above`) still steps down for a window that fills the display
  (`ABN_FULLSCREENAPP`); an `Overlay` does not. `set_keyboard` changes
  `WS_EX_NOACTIVATE` on the open window and takes or gives back the
  keyboard as opening and closing do.
- **Linux** (`layer.rs`): `zwlr_layer_surface_v1.set_layer` and
  `set_keyboard_interactivity`, then a commit. `set_layer` exists from
  the layer shell's second version; with a compositor that has only the
  first the change is left undone and said in the log (gevrek's has the
  fourth).
- **The compositor** (`protocols.rs`): a layer surface's layer was read
  anew at every use already, so drawing and finding followed. Two things
  did not, and do now. *The pointer*: when a surface's layer changes,
  what is under the resting pointer is looked up again, so the surface
  raised under it has the pointer at once (and what it covered has it
  back when it is put down). *The keyboard*: a surface that held the
  keyboard because it asked for all of it above the windows, and no
  longer does, lets go of it, and so does one that takes no keyboard any
  more (`gives_up_the_keyboard`); before, it kept the keyboard until it
  was closed.

## The look: the desktop soft behind parts of a surface (B)

The zones' surface covers a display and draws several grounds, so what
is asked for is a list of parts, each a rectangle with a radius.

### Linux: `ext-background-effect-v1`

Chosen because it is the protocol for exactly this (a blur region of a
surface, set with the surface's state), it ships in the `wayland-protocols`
the workspace already has (0.32.13; the `staging` feature), and other
compositors are taking it up, so the shell's surfaces would be soft
there too. The compositor had no protocol for it before; its own blur was
a layer rule for a whole surface, which stays as it was.

- **The client** (`layer.rs`): binds the manager if it is there, hears
  what the compositor can do (`capabilities`: blur), and for a surface
  that asks makes one effect object and sets a `wl_region`. A region is
  rectangles of whole logical pixels: round corners are made of rows
  (`look::strips`, one logical pixel high in the corners, at most
  `2 * radius + 1` rectangles a part). It is taken up with the surface's
  next picture, which is asked for.
- **The compositor** (`background_effect.rs`, `render.rs`): the global,
  the region kept with the surface's double-buffered state, cut to the
  surface, at most 1024 rectangles. Behind each rectangle it draws the
  part of the display's own soft picture (`soft.rs`: the windows and the
  wallpaper behind, small and softened), as behind a surface with the
  `blur` rule. A surface with such a region is no part of that soft
  picture itself. Honoured for layer surfaces; a window that asks is not
  refused and gets nothing.
- **What it looks like, as far as can be said without seeing it**: the
  corners of the soft part are steps of one logical pixel under the
  ground's own smooth edge; the soft picture is a frame behind and of
  the compositor's softness (not the zone's to choose).

### Windows: one window under the surface for each part

What was looked at, all from the documentation:

| Route | What it gives | Taken |
|---|---|---|
| `DWMWA_SYSTEMBACKDROP_TYPE` on the surface's own window | the material behind the whole window: the whole display soft | no |
| `DwmEnableBlurBehindWindow` with a region | per region, but since Windows 8 it blurs nothing (it only marks the region see-through) | no |
| `SetWindowCompositionAttribute` (accent blur) | per window, real blur, **undocumented** | no |
| A window of its own under the surface for each part, with `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW` and `DWMWA_WINDOW_CORNER_PREFERENCE` | per part, the system's material and the system's round corners | **yes** |

So (`win/look.rs`): for each part a `WS_POPUP` tool window that draws
nothing (black, with the frame extended over all of it by
`DwmExtendFrameIntoClientArea`), never active, with the material of a
passing window and the system's corners. It lies directly under the
surface's window: its procedure puts it there whatever moves it, and the
surface's procedure puts its backdrops under it again whenever its own
place among the windows changes (so they follow a peek). They move with
the parts and with the surface, and are gone with it.

What follows from the choice, and is to be known:

- **The corners are the system's**: 8 pixels, 4, or none (at a scale of
  one), whichever is nearest the radius asked for (`look::corner_of`:
  below 1.5 points none, below 6 the small ones, else the large). A zone
  with a radius of 12 shows a soft part whose corners reach about a
  pixel and a half beyond its ground's.
- **Before Windows 11 22H2 there is no such material.** It is asked once
  when the shell starts; `blur_available()` is then `false`, no window is
  made, and the plain see-through ground is what is left.
- **With "transparency effects" switched off** in Windows' settings (or
  on battery saver) Windows shows the material as one plain colour.
  `blur_available()` still says `true` then.
- Each part is a window: a dozen zones are a dozen small windows more.
  They are named `Gevrek_ShellBackdrop_<surface>`, which the window
  manager leaves alone (the name begins with `Gevrek_Shell`) and which is
  not the zones' own class.

## Privacy: out of pictures of the screen (C)

- **Windows**: `exclude_from_capture(id, true)` is
  `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on the surface's
  window and on its backdrops; `false` takes it back. Windows 10 of 2020
  and later; before that such a window is black in pictures.
- **Linux, the compositor tells** (`sharing.rs`): a new event,
  `screen.shared` (`RuntimeEventKind::ScreenShared`, the one line added
  to `crates/engine/src/executor/mod.rs`). Fields: `state` (`started` or
  `stopped`), `output` (the display's name), `shared` (the names of all
  displays shared now, commas between; empty: none). It is told when a
  second picture of a display is taken within two and a half seconds of
  the first (one picture is a screenshot and is not told), and ends when
  none was taken for three seconds and no program waits for one (a
  recorder that asks only when something changes is recording while it
  waits). It counts every way a picture is asked for (`wlr-screencopy`,
  `ext-image-copy-capture`, so the portal too); pictures of one window
  do not count, they show no surface of a shell.

  **What the shell must do to use it** (not done here; `crates/shell` is
  not this part's): where `main.rs` reads the window manager's events
  (`Event::Runtime { kind, fields, .. }`, as for `"desktop.pressed"`),
  take `kind == "screen.shared"`, read `fields["shared"]`, and hand the
  list of display names to the zones (cover the surfaces of those
  displays while it is not empty). A shell that starts while a display
  is shared hears nothing until the next change: the state is not in the
  published snapshot.
- **Linux, a rule leaves a surface out** (`sharing.rs`, `render.rs`): a
  layer rule with `"exclude_from_capture": true` keeps the surfaces of
  that name out of every picture another program takes of a display, the
  single screenshot too; they are drawn on the display as ever. It was
  cheap: pictures for other programs are drawn by two functions of their
  own, which now say so while they collect what to draw, as they do for
  the pointer. The key is read from the rule's own unknown keys, which
  the configuration keeps as written (`crates/model` has no field for it
  yet and was not touched):

  ```json
  "layer_rules": [ { "namespace": "gevrek-zones*", "exclude_from_capture": true } ]
  ```

- **`Control::exclude_from_capture` does nothing on Linux**, and that is
  decided: a program has no way to ask a Wayland compositor for this,
  and a protocol of gevrek's own is not wanted for it. On Linux privacy
  is the shell covering what the zones show when it hears
  `screen.shared`; the rule above is there beside it and is not switched
  at run time.
- Not left out anywhere: gevrek's own picture of a frame (the request
  file a debugging run uses), which is the display as it was drawn.

## Drags: the four things the compositor lacked (D)

1. **What is topmost under the pointer decides** (`dnd.rs`,
   `x11_window_under_pointer`): both ways of a drag between X11 and
   Wayland ask what the pointer itself would find (`surface_under`), not
   which window lies there. A Wayland drag over a panel, a launcher or
   zones raised over the windows is no longer told to the X11 window
   underneath as well, and an X11 program's drag reaches such a surface.
2. **An X11 program's drag is what it asks for** (`xdnd.rs`,
   `xwayland.rs`): the action in each `XdndPosition` (and `XdndActionList`
   where the person is to be asked) is carried over. A copy is offered
   as a copy; a move as a move with a copy beside it; to ask, as what is
   listed. What the Wayland side settles on goes back in `XdndStatus`
   and `XdndFinished`, so the X11 program deletes what it gave only when
   told it was moved. Because the library's drag says what it allows
   once, a change of the action in the middle of a drag (Shift pressed
   in the X11 program) ends gevrek's own drag and begins it anew: the
   surface under the pointer sees the drag leave and come back.
   The other way (a Wayland program's drag onto an X11 window) is still
   always a copy.
3. **Ctrl and Shift** (`dnd.rs`, `State::action_choice`): Ctrl asks for
   a copy, Shift for a move, Ctrl with Shift or Alt for asking; taken
   where both programs allow it, else the target's wish as before
   (Mutter's rule). The library asks for the choice only when the target
   names what it takes, so a key pressed while the pointer rests makes
   the drag leave the surface and enter it again: a surface is told
   `Left` and `Entered` for it, and should not flicker over that.
4. **The picture's place** (`state.rs`, `render.rs`): what the program
   moved its picture by (`wl_surface.offset`, or the place given with a
   buffer) is added up and drawn; a picture held by its middle has its
   middle under the pointer.

## Room kept clear of tiling (E)

- **Linux**: a region of a display kept for zones is a layer surface's
  exclusive zone, and a surface that fills the display cannot have one.
  So: a second surface at the edge, as deep as the room, with nothing
  drawn (`Place::reserved_strip`), which lets the pointer through
  (`Control::reach(id, Vec::new())`). `Control::resize` on a surface that
  reserves now changes the room it keeps with its size (before, the room
  stayed what it was when the surface opened). The compositor needed
  nothing: it gives the windows what layer surfaces leave.
- **Windows: the window manager keeps the room** (done after the
  rest, the same day). The panel's room is not kept by the panel's
  window either: the window manager knows the panel and takes its depth
  off the display. The zones' room goes the same way, by another table:
  1. the zones' snapshot names it (`reserved`: display by its lasting
     id, `edge`, `thickness`); the server reads that out of every
     snapshot of the attached shell (`crates/server/src/zones.rs`,
     `room_of`, `ZonesHub::room`). A shell that has attached and said
     nothing yet, and one that is gone (its snapshot is then served as
     stale), keep no room;
  2. the server leaves it for the window manager's loop
     (`gevrek_engine::shell_room::tell` / `told_since`: the two are
     threads of one program with no line between them for this);
  3. the loop of `backend-win32` looks each pass, hands it to
     `Outputs::set_shell_room`, lays out again the displays whose room
     is another, and publishes the state;
  4. `Outputs::get_adjusted_work_area` takes it off after the panel's
     room (a region's depth is from the side of what the panel leaves),
     at each side the deepest of the regions there, as many pixels as
     the display's scale makes it. It is a table of its own beside
     `desktop.monitors[].reserved`, which is the person's and is not
     written to.
  Room on a display that is not there counts for nothing. The work area
  in the state (`work_position`, `work_size`, so `QueryState`) is what
  is left for the windows, zones' room taken off.
- **Not kept twice on Linux**: only the Windows loop looks at what the
  server left; the compositor never does, and keeps room by the strip
  surface's exclusive zone alone.
- **The state says which room is the zones' own**: each monitor of the
  state has `zones_room` (`left`, `top`, `right`, `bottom`, in the units
  of `work_position`; absent or all 0: none). The work area the state
  names is smaller by the zones' own room, and the zones' core must be
  told only the room the panel leaves, so the shell takes `zones_room`
  out of what it reads before it hands it on. It is what was taken off
  for that very state, not what the shell last asked for. Windows: what
  the fold took (`Outputs::shell_room_taken`, in pixels there). Linux:
  the deepest exclusive zone at each side of the layer surfaces named
  `gevrek-zones-room...` on that display (`State::zones_room`). New in
  `gevrek.proto`: `Sides`, and `zones_room` in `MonitorInfo` (16) and
  `MonitorOverlayData` (24).
- Not seen: no window was tiled against a zone on a real desktop.

## A double click as Windows has it

Not asked of a surface; every surface of the shell on Windows has it.
egui counts two presses as a double click only within 0.3 seconds and 6
points, whatever the system says; Windows' time is half a second unless
the person set another. A double press that Explorer takes did nothing
on a shell's surface (and a hidden desktop could not be brought back by
one). Now each surface's toolkit is given the system's values before
every frame (`win/input.rs`, `double_click`): the time from
`GetDoubleClickTime`, the distance from `SM_CXDOUBLECLK` and
`SM_CYDOUBLECLK` as points of the surface's display. They are read when
the shell starts and again at every `WM_SETTINGCHANGE`.

- The distance is never less than egui's own six points: egui uses the
  same number for how far a press may move before it is a drag, and
  Windows' rectangle (two pixels to each side by default) would make
  every unsteady click a drag. So the distance only grows, for a person
  who set a larger rectangle.
- The same number being the drag threshold means: with a large
  rectangle set in Windows, a drag on a surface begins that much later.
- Linux is as it was (egui's own values): a Wayland compositor says
  nothing of it, and the desktop settings portal has no key for it that
  all desktops share.

## Tests

- `gevrek-shellkit`, pure (both systems): parts as whole pixels at
  1, 1.25, 1.5 and 2, parts that touch, parts that are none, the radius
  never more than half a side, rows of a round part (no gaps, mirrored,
  the area a circle leaves); the strip's place.
  Windows only: a double click's time and distance from Windows'
  setting at several scales and with values nobody set;
  level to place among the windows with and without a
  window that fills the display, which changes of level are possible,
  who is never the active window, the backdrop's class, the system's
  corner for a radius, parts as windows on a second display at 1.5.
  Linux only: level to layer, the room a reserving surface keeps.
- `gevrek-engine`: the shell's room taken off the display it names by
  its lasting id and given back, the same told twice changing nothing,
  room on a display that is not there, inside the panel's room and in
  the display's pixels; the deepest at each side; what is told being
  news once. `gevrek-server`: the room read from a snapshot, and kept
  only while the shell that said it is attached.
- `gevrek-smithay`, pure: the keys' meaning, the choice with and without
  keys over every combination, XDND's actions to Wayland's and back, a
  region with rectangles taken away, its cap, parts cut to the surface,
  when sharing begins and ends, the rule.
- `gevrek-smithay`, `tests/shell_surfaces.rs`, real clients against the
  headless compositor: a surface goes over a window and back (the
  resting pointer and the keyboard follow, the window is active again);
  a surface given the keyboard by a press keeps it on another layer and
  loses it when it takes none; a drag onto a surface raised over
  everything, with Ctrl and Shift changing the action for source and
  target while the pointer rests, the picture's place, the drop; the
  blur region taken up at the commit, a piece of the soft picture drawn
  behind each part by the renderer in memory, gone with the object; a
  red surface that is in a `wlr-screencopy` picture and one of a ruled
  name that is not, and the display counted as shared after the second
  picture and no longer after a while.

Run on 2026-11-19:

- Windows: `cargo build -p gevrek-shellkit --all-targets`;
  `cargo test -p gevrek-shellkit` (73 passed);
  `cargo clippy -p gevrek-shellkit --all-targets` (nothing);
  `cargo check -p gevrek-shell -p gevrek-engine -p gevrek-win32 -p gevrek-server` (nothing).
- Linux (WSL, Fedora): `cargo test -p gevrek-shellkit -p gevrek-smithay`
  (shellkit 27; smithay 70 beside the code, 17 in `host`, 5 in
  `shell_surfaces`, 15 in `tiling`; all passed);
  `cargo clippy -p gevrek-shellkit -p gevrek-smithay --all-targets`
  (nothing); `cargo check -p gevrek-shell` (nothing).

## Only compiled, never seen

- **Everything Windows shows.** That a `WS_POPUP` window without a frame
  is given the material at all; that it stays the material while the
  window is not the active one (its procedure answers `WM_NCACTIVATE` as
  if it were, which is a guess at what is needed); that the backdrops
  stay directly under a surface at the bottom and under one raised
  topmost, without a flicker when the surface is pressed; how far they
  lag behind a ground that is dragged; what a second display at another
  scale does. If the material does not come on such a window, the
  honest state is "no blur on Windows": make `look::supported` say
  `false` and the plain ground is drawn.
- **`WDA_EXCLUDEFROMCAPTURE`** on a window that is a layer of the
  desktop's compositor (`WS_EX_NOREDIRECTIONBITMAP`), and on the
  backdrops.
- **Peek on Windows**: that a surface goes from the bottom to topmost
  and back cleanly, and what "Show desktop" (Win+D) does to either.
- **Linux, on a real display**: the blur's look (the stepped corners,
  the softness), on the graphics card's renderer (the tests draw in
  memory); the shell's own surface asking through SCTK (the tests' client
  speaks the protocol itself).
- **X11**: no test has an X server. The action carried over from an X11
  program, the restart of gevrek's drag when it changes, and what GTK and
  Qt programs under X11 make of the action told back to them are read
  from the protocol, not seen.
- **Other compositors**: `set_layer` and `ext-background-effect-v1`
  against anything but gevrek's own.

## Manual checklist

`cargo run -p gevrek-shellkit --example look` (a surface with three
grounds, below the windows in the middle of the display; put the windows
aside).

**Windows 11**

- [ ] Behind each ground the wallpaper is soft; between the grounds it
      is sharp. If the grounds are plain dark and the text says "can be
      shown here: no", this Windows has no material.
- [ ] If the text says "yes" and nothing is soft, or the grounds are one
      plain grey: note it. That is the case named above.
- [ ] The first ground's soft part has round corners, the second square
      ones, the third round ones a little smaller than its own.
- [ ] Press another window, then look again: still soft (not grey).
- [ ] "Soft: off": all three plain and more solid. On again: soft again.
- [ ] "Move the third ground": the soft part follows it; note how far
      behind it is.
- [ ] "Raise over every window": the surface and its soft parts are over
      all windows, soft over the windows now; Escape closes it (it has
      the keyboard). Run again, raise, "Put back": below the windows,
      and the window that had the keyboard has it again.
- [ ] Press the surface while it is below: it stays below, and no grey
      slab flashes over it.
- [ ] "Leave out of screenshots", then Win+Shift+S and a recording (Xbox
      Game Bar, OBS): the surface and its soft parts are missing, the
      wallpaper is there. "In screenshots again": back.
- [ ] Two displays at 100 % and 150 %: run it with the pointer on each;
      the soft parts lie exactly under the grounds on both.
- [ ] Win+D, and again: note what becomes of the surface and the parts.
- [ ] Double click, slowly (as slowly as Explorer still opens a file):
      on the zones' bare desktop it hides and shows them. Then Settings,
      Bluetooth & devices, Mouse, "Additional mouse settings", move
      "Double-click speed" to slow and to fast: the shell's surfaces
      follow without a restart, as Explorer does. A single click and a
      drag of an icon are as before.
- [ ] Settings, Personalisation, Colours, "Transparency effects" off:
      plain colour behind the grounds, nothing broken.

**Linux, gevrek's compositor**

- [ ] The same three grounds: soft behind each, sharp between. The
      corners of the soft part do not show as steps beside the ground's
      own edge.
- [ ] With a window half under the surface while it is raised: the
      window is soft behind the grounds.
- [ ] "Raise over every window" with the pointer resting on a ground:
      the buttons answer at once, without moving the pointer. "Put
      back": the window under the pointer has it again.
- [ ] While raised, type: nothing reaches the window underneath. After
      "Put back", typing goes to the window that had the keyboard.
- [ ] A layer rule `{ "namespace": "gevrek-look-example", "exclude_from_capture": true }`,
      then `grim` and `wf-recorder`: the surface is missing from both.
      Without the rule it is there.
- [ ] `gevrek log` (the window manager's events) while `wf-recorder` runs:
      `screen.shared` `started` with the display's name, and `stopped`
      a few seconds after it ends. One `grim`: no event.
- [ ] `control.open(Place::reserved_strip(...))` from the zones: tiled
      windows keep clear of the strip; `resize` moves them.

**Linux, drags**

- [ ] Drag a file from a Wayland file manager over the zones while they
      are raised over an X11 window: only the zones show the drop mark;
      the X11 window underneath does not react.
- [ ] Drag from an X11 file manager (`GDK_BACKEND=x11 thunar`) onto the
      raised zones: `entered`, `dropped`.
- [ ] The same with Shift held in the X11 program: `Dropped { action:
      Move }`, and the file is gone from where it was only if the zones
      moved it. Without Shift: a copy, and it stays.
- [ ] Between two Wayland programs: hold Ctrl while the pointer rests
      over the target: the pointer's mark changes to "copy" at once;
      Shift: "move"; let go: the target's own.
- [ ] Over a zone, press and release Ctrl: the place a drop would land
      at does not flicker.
- [ ] The picture of what is dragged from the shell has its middle under
      the pointer; a file manager's picture is where it is in other
      compositors.
