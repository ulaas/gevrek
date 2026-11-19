# Zones: milestone 2

Written 2026-11-19. Zones that can be seen and used: backends for both
systems, the drawing and the pointer in the shell, drag and drop on the
shell's surfaces, and a way in for scripts and keys. Five notes say how
each part is made; this page says how they hang together and how far
anyone has seen them work.

**Nobody has seen it run.** Every part was built and tested without a
desktop: against backends in memory, headless surfaces, a server in the
test's own process. The first time zones are shown on a real desktop will
be the first time. The switch is off by default and the order under
"Trying it" is there for that first time.

## The parts

| Part | Where | Note |
|---|---|---|
| The core (milestone 1) | `crates/zones` | `milestone-1.md` |
| The backend on Windows: the shell namespace, icons, file operations, Explorer's journal | `crates/system/src/zones/windows/` | `milestone-2-windows.md` |
| The backend on Linux: the XDG Desktop folder, MIME, thumbnails, the trash, launchers | `crates/system/src/zones/linux*` | `milestone-2-linux.md` |
| Drag and drop for the shell's surfaces (OLE, `wl_data_device`) | `crates/shellkit/src/dnd.rs`, `win/dnd.rs`, `layer/dnd.rs` | `milestone-2-dnd.md` |
| The path from scripts, the command line and keys | `crates/zones/src/ipc.rs`, `crates/link/src/zones.rs`, `crates/server/src/zones.rs`, `gevrek zones` | `milestone-2-ipc.md` |
| The drawing, the pointer and keys, and what ties the rest in | `crates/shell/src/zones/` | `milestone-2-shell.md` |

## How they hang together

```
              config (desktop.zones, platform.windows.hide_desktop_icons)
                                   │  from the window manager, on every change
                                   ▼
 gevrek zones … ─► WM ─► link ─► shell: zones::surfaces::Running
 key bindings  ─┘   ▲      ▲            │
                    │      │            ├─ Zones (zones/mod.rs)
   snapshot, events ┘      │            │    ├─ ZonesCore + Host      (crates/zones)
   exec for launchers ─────┘            │    ├─ Store: state/zones/zones.json
                                        │    ├─ explorer: the system's own icons
                                        │    └─ Desk = SystemBackend  (crates/system)
                                        └─ one surface a display (shellkit), with
                                           drops taken and drags begun on it
```

- **One place decides.** Everything that changes the zones is a
  `gevrek_zones::Command`, whoever gives it: the pointer, a key, a menu,
  `gevrek zones send`, a key binding.
- **The displays are the window manager's.** The shell hands every state
  it has from it to the backend (`displays_of_state`), which gives each
  display the id made of its EDID. Arrangements are kept by those ids.
- **The configuration is written by the window manager.** What a command
  changes of `desktop.zones` goes there as the shell's other changes do,
  and comes back as the running configuration.
- **The state** (which item is where, where each zone is for each set of
  displays) is the shell's own file, written when it has been quiet for a
  moment.
- **On Linux a program is started by the window manager**, not by the
  shell (`exec` over the line), so it does not end with the shell.

## Explorer's icons (Windows)

Zones draw every item of the desktop, so Explorer's own icons are put
away while zones are shown. The rule is that a person's desktop is never
lost, and the order that keeps it is in one place
(`crates/shell/src/zones/explorer.rs`, tested against a desk in memory):

1. What an earlier run left away (it ended without putting back) is put
   back first.
2. How the desktop is, is written down (`state/zones/explorer.json`).
3. Only if that was written are the icons put away.
4. If putting away fails, what was begun is taken back and **no zone is
   shown**; `gevrek zones list` says why (`Not shown: ...`), and so does
   the shell's log.
5. They are put back when zones are switched off, when the shell ends in
   the ordinary way, and with `gevrek zones restore-explorer` (which also
   works with zones off, and as often as one likes). Asked back by hand
   while zones show, they stay back until zones are switched off and on.

`desktop.platform.windows.hide_desktop_icons` (on by default) switches
this off: zones are then shown and nothing of Explorer's is touched. Its
icons lie under the zones' surface then and cannot be pressed; it is for
trying zones, not for living with them.

If the shell is killed, the icons stay away until the shell next starts
with zones on, or until `gevrek zones restore-explorer`, or until the
person ticks "Show desktop icons" in the desktop's own menu, which is the
same switch.

## Trying it, deliberately

Nothing here was done by whoever wrote this. In this order:

1. **Explorer alone, by hand, first.** On a desktop that is yours:

       cargo run -p gevrek-system --example zones_desktop

   reads and changes nothing. Then

       cargo run -p gevrek-system --example zones_desktop -- --really-hide

   (or `cargo test -p gevrek-system -- --ignored really_hides`) writes
   the journal, puts the icons away and brings them back. Go through
   "Checklist: to be tried by hand on Windows" in
   `milestone-2-windows.md`, the crash line among them. **Do not go on
   if an icon is not where it was.**
2. **Zones without touching Explorer.** In the configuration:
   `desktop.platform.windows.hide_desktop_icons: false`, then
   `desktop.zones.enabled: true`. The shell opens a surface on every
   display. Look at: the loose items, a zone made from the desktop's
   menu, moving and sizing it, an item dragged into it, a double click.
   `gevrek zones list` should name what is seen.
3. **Zones as they are meant.** `hide_desktop_icons: true`. Explorer's
   icons go, ours stay. Then `desktop.zones.enabled: false`: Explorer's
   are back, each where it was.
4. **The ways back, each once:** `gevrek zones restore-explorer` while
   zones show; stopping gevrek; ending the shell from the task manager
   and starting it again.

On Linux there is nothing of the system's to put away: under `gevrek-wl`,
`desktop.zones.enabled: true` and the checklist in `milestone-2-linux.md`.

## What was verified

By tests and by building, on Windows and in WSL (Fedora). The commands
and their results are in the report of the day; in kind:

- the core: tables, scenarios on the backend in memory, long seeded runs
  (`milestone-1.md`);
- each backend against its system as far as that needs no hands: real
  folders in a scratch place, a made-up Explorer, real MIME and trash
  rules;
- drag and drop: the data formats there and back, the negotiation, both
  halves compiled; on Linux against a headless compositor;
- the path from outside: a real server and a real line in one process,
  the command line's parsing and printing;
- the shell: what lies at a point, every gesture against its command,
  the menus, the pacing of saves, drops onto each kind of place, the
  order of the steps around Explorer's icons, that nothing asks for the
  keyboard but a press, that a launcher is asked of the window manager;
  and pictures of a desktop with zones at two scales, looked at.

## What was not verified

- **Anything on a screen with a pointer.** No surface of the zones was
  ever opened by a real host, no drag was made with a mouse, no menu was
  shown, no item was opened.
- **Whether Explorer's icons go and come back on Windows 11** as the
  documentation says (`IFolderView2::SetCurrentFolderFlags`). This is the
  first thing to try, alone (step 1 above).
- What "Show desktop" (Win+D) does to the zones' surfaces, and whether a
  press that begins on one display and ends on another is told to the
  first surface, as the drawing assumes.
- The Linux side beyond WSL and headless: `gevrek-wl` on hardware has not
  run.

## Known gaps

- The first run on a real desktop is still to be made; expect things the
  tests could not show.
- With `hide_desktop_icons` off, Explorer's icons lie unreachable under
  the surface.
- Renaming is in place on Windows; on Linux the box is in the middle of
  the display.
- The Linux item menu has no "new folder", no properties, and no chooser
  for "open with" beyond the first six programs.
- A program's working directory from its desktop entry is not passed on
  when the window manager starts it.
- The studio has no page for `desktop.zones`, and the guide does not
  describe zones yet.
- For milestone 3: tabs made by dragging, blur, animation, roll-up on
  hover. For milestone 5: an accessibility tree for the items.
