# Using gevrek

Running it, the command line, the configuration, the look, gestures, the
settings that only one system has, and installing a build. For what gevrek
is and how to build it, see the [README](../README.md); for the Linux
session, [LINUX.md](LINUX.md).

## Run

**Linux.** As a 1920x1080 window inside the current Wayland or X11 session,
with the bar, the editor, the list of shortcuts and a terminal in it:

```sh
scripts/run-nested.sh
```

or directly, also on the hardware from a text console:

```sh
target/release/gevrek-wl -- foot
```

The `gevrek` commands below are the same on Linux (`target/release/gevrek`
in place of `target\release\gevrek.exe`). A few act on what only Windows has;
`gevrek capabilities` lists what the running backend offers.

**Windows.** From the repository:

```powershell
target\release\gevrek.exe start all
target\release\gevrek.exe status
target\release\gevrek.exe health
target\release\gevrek.exe state
target\release\gevrek.exe focus-last
target\release\gevrek.exe focus-window class:WindowsTerminal
target\release\gevrek.exe float-window class:Dialog
target\release\gevrek.exe center-window tag:scratch
target\release\gevrek.exe toggle-fullscreen-window tag:presentation
target\release\gevrek.exe toggle-pseudotile-window class:WindowsTerminal
target\release\gevrek.exe tag-toggle-window tag:communication urgent
target\release\gevrek.exe move-window-to-and-follow tag:communication name:work
target\release\gevrek.exe move-window tag:scratch 120 80
target\release\gevrek.exe resize-window tag:scratch 900 600
target\release\gevrek.exe focus-monitor next
target\release\gevrek.exe move-workspace-to-monitor 1
target\release\gevrek.exe swap-active-workspaces name:Secondary
target\release\gevrek.exe move-to name:web
target\release\gevrek.exe move-to-and-follow name:web
target\release\gevrek.exe workspace-rename name:web research
target\release\gevrek.exe float
target\release\gevrek.exe center
target\release\gevrek.exe pin
target\release\gevrek.exe tag-clear
```

`start all` starts the window manager and, when it is beside
`gevrek.exe`, the shell (`gevrek-shell`: the panel and what belongs to
it). The settings program, gevrek studio, is not started with them: open
it when you want to change something, from the launcher's row of buttons
or with `gevrek start editor`. The shell can be started and stopped by its name
(`gevrek start shell`, `gevrek stop shell`); `status` lists it, and `stop
all` stops it with the rest.

Stop all components cleanly with:

```powershell
target\release\gevrek.exe stop all
```

`stop all` also discovers matching gevrek processes when a PID file is stale or
missing, applies bounded graceful-shutdown deadlines, and force-stops any
component that does not exit.

To have gevrek started when you sign in to Windows:

```powershell
target\release\gevrek.exe enable-autostart     # disable-autostart undoes it
target\release\gevrek.exe autostart-status
```

This makes a scheduled task (`\gevrek\autostart` in Task Scheduler, where
it can be seen and switched off) that runs `gevrek start all` at your
sign-in, as you and without raised rights. Where tasks are refused (a
policy can forbid them) a script in the Startup folder is written instead,
and the command says so. An autostart script from an earlier version is
replaced by the task.

The WM listens on a **local endpoint only your own programs can open**: a
Unix socket on Linux (`$XDG_RUNTIME_DIR/gevrek/main.sock`), a named pipe on
Windows. Every gevrek program finds it by itself. `--address` chooses
another: `local:<instance>` for a second gevrek on the same machine
(`GEVREK_INSTANCE=<instance>` does the same for every program started with
it), or `ip:port` for TCP, which any program of any user on the machine can
then connect to:

```powershell
target\release\gevrek.exe --address 127.0.0.1:57651 start all
```

The configuration is `config.json` in gevrek's configuration folder
(`~/.config/gevrek` on Linux, `%APPDATA%\gevrek` on Windows;
`gevrek config-path` prints the file). `--config <file>` on `gevrek-wm`,
`gevrek-wl` names another (`gevrek-studio --file <file>` edits one). Saving keeps the previous
file as a backup in gevrek's state folder. A file written by a newer gevrek
loads in an older one: what the older one does not know is kept and written
back, and reported as a warning.

Monitor actions accept `primary`, a zero-based index, `name:<display name>`,
`next`, or `previous`. Two displays of one model have a number each in
their names (`Monitor ACM2702 (2)`, as `gevrek state` shows them); the name
without the number means each of them. The same selectors and actions are available in
advanced bindings, gestures, and the editor Command Center.
`move-to` is the silent variant: it keeps the current workspace visible.
`move-to-and-follow` switches to the destination with the moved window.
`workspace-rename` gives an existing selected workspace another title: what
its tab, the overview and the settings show of it. The name it is known by to
keys and rules stays, so nothing that names it has to follow; a workspace is
found by either (`name:1` and `name:research` mean the same one after
`workspace-rename name:1 research`). From the command line the title lasts
while gevrek runs; given from a panel's tab or in gevrek-studio it is written
into the configuration, for that display alone or for every display.

Each display has workspaces of its own. A workspace written down for every
display (`"monitor": "Any"`) is one on each of them. A display can have an
entry of its own for the same `name`, with another `title`, another layout, or
`enabled` off (it does not have it); on that display its own entry counts. A
workspace that is opened while gevrek runs and is not written down takes the
number of its place, is laid out as the display's others are, and goes when
it is left empty.
`float`/`tile` and `pin`/`unpin` are idempotent alternatives to the toggle
commands. `center` promotes a tiled window to floating before centering it in
the monitor work area; `raise` and `lower` adjust its Windows Z-order.

## Configuration workflow

When no `config.json` exists, the WM starts with the built-in example Aurora
(see "Look" below) without creating a file: Dwindle tiling, workspaces 1 to 4
(5 to 9 come when they are used), the bar, and the standard keys below. Every
manageable window becomes managed by default: windows that already exist when
the WM starts are adopted into their monitor's active workspace (set
`desktop.manage_existing_on_startup: false` to leave them in place instead),
and newly created windows are tiled unless a rule floats or routes them.
The automation graph remains the customization
layer — a graph is not required for basic tiling.

When the focused window closes, the window used before it on the same
workspace gets the focus (`desktop.focus.focus_next_on_close`, on by
default). Left to itself, Windows hands the focus to the next window in its
own order, which with several displays is often one on another display.

Movements follow one of eight curves (`desktop.animations.<channel>.curve`):
six easings and two springs (`spring`, `spring_bouncy`). A spring that
interrupts a movement still under way goes on at the speed the window has at
that moment, so a window caught halfway swings round to its new place; the
easing curves start every movement from rest.
`desktop.animations.reduce_motion` leaves out everything that moves across
the screen (windows and workspaces are in their places at once); a border
changing colour and a window fading still do.

The editor's **Configure** page edits focus behavior, global appearance,
animation curves with a live preview and live scheduler status, monitor/workspace
profiles (including per-edge reserved areas, persistent master position and
new-window placement, and Dwindle split policy), typed global key
and mouse bindings, named special-workspace/scratchpad profiles, and ordered
multi-condition window rules. Snap zones include nine presets,
custom normalized geometry with a visual preview, per-monitor overrides,
overlap thresholds, activation modifiers, drag snapping, and direct/cycling
commands. A window is dropped into a snap zone only while the activation key
is held (Shift by default); a plain drag of a tiled window puts it back into the
tiling. They are set under `desktop.snap_zones`; a file written before schema
version 26 has them under `desktop.zones`, and they are moved when it is read.
`Win+F1` (the `shortcuts` command) opens a list of the current key
assignments; `gevrek start` writes each component's log next to its pid file
(`%LOCALAPPDATA%\gevrek\wm.log`). Scratchpads support safe structured launches,
title/class/executable/path matching, active/primary/indexed/named monitor
placement, percentage sizing, seven anchor positions, and optional
focus-loss hiding.
Schema-v7 advanced bindings add press, release, native repeat, configurable
long-press, mouse-button and wheel triggers, named modal submaps, and explicit
consume/pass-through behavior. The editor groups bindings by submap and can
record a keyboard or mouse input directly into a binding.
Schema-v12 gesture profiles map exact two- through five-finger directional
swipes to the same typed actions. The Configure page reports whether compatible
Precision Touchpad hardware is available, shows live recognition progress, and
edits finger count, direction, travel threshold, axis lock, idle timeout, and
action parameters. gevrek observes raw touch contacts without changing Windows
touchpad settings, so an existing Windows gesture assigned to the same motion
may also run.
Schema-v8 monitor and workspace policies reserve usable pixels for docks and
bars, persist master placement on any edge, and choose whether new master-layout
windows append, become master, or enter after the focused tile. The effective
usable area and policies are visible in the editor, CLI state, and overlay.
Schema-v13 workspace lifecycle policies distinguish persistent workspaces from
lazy profiles. Persistent and default workspaces exist at startup; other named
workspaces are created on first switch and removed after they become inactive
and empty. An optional structured **created empty** launch can start a program
the first time the user switches to a newly created workspace. Moving a window
to a new workspace does not run that launch. Workspace names and active
selection survive configuration reloads even when lazy profiles change ring
positions.
Rules can be static or dynamic, match workspace names/indices and normalized
window tags, and apply/revoke runtime state when a window title or state
changes. Rule effects include pixel/percent work-area positioning and sizing,
centering, workspace/state changes, opacity, tags, and native border, caption,
backdrop, and corner decoration overrides. The editor's Command Center and
Windows and Workspace inspectors consume authoritative all-workspace WM state,
including windows on inactive workspaces, active-workspace identity,
tiled/floating placement, effective tags, matched rules, opacity, dimming,
shadows, and live effects reported by IPC.
**Apply configuration** validates, persists, and hot-reloads the entire
versioned configuration atomically. The graph remains available as the
lower-level automation surface: canvas changes sync as a draft, and **Push**
applies that graph. **Save** and **Load** operate on the editor's `--config`
path.

The editor's **Examples** menu and the studio's Overview page ("Start from
an example") offer the same eighteen built-in, loadable starters. Four are
looks (Aurora, Focus Stage, Neon Grid, Paper); the others cover Dwindle,
master-stack, multi-monitor routing and workspace movement,
dynamic geometry and
decorations, tag workflows, a terminal scratchpad, presentation and gaming
modes, media, accessibility, automation graphs, and a minimal monocle setup.
The Snap Zone Workbench demonstrates direct and cycling placement with an
editable priority-grid layout. Advanced Binding Lab demonstrates every input
trigger, mouse bindings, pass-through behavior, a modal resize submap, and
three- and four-finger workspace/overview gestures.
Loading a starter creates an editable working copy; it does not modify the
built-in template or change the running desktop. Use **Load and run** on an
example card to validate, save, and apply its complete configuration
immediately. After using **Load editable copy**, use **Run this starter** on
the Home page (or **Apply configuration**) when the draft is ready. Running
examples requires the WM connection indicator to be online.

## Look: effects, the bar, the examples

**Drawn effects** (`desktop.appearance.effects`, Configure page, "Global
appearance", "Drawn effects"). gevrek draws them itself around and over every
bordered window, so they look the same on every program:

| Effect | What it is | Settings |
|---|---|---|
| Shadow | a soft shadow under every window | size, offset, colour, strength for the focused and for the other windows |
| Glow | a light around the window in the border's colour (or its own) | size, colour, strength for the focused and for the other windows |
| Gradient border | the border runs from its colour into a second one; around the focused window it can turn | the second colours, direction, seconds for one turn (0: still) |
| Tint | a veil of colour over the windows that are not focused | colour, strength for the other windows and for the focused one |

All are off by default and need "Draw gevrek borders" on (a border of
opacity 0 gives the effects without a visible border). They are drawn on
Windows and on Linux.
A turning gradient costs a few percent of one processor core while it turns;
the rest costs nothing while nothing changes.

**The panel** (`desktop.shell.panel`, the studio's "Panel" page; drawn by
`gevrek-shell` on Linux and on Windows, by the same code). One strip on
every monitor. What stands on it is the configuration's: three lists,
`left`, `centre` and `right`, of `launcher`, `workspaces`, `layout`,
`title`, `clock`, `stats`, `notifications`, `power` and `spacer`. A file
written for `desktop.bar` is read as the panel. A click on a workspace
switches to it; a click on the layout opens the menu of layouts. A click
on the panel never takes the keyboard away from the window in use. The
mouse wheel over the panel goes through the workspaces, a notch for each.
What the window manager has to tell (its notices, `gevrek health`) is a
yellow mark on the panel for as long as there is something; a click lists
it. On Linux the icons of programs that run in the background stand
there too (a transitional tray: a press is passed to the program, the
other button opens its menu; it is not a module yet and has no place in
the configuration, see [docs/LINUX.md](LINUX.md)).

On Windows:

- The window manager keeps the panel's room free of the windows it lays
  out, from the configuration, and says where the panel is (top or bottom,
  with or without room around it, inside the edges you reserved); the
  shell puts the panel there, on every display the window manager uses.
  With no panel on the screen the room goes back to the windows. Maximised
  windows and windows gevrek does not manage lie under the panel, and it
  stays above a fullscreen window: it is not registered with Windows as an
  application bar.
- The panel follows the window manager by its events and is drawn when
  what it shows changes; left alone it uses no processor time to speak of.
- The keys' mode is shown while it is not the usual one.
- `launcher` is a button that opens the launcher (below).
- `notifications` is a bell that opens what gevrek has told (its own
  notifications, not the system's; below), and `power` a button that opens
  the power menu (below).

**Examples.** The editor's Home page lists complete configurations to start
from. Four of them show the look:

| Example | Look |
|---|---|
| Aurora | a gradient border that slowly turns, glow, soft shadows, windows and workspaces that fade; Dwindle |
| Focus Stage | the windows that are not focused are veiled and stand back; Centered master; bar at the bottom |
| Neon Grid | pink-to-yellow border, strong glow, sharp corners, quick animations; Grid |
| Paper | hairline borders, large soft shadows, a light bar; Columns |

Each has the standard keys below and a small automation graph to read and
change. "Load and run" asks before
it replaces the running configuration; the windows that are open go to the
example's first workspace.
Without the editor: `gevrek example` lists them, `gevrek example aurora --apply`
makes one the running configuration, `--out file.json` writes it to a file.

**The standard keys.** The starting configuration and these four examples
share one set of keys. It follows the default keys of the niri compositor,
with Super (the Windows key) held:

| Keys | What happens |
|---|---|
| Super+Shift+/ or Super+F1 | the list of shortcuts |
| Super+O (or Ctrl+Alt+E) | overview |
| Super+Q | close the window |
| Super+T | a terminal |
| Super+arrows or H J K L | focus the window in that direction |
| with Ctrl | move the window in that direction |
| with Shift | focus the monitor in that direction |
| with Shift+Ctrl | move the window to that monitor |
| Super+PageDown / U, Super+PageUp / I, Super+mouse wheel | next / previous workspace |
| with Ctrl | move the window there and follow it |
| Super+1..9, Super+Ctrl+1..9 | workspace by number; move the window there and follow it |
| Super+Alt+1..9 | send the window to that workspace and stay where you are |
| Super+Shift+wheel, Super+Ctrl+Shift+wheel | focus / move the window left and right |
| Super+[ ] , . W | tabs: join the neighbour, leave the tabs, tabs on or off |
| Super+F, Super+Shift+F, Super+M | one window fills the area; fullscreen; maximize |
| Super+C, Super+V | centre the window; floating or tiled |
| Super+- =, Super+Shift+- = | narrower / wider; lower / taller |
| Super+D or Space, Super+S, Super+N | launcher, control centre, notifications |
| Super+Shift+D | show the desktop, and again the windows |
| Super+Alt+L, Super+Shift+P | lock the screen, displays off (on Windows: the system's own lock screen, and the system's command for the displays) |
| Print, Ctrl+Print, Alt+Print | screenshot of a region, the screen, the window |
| the volume, media and brightness keys | what they say |
| Super+Shift+O | an empty workspace |
| Super+Shift+E | quit the session |
| Super+Escape | take the keys back from a program that asked for all of them (a virtual machine, a nested gevrek), or hand them over again |

**The overview of the windows.** Its key (`toggle_expose`) shows the windows
of the workspace small, side by side, each in its own shape and all made
smaller by the same measure, with their titles under them. Press one to go
to it; the arrows and Enter do the same, Escape or a press beside them puts
the overview away. The windows themselves stay as they are: what is shown
are live pictures of them, so no program is resized. It is the same on Linux, where
the compositor draws the pictures itself. How large a
window is shown follows from the room the overview has, not from the window:
one window alone is still shown smaller than it is.

**Showing the desktop** (Super+Shift+D, `gevrek show-desktop`, the command
`show_desktop`). As Win+D on Windows: the windows of the workspace every
display shows go out of sight, and the desktop, with its zones, is seen and
has the keyboard. The same key again brings them back, each where it was and
as it was (a maximized window maximized, the tabs as they were), and the
window that had the keyboard has it again. The windows are not minimized,
only put away as on a change of workspace, so nothing about them changes.
`gevrek show-desktop on` only shows the desktop, `off` only brings the
windows back.

It also ends by itself, with the windows back, as soon as you work with a
window again: when you show another workspace (the windows of the other
displays come back, those of the workspace you leave go with it), when a
window gets the keyboard (a press on its button in the panel, Alt+Tab, a key
that focuses a window, a window brought back from the taskbar), when a new
window opens on a workspace that is shown, and when a display comes or goes.
A new window brings the others back with it, where Windows would show it
alone: here it is laid out among them, and alone it would stand in a part of
an empty-looking workspace. A window that closes meanwhile is forgotten; one
moved to another workspace meanwhile stays out of sight with that workspace.
A scratchpad you call up over the desktop stays as you leave it. While the
desktop is shown, `gevrek state` says so (`desktop_shown` in the state a
client asks for), and the events
`desktop.shown` and `desktop.hidden` (with the `reason` it ended) are told.

**Hot corners** (`desktop.hot_corners`; in the studio under *Mouse and
gestures*). The pointer, put into a corner of a display and left there for a
moment (`delay_ms`), runs that corner's command. Every display has its
corners. Unless the configuration says otherwise there is one: the top left
corner shows the overview of the windows. A corner runs once, and again
after the pointer has left it; a pointer that lies in a corner when gevrek
starts does nothing until it has been moved out of it; nothing runs while a mouse button is held or
over a program that fills the display.

The launcher, panels, lock, screenshots, sound, brightness and media are the
shell's to carry out: a binding announces what is wanted (`gevrek log --json`
shows `shell.command`, and `gevrek shell <action>` announces the same from
the command line), and `gevrek-shell` answers. Without it running nothing
does. On Windows it answers the launcher, the notification centre and "do
not disturb", the power menu, the lock (the system's own lock screen),
"displays off" and the volume and microphone keys so far; the rest (screenshots, brightness,
media, the control centre, the night light) is not there yet. `gevrek
notify` and `gevrek shell osd` (below) go the same way on both systems.

**The launcher** (`gevrek-shell`; Super+D or Super+Space, `gevrek shell
launcher`, or the `launcher` button of the panel). A field in the middle of
the screen with the keyboard: type a few letters, the arrows move the choice,
Enter carries it out, Escape puts the launcher away, and so does asking for
it a second time. It lists the installed applications with their icons, the
best answer first and among equals the one started most often; open windows
whose title answers what is typed, to switch to; and after a `>` everything
gevrek can be told to do that needs nothing said with it.

- On Linux the applications are the desktop entries, started by the window
  manager in the session's environment; after a `:` the launcher lists what
  was copied lately, to put it back on the clipboard.
- On Windows they are what the Start menu lists: the shortcuts under
  `Start Menu\Programs` of the user and of all users, and the packaged
  applications (from the Store). Uninstallers, shortcuts to a manual, a web
  page or a folder, shortcuts whose program is gone, and a second shortcut
  to the same program are left out. A program's own name finds it too
  (`winword`). What is chosen is started as the Start menu starts it: the
  shell is handed the shortcut (or the packaged application's place in
  `shell:AppsFolder`), so the program goes on when gevrek-shell stops, and
  a shortcut's "run as administrator" or working folder counts. It has the
  rights gevrek-shell runs with. Windows lets no window keep the keyboard:
  a click on another window takes it, and the launcher then closes, as a
  menu does. Nothing is listed after a `:` yet.

**Notifications on Windows** are gevrek's own: what gevrek itself has to
tell. gevrek does not take the place of the system's notification centre;
what other programs send stays there. Two things arrive:

- what the window manager has to tell (its notices, such as another program
  on the same keys), once when it arises; its yellow mark stays on the panel
  for as long as it holds;
- what a command sends: `gevrek notify "Backup done" "Three files were
  copied."` (the text may be left out). Useful at the end of a script.

A notification is a card at the top right of the display with the keyboard,
under the panel. It never takes the keyboard, goes by itself after six
seconds (a notice after twelve), and at once on a click. The `notifications`
module of the panel is a bell: it opens what arrived, the newest first, with
a button that forgets all of it and the switch for "do not disturb" (kept,
not shown); Escape or a click elsewhere puts the list away. `gevrek shell
notification_center` and `gevrek shell do_not_disturb` do the same from a
key. Where the cards are and how many are shown is not a setting yet (top
right, five), and what arrived is forgotten when the shell stops.

**The on-screen display on Windows** is a small surface near the bottom of
the display that shows a value for a moment. It shows how loud the sound is
whenever that changes, whoever changed it: the volume keys are the system's
on Windows, and Windows shows its own display for them as well; with
`"osd": {"volume": false}` in `desktop.shell` gevrek leaves it to that one.
A key bound to `volume_up`, `volume_down`, `volume_mute` or `mic_mute`
changes the default device itself, by five in a hundred. `gevrek shell osd
"Backup" 40` shows words with a level from 0 to 100, `gevrek shell osd
"Done"` words alone. The layout and the workspace are not shown on a change
yet (nor are they on Linux); brightness and the media keys are not there on
Windows.

**The power menu on Windows** (`gevrek shell power_menu`, or the `power`
module of the panel): lock, sign out, sleep, restart, shut down, in the
middle of the display, with the keyboard (L, O, S, R, P; Escape closes).
They are the system's own calls, so signing out ends the Windows session,
not only gevrek. Sign out, restart and shut down want a second press;
programs with unsaved work are asked first, as when the Start menu is used.
`gevrek shell lock` locks at once.

**Locking, the displays and the idle times.** `gevrek lock` locks the
screen and `gevrek displays-off` switches the displays off until the next
key or movement, on both systems; they are the shell actions `lock` and
`displays_off` (Super+Alt+L and Super+Shift+P) by names of their own, and
like those need `gevrek-shell` running on Windows. On Linux gevrek is the
session: its own lock screen locks, and the screen locks and the displays
switch off by themselves after `desktop.session.lock_after_minutes` and
`displays_off_after_minutes`. On Windows all of it is the system's: the
lock is Windows' lock screen, the displays are switched off by Windows'
own command for it (a moment after it is asked for, so that letting the
keys go does not switch them on again), and when the screen goes dark and
the machine sleeps by itself is decided by the power plan in use
(Settings, System, Power), which gevrek reads and never changes. `gevrek
health` reports those times, marked as the system's ("Displays off: after
10 minutes on mains, after 3 minutes on battery", "Sleep: never", and
whether a password is asked for on waking where Windows says), and the
Session page of the settings program shows them above the two settings. A
configuration that sets `lock_after_minutes` or
`displays_off_after_minutes` is not kept on Windows, and the window
manager says so in a notice (`gevrek health`, the panel's mark, a
notification of the shell) instead of letting it be believed. With
`GEVREK_POWER_DRY_RUN` set in the shell's environment nothing of this, nor
anything the power menu offers, is carried out: it is logged as what it
would have been (the desktop checks run so).

**The list of shortcuts on Windows** (`Win+F1`, `gevrek shortcuts`) is a
surface of the shell where the shell runs: no window to tile or to keep
floating. Typing searches it; "super" and "win" both find the chords with
the Win key. Without the shell, the same key opens the settings program's
window with the same list, as before.

Super+T starts a terminal (Windows Terminal on Windows, kitty elsewhere). Any
other program can be put on a key with the binding command "Start a program"
(`{"command": "exec", "program": "firefox", "args": ["--new-window"]}`), or
started from the command line with `gevrek exec firefox --new-window`.

**The editor's graph.** A wire is made by dragging from one pin to another
(or by clicking one and then the other). "Tidy" puts the nodes in order, from
left to right the way things happen; "Fit" shows the whole graph. A node
shows what it is set to in its header (the keys of a key node, the text of a
text node).

## Zones on the desktop (not snap zones)

Two things have "zones" in their name, and they are not the same:

- **Snap zones** are places on a display that a *window* snaps into:
  `desktop.snap_zones`, `gevrek snap-zone 0`, `snap-zone-next`, the
  Snap Zone Workbench example. They are described under
  [Configuration workflow](#configuration-workflow) above.
- **Zones** are boxes on the desktop that hold its *icons*: zones filled
  by hand, portals that show a folder, smart zones that fill themselves,
  rules that sort, saved layouts, one arrangement for each set of
  displays. They are `desktop.zones` (off until `enabled` is set), drawn
  by the shell, and driven from the command line with `gevrek zones`:

  ```sh
  gevrek zones list
  gevrek zones create --title Work
  gevrek zones add work report.pdf
  gevrek zones rules dry-run
  gevrek zones events --json
  ```

  **Super+Z** brings them above the windows, and again puts them back;
  **Super+Shift+Z** hides and shows them. On Windows, Explorer's own desktop icons are put
  away while zones are shown and come back when they are switched off;
  `gevrek zones restore-explorer` brings them back at any time.

Everything about them (each feature by pointer, key, configuration and
command line, the rules, the events for status bars, what happens to
Explorer's icons) is in [zones/user-guide.md](zones/user-guide.md).

## Touchpad gestures

Gestures work with a Precision Touchpad on Windows and with any touchpad
libinput reports on Linux. Gesture settings live under `desktop.gestures`. This fragment maps a
three-finger swipe left to the next workspace:

```json
{
  "schema_version": 22,
  "gestures": {
    "enabled": true,
    "idle_timeout_ms": 350,
    "axis_lock_ratio": 1.25,
    "profiles": [
      {
        "name": "Next workspace",
        "enabled": true,
        "fingers": 3,
        "direction": "left",
        "distance_percent": 18.0,
        "command": {
          "command": "switch_workspace_selector",
          "selector": { "kind": "next" }
        }
      }
    ]
  }
}
```

The editor is the easiest way to add gestures because its action picker builds
the typed command object and validates ambiguous finger/direction pairs before
Apply. Hardware and recognition state are also available from `gevrek state`
and the expanded overlay.

## Settings for one system only

What only one backend can apply lives under `desktop.platform`, so one
configuration file works on every system:

```json
{
  "platform": {
    "windows": { "backdrop": "Mica", "corners": "Round", "shadow": "default" },
    "wayland": { "xwayland": true, "cursor_theme": "", "cursor_size": 0 }
  }
}
```

`platform.windows` holds what the Windows compositor does for windows
(backdrop, corner rounding, frame shadow); `platform.wayland` whether X11
programs run through Xwayland and the cursor theme programs are told to use.
Each backend ignores the other's section. Files written before schema version
21 had the three Windows settings under `desktop.appearance`; they are read
as they are and written in the new layout the next time they are saved.

A window rule can match the **Application id**: what a Wayland program calls
itself (`org.mozilla.firefox`), and on Windows the program name without
`.exe` (`firefox`), so a rule on it can serve both systems.

Window rules can opt floating applications into **Persistent size**. gevrek
then remembers their last positive floating dimensions by executable and
initial class/title, restores the dimensions on the next launch, and preserves
the cache across graceful WM restarts. The Dynamic Geometry & Decorations
starter demonstrates this together with matching, floating, centering, and
native decoration effects. **Minimum size** and **Maximum size** can use pixels
or percentages of the monitor work area and are enforced while a floating
window is resized. **Keep aspect ratio** follows the user's dominant resize
axis while honoring those bounds.

The same rule editor exposes three distinct focus policies: **No initial
focus** prevents launch-time focus stealing, **Never focus** rejects every
focus attempt, and **Ignore follow-mouse** exempts a window only from
focus-follows-mouse. **Stay focused** takes precedence when a rule explicitly
combines contradictory policies.

The CLI can push either a complete `Config` JSON file or a raw `NodeGraph`:

```powershell
target\release\gevrek.exe push config.json
```

Manual Trigger nodes can be fired from the editor. Common runtime actions are
also available directly:

```powershell
target\release\gevrek.exe focus-next
target\release\gevrek.exe focus-previous
target\release\gevrek.exe focus-last
target\release\gevrek.exe focus-window "title:Visual Studio Code"
target\release\gevrek.exe focus-window tag:communication
target\release\gevrek.exe focus-window "class:re:^(Chrome_WidgetWin|MozillaWindowClass)"
target\release\gevrek.exe focus-window "initialtitle:re:^Project .* - Editor$"
target\release\gevrek.exe move-window tag:scratch 40 0 --relative
target\release\gevrek.exe resize-window tag:scratch 80 40 --relative
target\release\gevrek.exe pin-window tag:overlay
target\release\gevrek.exe toggle-fullscreen-window pid:4242
target\release\gevrek.exe toggle-pseudotile-window class:WindowsTerminal
target\release\gevrek.exe tag-add-window class:Dialog urgent
target\release\gevrek.exe tag-clear-window tag:temporary
target\release\gevrek.exe move-window-to tag:communication name:chat
target\release\gevrek.exe close-window title:Temporary
target\release\gevrek.exe move-to-next
target\release\gevrek.exe workspace 0
target\release\gevrek.exe workspace name:code
target\release\gevrek.exe workspace back
target\release\gevrek.exe move-to +1
target\release\gevrek.exe group-toggle
target\release\gevrek.exe group-next
target\release\gevrek.exe group-toggle-window tag:communication
target\release\gevrek.exe group-next-window tag:communication
target\release\gevrek.exe group-focus-window class:Browser 2
target\release\gevrek.exe group-lock-window tag:communication
target\release\gevrek.exe group-move-window-forward pid:4242
target\release\gevrek.exe layout-set dwindle
target\release\gevrek.exe layout-set floating
target\release\gevrek.exe layout-set scrolling
target\release\gevrek.exe layout-ratio 0.55
target\release\gevrek.exe layout-promote
target\release\gevrek.exe dwindle-direction horizontal
target\release\gevrek.exe dwindle-cycle-split
target\release\gevrek.exe scratchpad terminal
target\release\gevrek.exe snap-zone 0
target\release\gevrek.exe snap-zone-next
target\release\gevrek.exe shortcuts
target\release\gevrek.exe submap resize
target\release\gevrek.exe submap-reset
target\release\gevrek.exe dwindle-preserve-split
target\release\gevrek.exe dwindle-pseudotile
target\release\gevrek.exe toggle-float
target\release\gevrek.exe tag-add communication
target\release\gevrek.exe tag-toggle urgent
target\release\gevrek.exe rule-disable "Float picture-in-picture"
target\release\gevrek.exe rule-toggle "Float picture-in-picture"
target\release\gevrek.exe capabilities
target\release\gevrek.exe config-pull live-config.json
target\release\gevrek.exe config-validate live-config.json
target\release\gevrek.exe rule-list
target\release\gevrek.exe rule-upsert browser-rule.json
target\release\gevrek.exe rule-remove "Browser"
target\release\gevrek.exe log --json
```

Several typed actions can run as one uninterrupted WM batch. The file uses
the same tagged command objects as bindings:

```json
[
  { "command": "switch_workspace_selector", "selector": { "kind": "name", "value": "code" } },
  { "command": "set_layout", "layout": "Dwindle" },
  { "command": "set_layout_ratio", "ratio": 0.58 }
]
```

```powershell
target\release\gevrek.exe batch workspace-setup.json --stop-on-error
```

## External extensions

External tools can discover the versioned `extension_events_v1` capability,
then subscribe directly over gRPC or consume newline-delimited JSON from:

```powershell
target\release\gevrek.exe log --json
```

Each line is a complete event object suitable for scripts and long-running
satellite tools:

```json
{"event":"window.moved","fields":{"height":"900","width":"1440","x":"120","y":"80"},"timestamp_ms":42,"window_handle":99}
```

The stable event vocabulary covers runtime lifecycle, configuration reloads,
binding triggers, dispatch success or failure, and window lifecycle, focus,
movement, sizing, dragging, minimization, restoration, and metadata changes.
Extensions can respond through the same typed CLI, `Dispatch`, or
prevalidated `DispatchBatch` command surface used by the editor.

## Installing a build

```sh
cargo xtask install            # release build to ~/.local/bin
cargo xtask install --prefix /usr/local
```

A session runs the installed programs, never the build in `target/` that is
being worked on. The build installed before is kept in
`<prefix>/lib/gevrek/previous`.

## Releases and updates

```sh
cargo xtask dist               # what a release offers, under target/dist
```

On Windows that is `gevrek-windows-x64.zip` (the programs with
`install.ps1`) and, where Inno Setup 6 is installed, the setup program
`gevrek-windows-x64-setup.exe`. On Linux it is `gevrek-linux-x64.tar.gz`,
and the `.deb` and the `.rpm` where dpkg and rpmbuild are there.
`cargo xtask dist deb` makes the named ones only, and fails if it cannot.

A release is made by GitHub (`.github/workflows/release.yml`) when a tag
`v<version>` is pushed, where `<version>` is the one in `Cargo.toml`: it
builds these files on Windows, Ubuntu and Fedora, tries each package, and
publishes them with `SHA256SUMS.txt`.

A gevrek that was installed on Windows from the zip or the setup program
updates itself from those releases (`crates/update`):

```powershell
gevrek update                  # get the newest release and install it now
gevrek update --check          # only say whether there is one
gevrek update --automatic off  # do not look by itself (on: look again)
```

Unless turned off, `gevrek start` leaves a look for a newer release running
behind itself, at most once a day. It asks GitHub for the latest release
(with nothing but `gevrek/<version>` as its User-Agent), downloads the zip
of a newer one, checks it against the release's `SHA256SUMS.txt`, and
unpacks it in the state folder. Nothing that runs is touched. The next
`gevrek start` that finds nothing of gevrek running puts the new programs in
place first, moves the old ones to the folder `previous` beside them, and
starts the new build; a notification says so. If a step fails, the old
programs are put back and gevrek starts as it was.

A build made from the sources (`cargo xtask install`) is not replaced:
there `gevrek update` only says what the newest release is. The same on
Linux, where the package manager updates.
