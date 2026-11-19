# Zones: milestone 2, the shell's part

Written 2026-11-19. The component of `gevrek-shell` that draws the zones
and hears the pointer on them: `crates/shell/src/zones/`.

**Nothing of it was run on a live desktop.** It was built against the
backend in memory, drawn into pictures, and tried in tests. The real
backends of both systems are wired in since, and what they do together
with this on a real desktop nobody has seen yet.

## What exists

| File | What it is |
|---|---|
| `zones/mod.rs` | `Zones`: the core, its host, the backend and the state's file, together. Paces the saving, hands configuration changes to the shell, and holds the doors for what comes from elsewhere. |
| `zones/view.rs` | Where everything is, as rectangles (`Layout`): looked up for a press (`hit`), for a drop (`target`), for a selection rectangle. Label breaking and snap guides. No drawing, no pointer. |
| `zones/gesture.rs` | What the pointer and keys mean (`Gestures`): out come `Command`s for the core, and the few things that are no command (open, a menu, a drag out). |
| `zones/draw.rs` | The drawing of one display from the layout, the reading of the pointer on it, and the items' pictures (`Pictures`). |
| `zones/surfaces.rs` | One surface for each display, the menus, the box to type a name into, and `Running`, which is what each half of the shell holds. |

In `main.rs` (Linux) and `windows.rs` each: a field, a message
(`Message::Zones`) and two short functions (`zones_configured`,
`zones_woken`).

Added to `crates/zones` (nothing changed): `Host`'s functions take a
backend behind a `dyn` (`?Sized`); `Host::take_icons` hands out the
pictures the backend said are ready, which the core passes over;
`ZonesBackend::desktop_folder` (default: none); and three commands,
`TrashItems`, `RenameItem` and `BringIn`, each a file operation that
`UndoFileOperation` takes back.

In `crates/backend-win32/src/windows_key.rs`: `is_the_desktop`, so that a
press on a zones surface counts as a press on the desktop.

## Turning it on

`desktop.zones.enabled: true` in the configuration. The shell hears of
every change of the running configuration from the window manager; zones
start when the switch is first on, are told of every later change
(`Host::set_config`), and close their surfaces when it goes off.

**Know what the switch does before setting it.** With it on, the shell
asks the system's backend what is on the desktop and covers every display
with a surface of its own, below the windows. On Windows it first writes
down how Explorer's desktop is and puts Explorer's icons away (see "The
system's own icons" below), unless `desktop.platform.windows.
hide_desktop_icons` is off. None of this was ever done on a real desktop:
`milestone-2.md` says in which order to try it.

## How it works

- **State.** Read from `state/zones/` through `Store` at the start (what
  `load` reports goes to the log). Written when nothing changed for 1.2
  seconds, at the latest 10 seconds after the first change that is not
  written, and when the zones end. A drag changes the state with every
  movement and writes it once.
- **Configuration.** What a command changes of it (a zone made, renamed,
  rolled up, its view) is sent to the window manager as the shell sends
  any change of its own (`Request::Apply`, the whole configuration with
  `desktop.zones` replaced). It comes back as the running configuration
  and changes nothing then.
- **Surfaces.** One for each display: `Level::Below`, `Edge::Fill`,
  nothing drawn where nothing is. On Windows it is put at the display's
  rectangle in pixels; on Linux it is a layer surface on the display's
  output.
- **The whole surface is ours; presses do not go through.** The bare
  desktop between the zones is where loose items lie, where a selection
  rectangle is drawn, where the desktop's menu opens and a double click
  hides everything. On Windows the system's own icons will be put away
  while zones show (decision A1), so nothing under the surface is to be
  reached. Two things follow that are not done: gevrek's own "desktop
  pressed" (`windows_key.rs` looks for a press on Explorer's desktop
  window) no longer sees presses there, and until Explorer's icons are
  hidden they lie unreachable under the surface.
- **It has the keyboard only when it is pressed** (`Keyboard::OnDemand`),
  as a press on Explorer's desktop takes it. Never by itself: not when it
  is shown, when the pointer is over it, or when it is drawn again; a test
  holds that nothing in the drawing asks for it. With the keyboard: the
  arrows go from item to item, Enter opens, F2 renames the one selected
  file (`Command::RenameItem`), Delete puts the selection into the bin
  (`Command::TrashItems`; both taken back by `UndoFileOperation`),
  Ctrl+A selects, Escape lets go, and letters typed in quick succession
  go to the first item whose name begins so. The boxes for a name and for
  finding are surfaces of their own that have the keyboard while open.
- **A press on it is a press on the desktop to the window manager.** On
  Windows `gevrek_win32::windows_key` gave the pressed display the
  keyboard when Explorer's desktop was pressed, knowing it by its window
  class; the zones' surfaces (`Gevrek_Shell_gevrek-zones...`) count as the
  desktop there now (`is_the_desktop`), since they cover it.
- **Drawing only when something changed.** A frame is drawn when the
  pointer does something on the surface, when the core says something
  changed, or when a picture arrives. While a drag is under way the next
  frame is asked for; at rest nothing is.
- **Scale.** Everything is laid out in the desktop's logical units and
  drawn in the surface's own points; an item's picture is asked for at
  its size times the surface's scale (64 units at 1.5 is 96 pixels). If
  the surface's scale is not the one the backend says the display has,
  the log says so once.
- **Look.** From the design system: the palette's roles, its radius,
  density (denser measures), high contrast (an opaque ground and a strong
  rim). Per zone and for all: ground colour, opacity, corner radius,
  title colour, title only under the pointer. An item whose picture has
  not come, or that has none, is drawn as a figure of our own: a folder,
  a sheet with its extension on a band of colour, a sheet with an arrow
  for a shortcut. `blur` is not drawn: it is the surface's, not ours.

## The system's own icons

`zones/explorer.rs` holds the order of the steps (the rule and the order
are in `milestone-2.md`): `put_away` (put back what an earlier run left,
write down, hide; a failure anywhere leaves nothing away and gives
`SystemIcons::Refused(why)`), `put_back` (as often as asked),
`after_a_crash`. `Zones` calls them: when it starts or is switched on
with `hide_icons` set, when it is switched off, when the switch changes,
on `Request::RestoreSystemIcons`, and when it is dropped (the shell ends).
While the icons are `Refused`, `Zones::spots` gives no display, so no
surface is opened; the reason is an event (`ZonesEvent::Refused`) and
`ZonesSnapshot::not_shown`, which `gevrek zones list` prints.

Only `Desk::hide_system_icons` changes anything of Explorer's, and only
`explorer::put_away` calls it.

## What the pointer does

Everything that changes the zones is a `Command`:

| Gesture | What follows |
|---|---|
| Drag a title | `MoveZone` with each movement (Alt: nothing pulls); lines show what it meets |
| Drag a side or corner | `ResizeZone` |
| Double click a title | `RollUp` |
| Press a tab | `ActivateTab` |
| Press, Ctrl+press, Shift+press an item | selection (the shell's own; no command) |
| Drag on the bare desktop or in a zone | a rectangle that selects what it touches |
| Drag items onto a zone, a portal, the desktop | `AddItem` (at the cell or line under the pointer where the order is by hand); `PlaceLooseItem`, keeping how the items lay to each other |
| Double click an item | the backend's `open` |
| Other button on an item | the backend's `context_menu`, or a menu of ours from `Desk::own_menu` |
| Other button on a zone | rename, roll up, grid or list, the order, icon size, the locks, out of its tabs, take away |
| Other button on the desktop | new zone here, find an item, sort by the rules, take the sorting back, hide |
| Double click the bare desktop | `QuickHide` |
| The wheel over a zone | scrolls it (the shell's own; not remembered) |

## The doors for what comes from elsewhere

**The backend** (`zones/mod.rs`):

```rust
pub trait Desk: ZonesBackend {
    fn on_news(&mut self, _wake: Box<dyn Fn() + Send>) {}
    fn picture(&mut self, icon: IconHandle) -> Option<gevrek_system::icons::Picture>;
    fn spot_of(&mut self, display: &DisplayId) -> Option<Spot>;   // Spot { output: String, pixels: (i32, i32, i32, i32) }
    fn told(&mut self, _state: &serde_json::Value) {}             // the window manager's state: the displays
    fn desktop(&mut self) -> Option<String> { self.desktop_folder() }
    fn own_menu(&mut self, _items: &[ItemId]) -> Option<Vec<DeskAction>> { None }
    fn act(&mut self, _items: &[ItemId], _action: &str) -> Result<(), BackendError> { .. }
}
pub fn desk(wake: impl Fn() + Send + Sync + 'static) -> Option<Box<dyn Desk>>
```

`zones/backend.rs` is the one of this system: `gevrek_system::zones::
SystemBackend` in a struct that implements `Desk` for it.

- The displays are the window manager's: with every state the shell has
  from it, `displays_of_state` makes the backend's displays
  (`SystemBackend::set_displays`), and the workspace shown where the
  keyboard is goes to `Zones::workspace` (`surfaces::shown_workspace`).
- A display's surface is put at `SystemBackend::pixels_of` on Windows and
  on the output of the state's `connector` (else `name`) on Linux.
- On Linux what an item opens is started by the window manager, not by
  the shell: `SystemBackend::set_launcher` is given a launcher that sends
  `{"command": "exec", ...}` over the line (in the terminal for a program
  that wants one), so no program is the shell's child. What is started
  only over the session's bus is refused. A test holds that opening asks
  for the command and makes no process.
- On Linux the items' menu is ours, from `SystemBackend::menu_for`: open,
  open with (the first six programs), allow launching, rename, move to
  the bin. Rename and the bin go through the core's commands; a new
  folder and the properties are not drawn. On Windows the menu is the
  system's (`context_menu`).
- The desktop's folder is `ZonesBackend::desktop_folder` (new, with a
  default of none), and until a backend answers it, what the backend's
  own helpers say.

**Drags from and to other programs** are the surfaces' own
(`gevrek_shellkit::dnd`, see `milestone-2-dnd.md`), and are wired
(`surfaces::drags`, called from each display's drawing):

```rust
impl Zones {
    pub fn drag_over(&mut self, at: Option<Pos2>, payload: &DropPayload) -> DragActions;
    pub fn drop_at(&mut self, at: Pos2, payload: &DropPayload) -> Dropped;   // Placed | Filed { moved } | Refused
    pub fn payload_of(&self, paths: &[String], text: Option<String>, allowed: DragActions) -> DropPayload;
    pub fn take_drag_out(&mut self) -> Option<(Vec<String>, Option<Picture>)>;
    pub fn drag_ended(&mut self, remove: bool);
}
```

- Each display's surface takes drops (`Control::accept_drops`). Its
  standing answer, shown until a drawing has looked, is "a move": most of
  what is dragged over the desktop is the desktop's own. From the next
  frame the answer is what `drag_over` says for the place.
- What comes are paths. Those that are items of ours are those items
  again (`payload_of`); the others are files from elsewhere.
- What was agreed at the drop is carried out here in full (a move moves:
  the source deletes nothing), by `drop_at` with only that action allowed.
- Going out: on real surfaces (`Zones::system_drags`) a drag of files is
  the system's from its first movement. The surface begins it
  (`Control::start_drag`, with the first item's picture where it has
  come), the gesture here ends, and what is let go on a zone comes back
  as a drop, each item where it was held. The button's release the
  surface is told as the drag begins ends nothing: `DragEvent::Ended`
  does (`drag_ended`; with `remove`, the originals still there go to the
  bin).
- On Windows every drag of items goes out with the backend's own data
  object (`Desk::begin_drag`: `SystemBackend::start_drag`, `take_drag`,
  `Control::start_drag_of`), which names what is no file too (This PC,
  the bin) and carries our items in a format of our own; what is let go
  on a zone is read back from the system's object (`Desk::dropped`:
  `Control::dropped_object`, `dnd::payload_of`, the list of shell ids),
  and only the items that are ours count as ours. On Linux a drag goes
  out as the files' paths, and what is no file is carried by the shell
  itself, on its own surfaces.
- Before a frame of a surface is drawn on Windows, the backend is told
  that surface's window (`Control::native_window`, `set_owner`), so the
  system's menu and its progress windows belong to it.

**Commands from outside, and what is told back,** go over the line to the
window manager (`milestone-2-ipc.md`), and are wired in both halves:

- at the start `Link::host_zones()` and an empty snapshot;
- with every `Message::Line`, `Link::poll_zones()`; what was asked goes to
  `surfaces::asked(&state.zones, &state.link, ask)`, which gives a command
  to `Zones::command` and answers with `Answer::of(&effects)`, or puts the
  system's icons back (`Request::RestoreSystemIcons`, also with zones
  off); without zones running the answer is a refusal that says so;
- when the shell is woken for the zones, `Running::publish(&link)`: the
  events since the last time, then a snapshot (`ZonesSnapshot::of`). A
  gesture wakes the shell when the button comes up, so a drag is told at
  its end and not with every movement.

```rust
impl Zones {
    pub fn command(&mut self, command: Command) -> Vec<Effect>;
    pub fn take_heard(&mut self) -> Vec<ZonesEvent>;
    pub fn core(&self) -> &ZonesCore;
    pub fn restore_system_icons(&mut self) -> Result<(), BackendError>;
    pub fn told(&mut self, state: &serde_json::Value, workspace: Option<String>);
}
```

## Pictures

    cargo test -p gevrek-shell the_zones_are_drawn_to_be_looked_at -- --ignored

writes into `target/zones-pictures/`: `zones-1x.png` (1280 by 800),
`zones-1.5x.png` (1920 by 1200), `zones-high-contrast.png` (with a search
dimming what does not fit). A working desktop on the backend in memory:
a grid of papers with two selected, a list (a portal), pictures found by
a smart zone and scrolled, a zone rolled up, a locked zone, two zones as
tabs, loose items on the wallpaper. The pictures were looked at while the
drawing was worked on.

The test is not run unless asked for, as the shell's other pictures are
not: it needs a graphics device that computes on the processor, and where
there is none that works (WSL here) making one ends the whole test
program. What it holds without a picture (that everything is painted,
and that pictures are asked for in the surface's own pixels) is a test of
its own that always runs.

## What was verified

- `cargo build`; `cargo test -p gevrek-shell -p gevrek-zones` (the shell
  141 passed and 7 ignored on Windows, 173 and 8 in WSL; the zones 89
  passed on both); `cargo clippy -p
  gevrek-shell --all-targets` with the two warnings it had before
  (`control.rs`, `tutoring.rs`) and none new; the same lint and the same tests in
  WSL.
- Tests beside the code (33): what lies at a point, overlapping zones,
  tabs, the cell or line of a drop, scrolling, label breaking, snap
  guides; every gesture in the table above against the command it gives;
  the menus; the pacing of saves; the configuration handed over once and
  not echoed; drops from outside onto each kind of place; system drags;
  the scale pictures are asked at; zones switched off.

## What was not verified, and what is not done

- Nothing ran on a real desktop: no surface was opened, no pointer was
  read from a real window, no menu or box was shown. That the surfaces'
  hosts deliver what `draw::hear` reads (above all a press that began on
  one display and is let go on another) is assumed from how they are
  written, not seen.
- The backends, drag and drop and the line to the window manager are
  wired, and none was ever exercised with a real surface, a real desktop
  or a real window manager from here: `zones/backend.rs` is compiled on
  both systems and its one pure part is tested; the drop path is tried
  in tests up to `drop_at`, `payload_of` and `drag_ended`; `surfaces::
  drags` and `surfaces::asked` themselves need a host and are only
  compiled, on both systems.
- A file dragged in from another program onto a zone filled by hand or
  the bare desktop is copied or moved into the desktop's folder
  (`Command::BringIn`) and put where it was let go when the backend tells
  of it. That rests on the backend naming the new file's path in what
  takes the operation back, which the backend in memory does and the real
  ones are asked to.
- Renaming is in place on Windows (the box is put over the title); on
  Linux the box is in the middle of the display, because a layer surface
  is placed by the compositor.
- A tab cannot be made by dragging a zone onto another (`Command::Tab`
  exists; no gesture gives it), nor a portal or smart zone from the
  desktop's menu.
- No accessibility tree beyond what egui gives a painted surface, which
  is nothing for painted items (milestone 5). Tabs made by dragging,
  blur and animation are milestone 3.
- Nothing is animated, so `reduce_motion` has nothing to switch off.
