# gevrek roadmap

What is open, in the order to do it, and what comes after. Rewritten on
2026-11-17 at version 0.6.0. The roadmap before this one (419 items done,
with their dates and the log of every session) is kept beside it on the
machine it was written on, as `ROADMAP-archive-2026-11.md`.

This file is in the repository, so that the work can go on from either
system. The archive is not: git leaves it out through
`.git/info/exclude`.

## How to use it

- An item is `- [ ]` until it is done **and checked**: a test or a check
  that runs, or for the items marked *person* the person saying so. Then
  `- [x]` with the date and the commit: `- [x] ... (2026-11-20, a1b2c3d)`.
- An item that turns out larger is split in place. Do not tick a half.
- Something found that is not on the list goes under **Found on the way**
  and moves into a section when it is decided.
- Every session ends with a line in the **Log**: the date, the system,
  what was done, what state the tree was left in.
- Each item says where it can be done: **W** Windows, **L** Linux,
  **any** either, **person** needs you (a judgement, real hardware, a
  week of use).
- Sections 1 to 4 are agreed work. Section 5 is proposals: nothing there
  is decided until it is moved up.

## Where things stand (2026-11-25)

- Version 0.7.6. Releases have installers for Windows and Linux and
  updates on Windows; macOS is built from source.
- The window manager, the shell and the studio work on Windows, on
  Linux and (new, from 2026-11-22) on macOS; the lock screen and the
  display manager on Linux.
- The pass of 2026-11-25 went through every crate for faults (what it
  found and fixed, and what it found and left, is under "Found on the
  way"), brought clippy to no warnings on Windows and Linux, and made
  the tree as `cargo fmt` writes it. CI checks the formatting, runs
  clippy on Linux too, and type-checks for macOS.
- The tutorial's tour is played from its first step to its last by a
  test, against the real core on a desktop in memory
  (`tests/tutorial_played.rs`). It found three faults a person would
  have met (section 3). Its chapters are not written, and nothing in
  the tour promises them any more.
- What is left waits for the person at a desktop: every item marked
  **person** in sections 2, 2d and 4, and the list of 2026-11-25 in
  section 2.
- The check scripts (`tests/desktop`, `tests/nested`, `tests/vm`,
  `tests/wayland`) are not in this repository: the public one leaves
  them out. On this machine only the crates' own tests can be run.

Before a session, on the system you are on:

| | Windows | Linux |
|---|---|---|
| Everything quick | `cargo xtask check` | `cargo xtask check` |
| The other system, type-checked | `pwsh scripts/check-linux.ps1` | `scripts/check-windows.sh` |

(On the Windows machine the Linux side builds and its tests run in WSL,
as the distribution's own user and not as root, through one script:

    scripts/wsl-cargo.sh test --workspace --exclude gevrek-win32

It uses WSL's default distribution (`GEVREK_WSL_DISTRO` names another),
keeps the build on the Linux side, and has everything drawn on the
processor: WSL's own graphics (its Direct3D driver, and OpenGL) end the
studio's picture test with a fault otherwise. What cannot be had there
is the desktop itself. With `GEVREK_BLESS=1` set inside, the pictures
are drawn anew.)

---

## 1. The ground under the work

Do these first. Until they are done, a fault can come in unseen.

- [x] **W** CI, the Windows job: the test of `conflicts` that failed
      on the runner and passed here. Found (2026-11-17): the runner is
      an administrator, and `notices` says so, rightly; the test asked
      for no notice at all. It asks now for none of FancyZones
      (`nothing_is_said_of_fancyzones_when_the_zones_are_not_on_shift`),
      and a second test holds what is said of the machine against what
      is so there. The Windows job on `91859eb` is green: every test,
      and clippy after them (2026-11-17, run 44074103140).
- [ ] **L** CI, the Linux job: the studio's
      `every_page_looks_as_its_picture` fails with every page 3 to 10 %
      off its picture. Found (2026-11-17): the studio writes in the
      system's own font unless told otherwise (`fc-match sans-serif`),
      so every line of every page was another on another machine; the
      pictures agreed only with the machine they were drawn on. The
      test writes in gevrek's own letters now, and the pictures were
      drawn anew so (2026-11-17, in WSL, by the Vulkan driver that
      computes on the processor; looked at; the test passes there three
      times running). Not committed yet: on `91859eb` the job still
      fails at this test, and at nothing before it. To tick: the job on
      a push with the new pictures, and the test on the person's Linux. Should a machine's driver still draw a page
      otherwise, the next step is the driver named in the test itself.
- [ ] **any** CI green once, to the end. Windows is (`91859eb`); the
      nested checks on Linux have never been reached.
- [x] **any** The check of the dependencies (cargo-deny) failed since
      2026-11-17, and passed the day before with the same
      `Cargo.lock`: RUSTSEC-2026-0543 calls `chrono` unmaintained, in
      every version, and `deny.toml` lets a crate nobody maintains fail
      where gevrek uses it directly. Decided by the person
      (2026-11-18): it is replaced. `jiff` has its place (it was in
      the tree already, under `env_logger`): the time of the day for
      the night light, the clocks of the panel and the lock screen,
      the names of screenshots and recordings, the time a saved
      state is stamped with. `cargo deny check` passes. What reads
      otherwise than before: the stamp of a saved state ends in `Z`
      where it ended in `+00:00` (nothing reads it back but the
      log), and a clock's format that ends in a lone `%` shows the
      time with the `%` where it showed the format as written. The
      clock's formats are held to what they wrote
      (`the_clock_writes_the_time_as_its_format_says`).
- [ ] **person, any** To be seen on both systems: the panel's clock
      and the lock screen's show the machine's own time (its zone is
      now read by `jiff`), and the night light turns at its hours.
- [ ] **person** The check scripts brought back to this machine
      (`tests/desktop` above all), from wherever the private tree is.
      They stay out of the public repository; `.git/info/exclude` gets
      their folders.
- [ ] **person, any** Decided: whether the checks stay private. The CI
      step for the nested checks looks for `tests/nested/checks.py` and
      does nothing without it.

## 2. Seen on the desktop

Written and tested, not yet seen in use. Each wants a few minutes of the
person at the machine.

- [ ] **person, W** A new window comes in without flicker (`61faf40`):
      Obsidian, a browser, a newer Windows program, with the Scale and
      the Slide entrance.
- [ ] **person, W** The control centre and the notification centre stay
      open with a TeamViewer session in front (`0cfcd38`), and go on a
      press elsewhere.
- [ ] **person, W** The helper for elevated windows, and the launcher
      starting a program elevated (`eadfd11`): an elevated window is
      tiled, moved and closed as the others.
- [ ] **W** The tutorial's tidying after a gevrek that stopped in the
      middle of a lesson: the practice windows gone, the workspace
      closed, the one of before shown.

Written on 2026-11-25, tested without a desktop, to be seen once:

- [ ] **person, any** The tour from the welcome to its last step, on
      each system. The lesson on workspaces now goes to a second
      workspace of the tutorial's own (`Tutorial 2`) and no longer to
      your first one; "make this window wider" now works in Dwindle;
      "let this window float" is seen done.
- [ ] **person, any** The welcome with the window manager stopped: it
      is not marked as answered, and comes again at the next start.
- [ ] **person, W** After a fault in the middle of a pass
      (`GEVREK_INJECT_FAULT=panic`): a dimmed, cut, fullscreen or
      title-less window is as it was before gevrek.
- [ ] **person, W** A window that opens while gevrek is paused is seen.
- [ ] **person, W** A black border is black. Fullscreen made by gevrek
      is undone by `gevrek stop`.
- [ ] **person, L** The lock screen: a second program asking for the
      lock while it is locked is refused; a key held as the lock comes
      is not stuck after unlocking; a window cannot be captured while
      locked.
- [ ] **person, L** Login and logout through gevrek-dm still work (the
      password is forgotten once the session is open; a configuration
      is read with a time limit).
- [ ] **person, M** macOS: a window that opened after gevrek started,
      whose program then hangs, does not hold the keys up for seconds;
      a parked window whose program is slow comes back at the next try.
- [ ] **person, W** The elevated helper after the later rounds: it is
      found and spoken to (the window manager now asks who serves the
      pipe: the copy in Program Files, or the one beside it), bindings
      still reach it (they are told from a thread of their own), it
      comes back after it was ended, and `gevrek elevated install`
      still registers its task (the XML lies beside the helper now).
- [ ] **person, W** A column of the scrolling layout cut at a
      display's edge keeps its border cut after the keyboard goes to
      another window. A mouse binding with a long press does not come
      with the button up.
- [ ] **person, L** A lock screen that is ended (`pkill gevrek-lock`)
      while locked comes again at the next key. Screen sharing of one
      window goes on after the window is given another size. Login
      through gevrek-dm with SELinux on, from the RPM.
- [ ] **person, any** The studio: opened before the window manager,
      something changed, the window manager started: the change is
      kept as a draft. A trial left to run out with the window manager
      stopped and started again meanwhile goes back.
- [ ] **person, any** The web page in a browser (`site/index.html`),
      before it is published. (Drawn by a headless browser at 1400 and
      at 390 pixels wide on 2026-11-25, and looked at: as it should be.)
- [ ] **person** The workflows on GitHub: the actions were brought to
      their current major versions without a run (checkout 7,
      upload-artifact 7, download-artifact 8, the Pages actions).

## 2b. Workspaces (2026-11-17)

Written and tested (the log has what). Still open:

- [ ] **person, L** The panel's tabs, their renaming and the
      hot-plugging of a display behave on Linux as on Windows. (The
      Linux shell builds and its tests pass, in WSL; this is about the
      desktop.)
- [x] **L** The studio's picture of the workspaces and displays pages
      drawn anew, with every other page's (2026-11-17, in WSL; looked
      at; not committed yet).
- [ ] **person, W** Seen on the desktop: a tab renamed twice, on each
      display; a workspace taken out, put in another order and added in
      the studio while windows are open; a workspace of the moment kept.
- [ ] **person** The person's own configuration tidied in the studio
      (`Saturn`, `Gevrek`, `Web` are names there, with `1`, `Gevrek2`,
      `4` as titles on one display; `Gitem-2` and `gitem` are strays).
- [x] **any** Key bindings, gestures, hot corners and rules choose a
      workspace from those there are, instead of a typed name: the one
      chosen is shown, with its title, the others on a press, and a name
      that none of them has is said to be none (`form.rs`,
      `workspace_control`). The graph names no workspace that is there
      (its one name is of a workspace it makes). (2026-11-17, tested,
      not committed yet)
- [x] **any** Rules and a change of the workspaces. Found: a rule goes
      by a workspace's name and its place, never by its title, so a
      title changes nothing for it. What was wrong was the order of a
      reload: the rules were held against the workspaces before they
      were put as the configuration has them (one taken out, another
      put before it). They are held against them after it now
      (`engine/reload.rs`), on both systems. (2026-11-17, tested, not
      committed yet)
- [x] **any** The overview of every workspace goes to a workspace by
      its display and name (by its place only where it has no name),
      and it and the window switcher show a workspace's title.
      (2026-11-17, tested, not committed yet)
- [ ] **L** The compositor's workspace protocol tells the title.
- [x] **any** A saved session finds its display by what it is known as
      (its device path, or its name) where its handle is another, and
      the workspace each display showed by its name. Older sessions are
      read as before. (2026-11-17, tested, not committed yet)
- [x] **W** The panel's modules display by display on Windows
      (`panel::bar_on`), and a display the configuration gives no panel
      has none and keeps no room for one (`Outputs::bar_on`).
      (2026-11-17, tested, not committed yet; to be seen by the person)
- [x] **any** The studio does not offer Apply while the window manager
      is away, and does not go on sending what is changed live; what is
      changed stays and can be applied when it is back. (2026-11-17,
      tested, not committed yet)
- [x] **any** From the studio's audit: the slider ranges lie within the
      model's limits, with a test that holds them there (border width,
      opacities, fps, the zones' two parts of one, gestures, long
      press, hot corners); a new startup program or scratchpad names
      the terminal of the system it is made on; `--file` mode does not
      write over a file that was changed elsewhere without saying so (a
      second Save does). (2026-11-17, tested, not committed yet)

## 2c. Next time on Linux

What the Windows sessions since 2026-11-17 left for Linux. Most of it
was done from Windows after all, on 2026-11-17, once it was found that
the Linux side builds in WSL (section "Where things stand"): what is
ticked here was built and tested there, and not seen on a desktop.

- [x] **L** It builds, and the tests pass (`cargo test --workspace
      --exclude gevrek-win32` in WSL; all but the studio's pictures).
      `shell/src/main.rs`, edited unbuilt before, was right.
      (2026-11-17)
- [x] **L** A renamed workspace is on its tab at once: the compositor
      publishes its state before it answers an Apply (it did so a
      quarter of a second later at the latest), and the shell asks for
      it on `Applied`. (2026-11-17, built; to be seen)
- [x] **L** Hot corners are watched by the compositor
      (`State::tick_hot_corners`, from the pointer's movement and from
      the host's passes, which go on while the pointer rests in a
      corner). A test of the compositor holds it. (2026-11-17)
- [x] **L** The overview of the windows there. The compositor drew the
      windows as pictures already; they were fitted into a grid that
      filled the display. They are placed by `expose::arrange` now, as
      on Windows: each at its shape, all smaller by one measure.
      (2026-11-17, tested in the engine; to be seen)
- [x] **L** The compositor's workspace protocol tells a workspace's
      title as its name; its lasting id is still made of its name.
      (2026-11-17, built)
- [x] **L** The two settings of the workspace slide say in their own
      words that they are for Windows (what the studio shows beside
      them). (2026-11-17)

Still open there:

- [x] **L** The studio's pictures drawn anew, every page (the test
      writes in gevrek's own letters now, section 1; and workspaces,
      displays, animations, panel, snap zones, input, window rules,
      mouse and gestures had changed besides). Drawn in WSL with the
      settings above, and looked at. (2026-11-17)
- [ ] **person, L** The picture test passes on the person's Linux too
      (`cargo test -p gevrek-studio`). If a page is off there, say by
      how much before anything is kept anew.
- [ ] **person, L** The desktop as on 2026-11-17 (the person's own
      round of tests), again: nothing of it should be different.
- [ ] **person, L** Seen: the panel's tabs and a tab renamed; a display
      plugged in and out; the hot corner; the overview of the windows
      (smaller pictures, in rows); a window dragged to the display
      beside.
- [x] **L** A window chosen in the overview there (a press on its
      picture) ends the overview and has the keyboard, as on Windows;
      it used to be brought to the middle. (2026-11-17, a scenario of
      the engine holds it; to be seen)
- [x] **L** The overview of the windows on Linux has what it has on
      Windows: each window's title under its picture, a frame around
      the one pointed at, the arrows and Tab from picture to picture,
      Enter to go to one, Escape to put it away
      (`backend-smithay/src/window_overview.rs`; the way an arrow goes
      is the engine's for both, `expose::towards`). A test of the
      compositor holds the keys; what is drawn was looked at in a
      picture of the compositor without a display (three windows of
      the test client). (2026-11-17)
- [x] **any** A pointer that lies in a hot corner when gevrek starts
      does nothing until it has been moved out of it: the compositor
      has the pointer at the top left before anybody moved it, and
      showed the overview by itself at the start of a session (found
      by that picture). (2026-11-17, tested)
- [ ] **person, L** A new startup program and a new scratchpad in the
      studio name kitty there, not Windows Terminal.
- [ ] **L** CI, the Linux job (section 1): on a push.

Not Linux's, but waiting for the person on Windows from the same days:

- [ ] **person, W** The overview: how it looks and feels with many
      windows, on both displays, with a program that draws its own
      transparency; that a window is never left unseen after it.
- [ ] **person, W** The hot corner at the top left, with the panel
      there; the moment it takes (250 ms).
- [ ] **person, W** A workspace renamed from its tab: there at once.
- [ ] **person, W** A window dragged to the display beside goes there
      at the first try (reported 2026-11-17: it took a few). It went to
      the display most of it was on, and most of a large window is
      still on the one it came from when it is let go just beyond the
      edge. It goes to the display the pointer is on now
      (`display_dropped_on`), on both systems.
- [ ] **person, W** Borders around nothing, seen now and then on a
      workspace (reported 2026-11-17). One cause found and closed: a
      window is hidden by a message it answers later (and one that
      fades out with its workspace only after that), Windows calls it
      visible meanwhile, and a border placed in that moment was shown
      again with nothing to hide it afterwards. A window gevrek has
      hidden or asked to be hidden has no border now, and one that went
      out with its workspace loses it when it is hidden. To be seen:
      whether they are gone. If not, the next to look at: a window that
      shows itself again on a workspace that is not shown.
- [ ] **person, W** The setup program with the helper's checkbox, an
      update over it, and the uninstall (never run on this machine);
      the release workflow's new step on a runner.
- [ ] **person, W** The two settings of the workspace slide: what they
      do to the graphics card's load (nothing was measured).

**This file is in the repository** (since 2026-11-17): a pull on the
Linux system brings it. Its archive, `docs/ROADMAP-archive-2026-11.md`,
is not (`.git/info/exclude`); nothing here needs it.

## 2d. Faults and speed: the pass before 0.7.0

Asked for on 2026-11-17: faster, better, stronger, more efficient, and
nothing broken on the way. Nothing new is built in this pass.

**The rules of it**

- What works keeps working. The configuration's format, the commands,
  the protocol and what a key does stay as they are (the golden files,
  the schema's tests and `core_scenarios` hold them).
- A fault that is mended gets a test that failed before it was. Where a
  test cannot reach it (it wants a desktop), it is a `person` item with
  what to look at.
- Nothing is made faster that was not measured first. Each such change
  has its number before and after in the log, and is taken back where
  the number is no better.
- One thing in one commit. Moving code and changing it are never the
  same commit.
- Before each commit: the tests on Windows, and the Linux side in WSL.
  No full desktop run unless the person asks.

**Step 0: the net** (first; the rest leans on it)

- [ ] **any** CI green to the end on both systems: the pictures pushed,
      `chrono` replaced (section 1, done). From then on a red run stops the
      pass until it is green again.
- [ ] **any** The numbers as they are now, written down in the log, on
      the person's machine: processor and memory of a still gevrek
      (window manager, shell) over five minutes; handles and GDI objects
      on Windows; the time from start to the first layout; the size of
      the programs.
- [x] **any** Benches for what the engine does most
      (`crates/engine/benches/engine.rs`, no crate added for it):

          cargo bench -p gevrek-engine --features test-support

      A layout of 1, 10 and 50 windows in each layout; windows opened
      and a workspace changed on the desktop without a windowing
      system; 20 rules against 50 windows; the overview's arrangement;
      the starting configuration (116 bindings) read, written, checked
      and held against another for a reload. `GEVREK_BENCH_OUT=<file>`
      writes the medians down. The first numbers are in the log
      (2026-11-17, late evening).
- [x] **any** The same for the state as it is published: it is put
      together in the engine after all (`WmCore::monitor_data`), and
      the bench has it. 9 workspaces with 30 windows: 34 µs on the
      desktop without a windowing system; held against the one before,
      45 ns; copied, 7.7 µs. What it costs on Windows is what it asks
      the system for each window, which the time of a pass shows (the
      next item), not a bench.
- [x] **any** The state on its way out, timed
      (`cargo bench -p gevrek-server`; the conversion is a function of
      its own for it, `overlay_response`, moved as it was). For 30
      windows, 9.5 kB on the line. In the server, for each one who
      asks: copied 9 µs, made into the message 10 µs, written 5 µs. At
      the other end: read 18 µs, made into JSON 145 µs. A quarter of a
      thousandth of a second for a question and its answer, most of it
      the JSON in the shell and the studio.
- [ ] **person, W** The time of a pass of the runtime, measured where
      it runs (`runtime/pass_times.rs`, tested): a pass is timed in six
      parts (commands, looking around, events, layout and motion,
      checks, state and effects). Nothing is said unless asked for:

          RUST_LOG=info,gevrek::passes=trace

      Then a pass longer than a frame (16 ms) is told at once with its
      parts, and every half minute the passes are summed up (the sum
      alone at `debug`, which is on by default before 1.0). To tick:
      the person runs gevrek so for a while of ordinary use, and the
      lines go into the log here; they say what step 2 begins with.
- [ ] **any** The large files have their behaviour held by tests before
      they are opened: `window_manager.rs` (8,199 lines),
      `settings.rs` (5,502), the Linux shell's `main.rs` (4,889),
      `server/src/lib.rs` (3,591). Where a part has none, tests that
      say what it does now come first.

**Step 1: faults**

- [ ] **person, W** The two that were reported and mended without being
      seen: borders around nothing; a window dragged to the other
      display (section 2c). Not closed until seen for a day.
- [x] **any** A panel is its own display's alone (reported by the
      person 2026-11-19): a window on one display is shown, many times,
      in the taskbar module of the panel on the other. Found: a panel
      never went by where a window lies. It draws its own display's
      part of the window manager's state, and that lists a window for
      the workspace it belongs to (`engine/appearance.rs`,
      `panel::view_at`); the tabs, their counts, the title and every
      press (a workspace by its display and name, a window by its
      handle) go by the same. What was wrong is that the panel was not
      told. A window let go of on the other display belongs to that
      display's workspace from then on, and the panels ask for the
      state only after certain events (`panel::follows`), of which the
      end of a drag was none: the panel of the display the window had
      left went on listing it, and the other one did not have it, until
      something else happened (the keyboard went to another window, a
      workspace was shown). The log of the day has seven such drags,
      each followed by one to four seconds in which nobody asked for
      the state. The same was so of a window sent across by a key of
      the graph (`binding.triggered` is not followed either). Mended
      twice over: the panels ask after `window.drag_ended`, and the
      window manager tells `window.workspace_changed` whenever a window
      is published for another workspace or display than the last time
      (`overlay_data::changes`), whatever brought it there, on both
      systems. Two cases in which a window did keep the wrong display's
      workspace are mended with it (`executor/events.rs`): a floating
      window whose drag's end was never told, and a tiled window
      dropped on a snap zone of the other display. Held by tests: the
      published state lists every window once, for the display its
      workspace is on, across the edge, with its workspace put away,
      minimized, dragged over, sent over, and with a display going and
      coming (`core_scenarios.rs`); the same after every step of the
      seeded runs (`random_runs.rs`); two panels read from one state
      share no window, and a press is for the panel's own display
      (`shell/src/panel.rs`). (2026-11-19, tested, not committed yet;
      to be seen by the person)
- [ ] **person, W** The above seen on the desktop: a window dragged to
      the other display leaves the first panel's taskbar and stands on
      the second's as it is let go of, tiled and floating, both ways.
      (What was left to be decided here, a floating window moved across
      with no drag, was decided on 2026-11-19: the next entry.)
- [x] **any** A floating window that has moved to another display
      without a drag joins that display (decided by the person
      2026-11-19). Its program moved it, or Windows did for
      Win+Shift+arrow: it stayed its old workspace's, and on the panel
      of the display it had left, until it was dragged or sent. Now a
      floating window of a workspace that is shown, in sight, neither
      maximized nor fullscreen, that is told of as moved and lies with
      more than half of it on another display (or, where part of it is
      beyond them all, with its middle there) is that display's
      workspace's once it has stayed there for a quarter of a second,
      by the same step a drag's end takes (`floating_window_is_on`), and
      `window.workspace_changed` tells the panels
      (`executor/events.rs`: `note_moved_across`,
      `settle_moved_windows`, looked at by both hosts beside the drags,
      every half second on Windows and every quarter on Linux). One
      that lies half and half stays with whichever display has it. Not
      for a window the desktop is moving itself (a workspace that
      slides), one that is hidden or on a workspace that is put away
      (what it says of its place is where it was put away), one that
      is held (the drag's end says, by where the pointer is), one
      gevrek has just placed, one put out of the way, a tiled one, and
      nothing is carried over a change of the displays. Held by tests:
      `core_scenarios.rs` (moved and rested, told once, back again;
      across the edge; every case in which it must not happen) and the
      seeded runs, which now move windows as their programs do
      (2026-11-19, tested on both systems, not committed yet).
- [x] **any** A window that belongs to another window opens on that
      window's workspace (decided by the person 2026-11-19; reported: a
      dialog Unreal Editor opens while one works on another workspace
      comes up there, with the keyboard). The backends say which window
      a window belongs to (`WindowQueries::parent`: the parent of a
      Wayland toplevel, the window an X11 window is transient for, the
      owner on Windows). Where the nearest such window that is on a
      workspace is on one that is not shown, the new window joins that
      workspace, floating, out of sight with it, and the keyboard stays
      where it was; it is there with its window when the workspace is
      shown (`executor/window.rs`: `join_parent_out_of_sight`, for a
      window gevrek takes itself and for one the graph arranges). A
      rule of the person's that sends the window to a workspace wins;
      a window whose window is in sight, minimized by the person, a
      scratchpad's, or on no workspace opens as before. On Linux a
      window with a parent was no window of gevrek's at all until now:
      it was told to the rules as a "child window", which the system
      filters leave alone, so it stood on every workspace; it is told
      as a window with an owner now (`backend-smithay/desktop.rs`,
      `facts`) and is a floating window of a workspace like any other
      dialog (with a button of its own on the panel's taskbar, as those
      have; menus, tooltips and notifications are no toplevels, or are
      left to their program as before). On Windows most windows that
      belong to another are no windows of gevrek's (`window_kind`), and
      were hidden with their owner only if they were there when it was
      hidden: one that opened afterwards came up where the person was.
      Such a window, shown while a window above it in its chain of
      owners is hidden by gevrek, is hidden with it now, and shown
      again with it; so is what a program puts up beside its windows (a
      notification it draws itself) while all its windows are hidden,
      never while one of them is in sight; and the keyboard goes back
      to the window that had it (`WindowManager::hide_late_arrival`,
      asked by the runtime as each window is shown; in the journal of
      hidden windows like every other). Held by tests: the decision
      (`late_arrival_tests`), `core_scenarios.rs`, the seeded runs
      (windows open belonging to others, wherever those are), and two
      real Wayland clients (`backend-smithay/tests/tiling.rs`)
      (2026-11-19, tested on both systems, not committed yet).
- [x] **any** Under the Floating layout a window that belongs to
      another, or that stands where its program put it, keeps its place
      (decided by the person 2026-11-19): only a program's own window
      is brought to the middle of the display as it comes. The backend
      says which windows stand at a place of their own
      (`WindowQueries::placed_itself`: on Linux an X11 window that names
      its place, and a dialog put over its window); such a window is
      only brought to the middle where less than 32 by 16 of it would
      be on the display (`executor/layout.rs`, `settle_floating_layout`;
      the measure is the one a workspace that is shown goes by,
      `executor::reaches`). Held by tests: `core_scenarios.rs` and a
      real Wayland client whose dialog stays over its window
      (2026-11-19, tested on both systems, not committed yet).
- [ ] **person, W L** The three above seen on the desktop. A floating
      window sent across with Win+Shift+arrow, and one a program moves
      across, stands on the other panel's taskbar within a second, and
      back; nothing changes panel while a workspace slides. A program
      on a workspace that is put away opens a dialog (Unreal Editor
      while one works elsewhere): nothing comes up, the keyboard stays,
      and the dialog is over the editor when its workspace is shown. On
      a workspace with the Floating layout a dialog opens over its
      window, and a program's first window in the middle.
- [x] **any** The notes of the kind TODO and FIXME in the code: there
      are none. The 64 that were counted are a variable called `todo`
      (what the tutorial is to do next) and one `XXX` in the words for a
      display's name; no `todo!()` and no `unimplemented!()` either
      (2026-11-17, looked for). One field is never filled:
      `OverlayData::metrics` ("can be added later",
      `runtime/mod.rs`): the time of a pass could go there, or the
      field could go.
- [ ] **W** The `unsafe` of the Windows backend read through (most of
      the 507 in the tree are there): every handle closed once and by
      whoever owns it, nothing used after its window is gone, a line
      beside each block saying why it holds.
- [ ] **any** The 71 places that sleep: each one waits for something
      that could be waited for itself (an event, an answer, a time
      limit), or says why it cannot.
- [ ] **any** What can end the program outside the tests (`unwrap`,
      `expect`, an index, a lock that was poisoned): on the paths that
      run while the desktop does, each becomes a fault that is logged
      and lived through, or says why it cannot happen.
- [x] **any** No configuration and no saved session from outside ends
      the program (`engine/src/outside_input.rs`): each of the examples
      and a saved session, cut short at a hundred places and spoilt
      some thousand ways (nonsense where a value belongs, numbers as
      large as they get, lists emptied or made very long, keys taken
      out), and taken as far as a real one: read, checked, tidied,
      written back, made into rules and zones, laid over a desktop
      with windows, the windows laid out. Ten seconds with the tests;
      `GEVREK_SPOIL=<n>` does n times as much in other ways (50,000
      were tried so, 2026-11-17). Found and mended: a pin or a counter
      of the graph as large as numbers get was counted past (it ended
      a debug build and began again at nought in a release).
- [x] **any** The same for a command as JSON (`gevrek dispatch`, the
      panel, a script): one of nearly every command (found by asking
      the reader what is missing, so one that is added is there too)
      and those of the examples, cut short at every letter and spoilt,
      carried out twice with windows on the desktop
      (`no_command_from_outside_ends_the_program`; 40 times as many
      tried, 2026-11-18). Found and mended: a window moved or resized
      by, and gaps, padding and the master count changed by, a number
      as large as numbers get were counted past in fifteen sums (a
      debug build ended; a release put the window at the other end of
      the numbers). The spoiling is a file of its own now
      (`engine/src/spoil.rs`) that another crate's tests take in.
- [x] **W** The same for the journal of hidden windows, read before
      anything else at the start
      (`no_journal_from_outside_ends_the_program`): nothing found.
- [x] **any** The shell's own files: how often an application was
      started, as large as numbers get, was counted past; and a note
      written down with such an age ended the shell at its start. Both
      mended, with the nonsense a file may hold tried
      (`counts_from_outside_are_lived_with`,
      `what_was_written_down_by_another_is_lived_with`).
- [x] **W** What the elevated helper is asked. What it does to a
      window is behind a desk that can be stood in for
      (`elevated_helper::Desk`; the helper's own is `TheDesk`), and
      with one that only writes down: of the flags of a placing only
      those that place, of the ways to show only those that show and
      hide, of a style only gevrek's bits, of what the compositor is
      told only gevrek's seven, no window that may not be asked
      about, no binding of a plain letter to the hooks; what was
      hidden is shown again when the asker goes; and what is asked,
      spoilt and cut short, is answered line for line and takes the
      helper no further
      (`no_asking_from_outside_ends_the_helper_or_takes_it_further`).
      Nothing found.
- [x] **L** The same for the display manager's service, which runs as
      root and reads what the login's program writes
      (`service::Machine`, `TheMachine`): a login is a person's
      session on the session's console with the session's command
      and nothing added, one at a time; the machine is not switched
      off under a session; who is not let in may try again; when the
      session is over the login's console is shown and the
      session's let go; a line longer than a name and a password is
      not understood
      (`no_asking_from_outside_ends_the_service_or_takes_it_further`).
      Nothing found.
- [x] **any** The shell's smaller files: the tutorial's memory spoilt
      and cut short (a step no lesson has is the lesson's last), and
      the tray's choice and the clipboard's texts with what is no
      list of words, more texts than are kept and one longer than a
      text may be (`a_memory_from_outside_is_lived_with`,
      `a_choice_from_outside_is_lived_with`,
      `texts_from_outside_are_lived_with`). Nothing found. (The
      wallpaper has no file of its own: what it shows is in the
      configuration.)
- [ ] **any** Left of what is read from outside, looked at and not
      spoilt: the session's worker (it reads its order from the
      service, which is root as it is, and what follows is PAM), and
      what the helper tells the window manager (the reader has fewer
      rights than the writer).
- [x] **any** The engine driven by long runs of chance
      (`engine/src/random_runs.rs`): the commands the examples bind
      keys to, more with numbers beyond their edges, and windows that
      open, close, are minimized, dragged and get the keyboard. After
      each step: the program lives, every window is in one place only,
      none that has gone is kept, no tile is without a window, the
      tiles have a size. 24 runs of 400 steps with the tests (a third
      of a second); `GEVREK_RUNS=<n>` for n times as many, and
      `GEVREK_SEED=<seed>` for the one a failure names. 7,200 runs were
      clean (2026-11-17). Found and mended: Toggle floating, for a
      window that has the keyboard and is on a workspace that is not
      shown, put it on the shown one as well, and it was on two from
      then on (`a_window_on_another_workspace_floats_where_it_is`).
- [x] **any** Found by the same runs: the layouts that give each new
      window a share of the last one's room (BSP, dwindle, the
      spiral) came down to tiles of a few pixels, one and none with
      many windows (nine windows, split one way, at 0.7: the last
      were 3, 1 and 0 wide). Decided by the person (2026-11-18): a
      tile is not made smaller than 120 pixels
      (`layouts::SMALLEST_TILE`). A room that can be shared no
      further (one of the two would be smaller, or the rest would
      not hold the windows still to come) is shared in equal parts,
      in rows, by the window that has it and all after it. With few
      windows nothing is other than it was; on 1920 by 1080, halves,
      the first four tiles of twelve windows are where they were and
      the other eight are a grid in the last room
      (`a_tile_is_not_made_smaller_than_the_smallest`,
      `few_windows_are_laid_out_as_they_were`). With more windows
      than a display holds at that size the tiles are smaller, and
      equal. Dragging an edge between tiles of such a grid moves
      nothing.
- [ ] **person** The same seen on a desktop: ten windows and more on
      one workspace in dwindle, BSP and the spiral.
- [x] **any** The runs of chance with more in them: every other run
      has two displays, the second coming and going, windows opening on
      either; the rules and the scratchpads of the examples; and one
      more thing held: no window of a workspace that is not shown is
      in sight. 6,000 runs clean in the end (2026-11-17). The desktop
      of the tests takes the keyboard from a window that is hidden, as
      every real one does. Found and mended, each with a test of its
      own in `core_scenarios`:
      - A layout command (a gap, a ratio, the layout itself) given
        while the window with the keyboard is on a workspace that is
        not shown laid out that workspace and brought its window into
        sight. It is for the workspace that is shown now.
      - "Move to a workspace and follow" followed on the display with
        the keyboard, not on the window's: with the keyboard on the
        other display (its desktop pressed on, a workspace sent there),
        that display changed to whatever it had at the same place and
        the window was left out of sight. Its own display follows now.
- [x] **any** The runs of chance wider still: one of nearly every
      command there is (131 of them were given in four runs looked at),
      those that choose a window, a workspace or a display among them;
      the configuration read anew in the middle of a run (another
      example's rules, scratchpads, workspaces and graph, and its
      startup run again); the keys the graph waits for pressed; and
      one more thing held: the display that is worked on is one that
      is there. 1,440 runs clean (2026-11-18): nothing found. Left out
      on purpose: "restore windows" (it shows every window wherever it
      is, for when something went wrong), and the tiles' size is not
      looked at while the tiling is held. Thin: only one example's
      graph waits for a key, so a key of the graph is pressed seldom
      (the next item).
- [x] **any** The runs of chance with a graph nobody drew
      (`graph_of_chance`): behind each of eight keys, behind what
      windows do and behind the start a row of nodes of any kind;
      what a node wants handed to it from any node that has it, round
      in a circle too; and what is written in the nodes spoilt now
      and then. Every other pair of runs begins with one, and every
      other configuration read anew brings one. 2,400 runs clean in
      the end (2026-11-18). Found and mended, each with a test that
      failed before:
      - A sum (or any node that only works a value out) that is one
        of its own numbers was followed until the program had no
        room left and ended; so were two loops that run each other,
        which begin their row anew each time and so had no end. A
        node that is being worked out is not asked again, a row is
        64 nodes at the most and a value 32 behind each other (the
        longest row in gevrek's own graphs is six), and no more than
        250,000 nodes are run for one thing that happened
        (`a_node_that_is_asked_for_its_own_value_is_not_followed_for_ever`).
      - A loop that counts to the largest number there is, by large
        steps, counted past it (the same test).
      - A scratchpad's window that was given a place on a workspace
        (made to tile or to float, moved, taken back among the
        managed) stayed the scratchpad's: asked for from another
        workspace it was in sight and on one that was not. It is an
        ordinary window from then on, and the scratchpad takes
        whichever window is its kind when it is asked for
        (`a_scratchpads_window_given_a_place_is_the_scratchpads_no_longer`).
        Reached by the command as well as by the node.
      - A scratchpad's window that was put away and then told of as
        restored from being minimized was given a tile
        (`a_scratchpads_window_that_is_restored_is_given_no_tile`).
      - A graph is handed the window that closed, and moved it to a
        workspace or arranged it, at once or by a key pressed
        afterwards: it was kept there for good. A window that has
        gone is given no place, and what the graph was told of it is
        not kept (`a_window_that_has_gone_is_given_no_place_by_a_graph`).
- [x] **person** Decided (2026-11-18), since it changes what a key
      does: with a scratchpad shown and the keyboard in it, "toggle
      floating" (or a move to a workspace) makes its window an
      ordinary one; the scratchpad's key then takes a window of its
      kind again, this one if there is no other. It stays so.
- [x] **any** Several nodes that wait for the same key, or for the
      same thing a window does, were run in the order a table of
      that run happened to give them: a graph did one thing first
      today and another tomorrow, and a run of chance was not the
      same twice. They are run in the order they have in the graph
      (`the_nodes_of_one_key_are_run_in_their_order`).
- [ ] **person, W** Both of these on the desktop with two displays:
      they may be part of what was reported there (windows not where
      they belong after a move to the other display; borders around
      nothing).
- [ ] **any** Not yet in the runs: a window given the keyboard while
      it is out of sight (none of the systems does it, but a program
      may ask), commands that name a window by a selector, the graph's
      nodes, a reload in the middle. And not yet held: the focus on a
      window that is there.
- [ ] **W** What happens around the edges, each with a test or a
      `person` item: a display taken away and put back, another scale,
      sleep and waking, the Explorer started anew, a window of an
      administrator's program without the helper, the window manager
      ended by force (the hidden windows come back).
- [ ] **W** Nothing is left behind over time: ten thousand changes of
      workspace and a thousand windows opened and closed, with the
      handles, GDI objects, pictures of windows, borders and memory
      counted before and after.
- [ ] **L** The same for the compositor without a display (headless):
      clients that come and go by the thousand, surfaces and buffers
      counted.
- [ ] **any** Every answer on the line has a time limit, and each side
      lives through the other going away and coming back (the shell
      without the window manager, the studio without either).
- [ ] **any** The compositor's workspace protocol tells the title:
      written, and without a test (section 2b).
- [ ] **person, W** Reported by the person (2026-11-17): with another
      scale chosen for a display, what the panel shows grew and the
      panel did not, so it was cut off. Found: the window manager says
      where the panel is and keeps its room, and took the height and
      the margin of the configuration as pixels whatever the display's
      scale; the shell draws at the display's scale. Now both are as
      many pixels as the display's scale makes them, where the displays
      are measured in their own pixels (Windows; a compositor counts
      the scale in already)
      (`the_panel_is_as_high_as_its_displays_scale_makes_it`). To tick:
      seen at 100 %, 125 % and 150 %, and with the scale changed while
      the panel is up.
- [ ] **person, W** Asked by the person (2026-11-18): the layouts
      follow a display's scale as it is changed. The windows were laid
      out again already; the lengths of a layout were pixels whatever
      the scale. Now the gaps (round the windows and between them), the
      border (its width and its offset), the tab strip of a group and
      the room of the borders that are not seen are as many pixels as
      the display's scale makes them, on Windows; a workspace is told
      its display's scale each time it is laid out, and what the
      configuration and the commands say stays what it is
      (`a_layout_is_laid_out_by_its_displays_scale`). Not changed: the
      edges kept free for other programs and a workspace's offset
      (they are pixels of the display). To tick: seen at 100 % and
      150 %, with the scale changed while windows are tiled.
- [x] **any** The studio's top bar at the smallest window (760 wide):
      the file and the state lay under the search. Narrower than
      1200, the bar has two rows now: the name, the state and the
      window's buttons, and under them what is done with the document
      (search, undo, apply, the sandbox). Looked at in a picture at
      760 wide with a file open; with a window manager connected and
      a draft (the most there is in the row) it is not looked at.
- [ ] **person, W** Reported by the person (2026-11-17): the toggles
      for a display's scale on the studio's Displays page did nothing
      on Windows. The studio wrote the scale to the configuration, and
      the Windows backend never applied one. Decided: gevrek sets it,
      as the Settings window does. Written (`display_scale.rs`): the
      scale an entry of `desktop.outputs` gives its display is set
      where the display offers it (Windows' own steps, 100 % to 500 %,
      counted from the one it recommends), said and left where it does
      not; a display whose entry gives none any more gets back what it
      had. Windows has no documented way for this: the two values the
      Settings window itself uses are used, and a later Windows may
      answer them otherwise (then nothing is set and the log says so).
      The page shows the scale the display has where none is written
      down. Tested: the plan and the reading of Windows' answer; asked
      on the person's machine, the three displays answer rightly.
      **Not run: the setting itself**, which changes how the person's
      screen looks. To tick: a scale chosen on the page is the
      display's, another is, and with the entry's scale taken out it
      is the one of before.
      Note: a configuration that already gives a display a scale (the
      toggle was pressed before, and did nothing) has it set at the
      next start.

**Step 2: speed and what it costs**

By what step 0 measured; the order below is a guess until then.

- [x] **any** Found by the first benches: a rule that matches by a
      regular expression has the expression made anew for every window
      it is held against (`rules.rs:32` and `:678`, `matching.rs:17`).
      20 rules, five of them such, against 50 windows: 10 ms, where all
      else the engine does for a window is measured in millionths of a
      second. The rules are held against every window after each
      reload and for every new window. Now an expression is made once
      and kept (`matching::regex_for`; a pattern that does not compile
      is kept as that): the same bench, 9.96 ms before, 0.95 ms after
      (2026-11-17, tested).
- [ ] **any** What is left of that bench (a millionth of a second for
      each rule and window): every match that does not regard case
      writes both texts out in small letters first. Only if a number
      says it is worth it.
- [ ] **any** The state made into JSON at the asking end (145 µs of the
      190 a question costs): the shell and the studio read a few fields
      of it. Worth it only if the time of a pass or a still desktop's
      numbers show that the state is asked for often.
- [ ] **any** Looked at after that, by the same numbers: a reload's
      plan (0.5 ms: both configurations are written out to be held
      against each other, part by part) and the checking of a
      configuration (0.28 ms). Neither runs often; only if cheap.

- [ ] **any** A still desktop does nothing: no pass, no state, no
      picture drawn while nothing happens. Every timer that fires by
      itself (the state once a second, the shell's and the studio's
      repaints, the hot corners) is named, and stays only where
      nothing else can wake what it wakes.
- [ ] **any** The state is put together only when something changed,
      and only what is asked for; what did not change is not written to
      JSON again. Told to those who listen once per pass, not once per
      event.
- [ ] **W** A pass places only the windows whose place changed, and
      asks Windows once for what it asks several times now.
- [ ] **W** Motion: one frame's work in one frame's time on both
      displays, with the frames that came late counted; borders and
      pictures of windows drawn only when they change (begun in
      `8314a98`).
- [ ] **L** The compositor draws what changed and no more, on each
      display by itself; what a frame is made of is kept from the frame
      before where it is the same.
- [ ] **any** The shell: the panel draws when what it shows changes;
      what it reads of the system (network, sound, battery, the
      numbers) is read as often as it can change and no oftener, and
      not at all for a module that is not shown.
- [ ] **any** The studio: a page is not laid out anew while nothing on
      it changes; the schema and the form's lists are made once.
- [ ] **any** From start to the first layout, and a reload: what is
      read or made twice is read or made once.
- [ ] **any** Memory: what is copied for every event (the state, the
      configuration, the lists of windows) is lent where it can be.
- [ ] **any** The numbers of step 0 taken again and written beside the
      first; a check that fails where a bench grows by more than a
      tenth ("Kept an eye on", section 5, is this).

**Step 3: stronger**

- [ ] **any** The large files in parts, each part moved as it is and
      its tests with it; nothing changed in the same commit.
- [ ] **any** One way to do a thing where there are two (found on the
      way through steps 1 and 2, listed here as they are found).
- [ ] **any** The warnings: clippy without any on both systems (two
      are known in the shell), then CI refuses new ones.
- [ ] **any** What the log says at each level read through once: a
      fault says what was being done and to which window; nothing is
      said a hundred times a second. Known already: the server writes
      a line at the info level for every question for the state
      (`"GetOverlayData RPC called"`, `server/src/lib.rs`).

**Done when** every item above is ticked or handed to the person with
what to look at, the numbers are no worse anywhere and better where
this was after them, and CI is green. Then 0.7.0 (section 5).

---

## 3. The tutorial

The designs are in the archive under "The welcome and the tutorial". The
code: `crates/tutorial`, `shell/src/tutoring.rs`, `shell/src/coach.rs`,
`studio/src/welcome.rs`, `studio/src/sample.rs`.

### Stage 1: the tour, finished

- [x] **any** Practice windows say how they stand: the keyboard,
      floating, maximized, a tab of how many. A practice window asks
      the window manager (`Standing::of`, by its title and its
      application) when something happens on the desktop, and writes it
      under its name. (2026-11-17, tested; to be seen in the tour)
- [x] **W** The ring around what a step is about: the window with
      the keyboard, every window of the workspace, or the panel. What
      it is around is worked out the same on both systems
      (`ring_around`, `ring_sides`); the Windows shell draws it as four
      thin surfaces in the accent's colour, put where they belong as
      the desktop changes (a surface as large as the display would be
      black there). (2026-11-17, the places tested; to be seen)
- [ ] **L** The ring on Linux: a compositor places a shell's surface
      itself, so the four sides cannot be put at a rectangle there. It
      wants either one surface over the display with a see-through
      ground, or the compositor to draw the ring when asked.
- [ ] **any** A ring around a surface of the shell's own (the
      launcher, the control centre): the shell knows where it put them.
- [x] **any** The tutorial on the display the person is on, not always
      the main one: the display with the keyboard when it begins, by
      its name; the main one where the desktop does not say.
      (2026-11-17, tested; to be seen with two displays)
- [x] **any** A window of the person's own on the tutorial's workspace
      no longer keeps that workspace from going at the end: it is taken
      to the workspace that was shown before (`tidy_commands`), also
      after a gevrek that stopped in the middle. (2026-11-17, tested)
- [x] **any** The card does not lie over the lower right window's foot:
      its room at the foot of the display is kept free of tiles on the
      tutorial's workspace (`card_room`, the workspace's own offset,
      which goes with the workspace). (2026-11-17, tested; to be seen)
- [x] **any** The welcome window: the main key chosen there, Super or
      Alt. Every binding that was held with the one is held with the
      other (`DesktopConfig::set_main_key`: the two change places, so
      nothing collides that did not before), and what then lies on keys
      the system or most programs use is said under the choice.
      (2026-11-17, tested; the page looked at in a picture)
- [ ] **any** The welcome folds into the coach's card instead of
      closing (the welcome is the studio's window, the card the
      shell's surface: to be seen on a desktop).
- [x] **any** Every lesson played by a test, each step done by its own
      command and seen done, against the real core and the shared host
      on a desktop in memory, with the state read as the shell reads it
      (`tests/tutorial_played.rs`, 2026-11-25). What stands in: the
      practice windows as programs and the shell's own surfaces. Found
      by it and fixed: the lesson on workspaces went to the person's
      own first workspace (the next one after the tutorial's, the last
      of the display) and tiled a practice window among theirs, so it
      has a second workspace of its own now; `resize_axis` did nothing
      in Dwindle, the layout gevrek starts with; a floating window with
      the keyboard was told as without it in the state, so "let this
      window float" was never seen done. Also: a surface put away
      before the step that asks for it no longer holds the lesson up.
- [ ] **any** The same on a real desktop, by the check scripts
      (section 1): the card, the ring, the practice windows as programs.

### Stage 2: the chapters

Only the tour's lessons are written (`lessons.rs`). Each of these is a
lesson or a few and lands by itself, in this order:

- [ ] **any** The layouts and the scrolling strip.
- [ ] **any** Groups and tabs.
- [ ] **any** Floating and snap zones.
- [ ] **any** Several displays (only shown where there is one).
- [ ] **any** Scratchpads.
- [ ] **any** The panel, the control centre, notifications, screenshots.
- [ ] **any** The pointer and the touchpad.
- [ ] **any** Key bindings, window rules and the look, in the studio
      with its own marks.
- [ ] **any** The automation graph.
- [ ] **any** Session, lock and power (shown, not carried out).
- [ ] **any** The command line.
- [ ] **any** When something goes wrong.

### Stage 3: around it

- [ ] **any** What is new after an update: one card on the first start
      after it, a line for each thing with its lesson; never on a first
      launch. (A lesson has the version it came with, `since`; nothing
      reads it yet.)
- [ ] **any** The person's own keys on one page, made from their
      configuration: at the tour's end and from the studio.
- [ ] **any** "How do I": after a `?` the launcher finds lessons and
      keys by what they do.
- [ ] **any** The documentation made from the lessons, so that the two
      say the same.

## 4. Waiting for the person

- [ ] **person** The list of what the studio lacks for configuring
      gevrek through it alone; then a week of doing only that.
- [ ] **person, L** gevrek on physical hardware from a text console. So
      far: nested in another session, and a virtual machine.
- [ ] **person, L** Games on the first real machine: Steam, a native
      game, a Proton game, a game inside gamescope, with a list of what
      was tried (`docs/GAMES.md`).
- [ ] **person, L** NVIDIA's own driver on a real display: a fullscreen
      program's buffer handed straight to the display.
- [ ] **person, L** Fingers on a real touchpad, in the studio's gestures
      page and on the desktop.
- [x] **person, any** A Mac, for the macOS backend (section 5): an
      Apple-silicon laptop with macOS 27.2, from 2026-11-22.

---

## 5. After that: proposals

Nothing here is decided. Each is one line of why; move it up when it is
wanted, strike it when it is not.

### Motion without flicker, everywhere (W)

- [ ] The Slide and Scale exits use a picture of the window, as the
      entrances do since `61faf40`. Today the closing window is resized
      on every frame.
- [ ] Open windows moved by a layout change use a picture too, where
      their program cannot keep up with being resized every frame.
- [ ] The same looked at for a workspace's windows going out and coming
      in.

### 0.7.0

- [ ] A release when sections 1 and 2 are done and stage 1 of the
      tutorial is finished: the first version with a first launch that
      explains itself.
- [ ] What changed, written for people: a page on the site and the
      release's text made from one list.

### Getting it to people

- [ ] The Windows programs signed, so that the setup program runs
      without the warning. Needs a certificate: the person's decision
      and cost.
- [ ] winget and Scoop for Windows; the AUR for Arch (a package is in
      `packaging/arch`).
- [ ] Updates on Linux told of, where the package did not come from a
      repository.
- [ ] The site: a page that shows gevrek moving (short films of the
      layouts, the overview, the studio), made from the checks'
      pictures where they can be.

### The studio as the only way in

- [ ] Whatever the person's list (section 4) names.
- [ ] Every setting of the schema reachable from a page, checked by a
      test that walks the schema and fails for one that no page shows.
- [ ] A configuration brought over from another window manager's file
      (komorebi, GlazeWM, Hyprland, niri): the bindings and the rules,
      with what could not be read said.

### A complete desktop

- [ ] `docs/CAPABILITY_MATRIX.md` is about window management and was
      established on Windows. The same table for the desktop's other
      parts (the panel, notifications, the control centre, the lock,
      the login, sharing the screen), with a column for each system.
- [ ] What Windows lacks beside Linux in the control centre: what is
      playing by its name, the power profiles, a recording of the
      screen.
- [ ] The interface in other languages: what is said gathered in one
      place first, then a first other language of the person's choosing.
- [ ] Screen readers on Windows seen with a real one (UI Automation is
      written, not seen); the same on Linux with Orca.

### macOS

- [x] The backend: `gevrek-wm` runs on macOS (`crates/backend-macos`,
      2026-11-22). Windows through the Accessibility API, tiled, focused,
      swapped, closed; programs that start and end found; workspaces by
      parking (a sliver of a hidden window stays in a bottom corner: macOS
      keeps one; `park.rs`), with a journal that puts them back after a
      crash; displays with their work areas; the bindings' keys from an
      event tap (Win is Option, Alt is Control+Option+Command: `keys.rs`).
      Tried on a Mac with two programs (`GEVREK_MACOS_ONLY` limits it to
      those named), by commands and by keys.
- [x] Borders (2026-11-22): a window of gevrek's own around each window
      (`borders.rs`), put directly above it in the order of windows, that
      follows it, hides with its workspace, and goes when it closes. The
      plain ring only, square or rounded; it trails a dragged window a
      little.
- [x] The double and dashed borders, as on Windows (2026-11-22,
      243980d): two thin rings; dashes along the band, laid anew when the
      window's size changes. Glow, shadow and gradient are refused by the
      configuration on every system, so there is nothing to draw.
- [x] The corner a rounded border follows (2026-11-22): measured in
      captures of Calculator, Dictionary and Activity Monitor and of
      windows of gevrek's own with no toolbar, a toolbar and a compact
      one: every one has the same corner on macOS 27.2, 17 points (it was
      taken as 12). To be measured again when macOS changes its windows.
- [x] Stackbars (2026-11-22, `stackbars.rs`): a panel over a group's
      shown window, its titles drawn by Core Animation, the shown tab
      marked; a click on a tab shows that window without gevrek taking the
      keyboard (the clicks are taken from AppKit's queue each pass). The
      bar goes over to the next window shown, and is put above its window
      again when macOS says the focus changed and once a second after.
- [x] Mouse and wheel bindings, long presses, a held button's repeat,
      and windows moved and resized with Option and the mouse
      (2026-11-22, `keys.rs`). The touchpad's scrolling is not a wheel's
      steps (it is left to the programs).
- [x] Touchpad gestures (2026-11-22, 4a1c077): the fingers from
      MultitouchSupport (private, loaded with dlopen while a gesture
      profile is on; without it there are no gestures and nothing else
      changes) through the engine's recogniser, as on Windows. What every
      system did with a recognised gesture is the shared host's now
      (`Bindings::gesture`, `gesture_registrations`). The framework loads
      and the built-in trackpad starts (`probe touchpad`).
- [x] **person** A swipe tried on the trackpad (2026-11-22): `probe
      touchpad 10` while swiping saw 399 frames, up to five fingers.
- [x] Keys by the keyboard's layout (2026-11-22, 63279bf): a letter is
      the key that writes it in the layout in use, as on Windows (asked of
      the layout with Carbon's UCKeyTranslate, again when another one is
      selected); every other key by its place. The panels' shortcuts the
      same. Checked against German, French and Turkish F without
      selecting them (`probe layout <id>`).
- [x] An app bundle, signed, for the permissions, and started at login
      (`packaging/macos`, 2026-11-22): `build-app.sh` makes Gevrek.app
      (`io.github.ulaas.gevrek`), `install.sh` puts it in ~/Applications;
      `gevrek start wm` opens it, `gevrek enable-autostart` writes a
      LaunchAgent.
- [ ] macOS in the release workflow: built, signed with a Developer ID
      and notarized on a macOS runner (the secrets: the certificate, an
      app-specific password or an API key).
- [x] The third `shellkit` host (2026-11-22, `shellkit/src/mac`):
      panels without a frame on every Space, drawn with Metal, that never
      make the shell the active program; mouse, wheel and keys as egui's
      input; popups, menus and hints; a press outside the shell found from
      the mouse's buttons; woken by other threads through a run loop
      source. The arithmetic of places is shared with the Windows host
      (`shellkit/src/place.rs`). The `spike` and `menu` examples run on
      it unchanged.
- [x] The rest of that host (2026-11-22): the desktop soft behind the
      parts a surface asks for, and drag and drop both ways, with the
      shell's pass made inside AppKit's loop of a drag (b943d78); text
      through the system's input methods (dead keys, the input methods'
      composing) and Command-C, X, V (4def974); screen readers through
      AccessKit (ff2fe68).
- [x] The shell on macOS (2026-11-22): the Windows half
      (`shell/src/windows.rs`) is the half of both guests' systems; the
      window manager keeps the panel's room on macOS as on Windows.
      `gevrek_system` has macOS halves: the applications (the bundles of
      the application folders, started by `open`), the volume (by the
      system's scripting), the keys of what plays and of the sound, the
      screenshot keys, the modifiers held, the ends of the session.
- [x] More of `gevrek_system` on macOS (2026-11-22, 483d4d4): the
      volume through Core Audio and heard of when it changes, the
      clipboard kept for the launcher, the network and the battery, the
      wifi's radio, bluetooth (its radio, devices, connecting; only from
      Gevrek.app, which says why it uses it), the built-in display's
      brightness.
- [x] The rest of the shell's system on macOS that macOS allows (asked
      for: everything technically possible, 2026-11-22):
      - pictures of windows through ScreenCaptureKit, with the Screen
        Recording permission (923de54);
      - the tray: other programs' items of the menu bar, read and pressed
        through the Accessibility API, pictured from the screen with the
        bar's tint taken out (e90daeb);
      - the wifi's networks and joining one, once gevrek may know the
        location (Gevrek.app asks, signed with the location entitlement);
        the programs gevrek starts are their own responsible process, so
        the questions are Gevrek's, not the terminal's (c91529a).
- [ ] What plays: MediaRemote answers only Apple's own programs since
      macOS 15.4. The way round others use (a library of their own loaded
      into the system's perl, which is Apple's) gets past that check
      rather than using an interface, and was not done; it is the
      person's decision whether gevrek should.
- [ ] **person** The location question answered for Gevrek (it was
      asked, not yet answered), then the networks seen in the control
      centre. Whether the shell, now its own responsible process, is
      asked for Accessibility and Screen Recording again beside the
      window manager (as Gevrek, or by its own name), to be seen when it
      runs from the installed bundle.
- [x] Windows in macOS's own full screen (2026-11-22): a window that
      goes into it leaves the layout and its border goes, as for a hidden
      program; when it comes out it joins the layout again and its border
      is put above it once macOS's animation is over. A border once hidden
      did not come back when its window could be seen again (a program
      shown after Command-H too): it does now. Tried with Dictionary
      (`probe fullscreen <window> on|off`).
- [x] Windows on other Spaces: gevrek lists every Space's windows, so a
      switch of Space drops none; a window first seen on another Space is
      adopted when that Space is shown, into the workspace in front.
      Decided by the person (2026-11-22): gevrek's workspaces and macOS's
      Spaces stay independent.

### Kept an eye on

- [ ] What a still gevrek costs (processor, memory, the battery), with
      a number written down at each release and a check that fails when
      it grows.
- [x] The two clippy warnings in the shell, and the others: none are
      left on Windows or Linux (2026-11-25).
- [x] `actions/checkout@v4` ran on a Node the runners are putting away:
      the actions are at their current versions (2026-11-25, not run).
- [ ] `ubuntu-latest` moves to Ubuntu 26.

---

## Found on the way

### The pass of 2026-11-25

Every crate was read for faults. Fixed, each with a test where one can be
written without a desktop (the ones marked * have none):

- **Engine and model.** A display that came back as the only one, in the
  same look at the displays in which the other went, took the place of
  that one's workspaces: its windows were known to nobody. Joining two
  windows as a group, or taking one out, moved every split's share
  after it by one. The window used before was not the current one when
  the current one went (`FocusManager`). Dwindle could not be resized by
  the keys. A floating window with the keyboard was told as without it.
  A window dragged larger in columns or rows turned a quarter got nine
  tenths*. A file that says no schema version lost the desktop's zones.
  The session and the sizes are written whole or not at all*. A rule's
  yes or no is read one way. An item renamed over one before it kept the
  wrong place. Geometry from outside no longer runs past what a number
  holds*.
- **Host, server, command line.** `gevrek stop` on Linux and macOS
  could end a program that is not gevrek (a process number left in a
  file), and a number of 0 was the whole group*. A startup program
  switched on by a reload was not started; one that is off, asked to
  start again, wrote a warning with every pass. A program that could
  not be stopped was let go of. `start --foreground` gave 0 whatever
  came of it*. An update whose stop failed left the desktop stopped*. A
  configuration handed over waited five seconds where the server waits
  ten*; one handed to a line that is down is said to have failed*. The
  Windows window manager ended in the middle of a pass when its server
  could not listen, and the macOS one went on with nobody able to speak
  to it*. The graph's log grew without end*.
- **Windows.** Windows that opened while gevrek was paused stayed
  cloaked*. After a fault in a pass only hidden windows were given back,
  not the dimmed, cut, fullscreen or title-less ones*. Fullscreen made
  by gevrek was never undone*. A style was changed on a window that
  does not answer*. A black border was drawn red*. A border or a tab
  strip whose placing failed stayed on the screen*. The wallpaper's
  object was let go of after the thread said it was done*.
- **Linux.** Any program could ask for the lock of a locked session and
  unlock it*. A program that named its icon as a path had that file
  deleted when its window went*. One window could be captured while the
  session was locked*. A key let go while locked was remembered as
  held*. The display manager waited without an end for a
  configuration that was a pipe, and kept the password for as long as
  the session ran*. An X11 resize began without a button held*. The
  safe and the previous session wanted a second polkit agent*.
  `scripts/wsl-cargo.sh` said all was well when cargo failed.
- **Shell and system.** A notification that waited its turn went
  unseen when the one before it was dismissed. A notification's picture
  was read from whatever its sender named, a pipe too*. A folder that
  is there and cannot be listed was gone to again and again (Linux)*.
  What was thrown away through a link to its folder could not be put
  back. A program's name was cut by the length of its small letters. A
  dragged list of the shell's items was read past its end. A text on
  the clipboard that does not end was read on*. The ids of the portal's
  notifications were all forgotten at once, the shown ones too*. gevrek
  studio, opened from the panel, stayed in the process table*.
- **Studio.** Keys went on being taken after another page was shown.
  Delete, pressed to be a node's key, took the node away*. A trial
  whose time was up sent the old configuration into a line that was
  down*. A conflict left the putting-back for a fault armed*. The raw
  page read a text otherwise than a file is read*.
- **macOS** (type-checked, not run). A window that came by an
  observer's word was asked with the system's six seconds. No list of
  windows was taken for every window gone. A parked window's place was
  forgotten before it was back.
- **Tidy.** Nineteen functions nothing calls are gone; `thiserror` and
  two more dependencies no crate uses; the workspace's unused `authors`.

A second round the same day took most of what the first had left:

- **Engine.** An edge or an axis is resized as it is seen in a layout
  that is mirrored or turned, and a turned layout keeps its gaps.
- **Server, updates, sockets.** A subscriber that leaves is noticed at
  once*. A graph that is turned away is not the working graph. One
  update at a time has the update's folder. One window manager at a
  time looks at an instance's socket and takes it*. Stopping, pausing
  and going on while locked are meant (the way out of a session whose
  lock screen ended), and say so.
- **Linux.** A window's frame goes no further than what it draws*. A
  made-up pointer that goes lets go of its buttons, and its notches do
  not run over*. No frame is owed to a display that is off*. Xwayland
  that ended is still answered for*. A lock screen that ended by a
  fault is started again at the next key or button*; who may lock and
  unlock is tried by real clients (`tests/lock.rs`). The source RPM
  and the Arch package carry the display manager's SELinux module, and
  the RPM installs it*.
- **Windows.** A second window manager of an instance is turned away
  whatever address it listens on*. The helper waits again after a
  program that may not ask*, is asked for once more when it is lost*,
  uncloaks what it cloaked, and is spoken to only where gevrek's own
  helper serves the pipe*. A delete that was for good puts no older
  file back.
- **Zones' files (Linux, macOS).** A rename is made only where nothing
  has the name, in one step (`rename_new`), for moves, renames and the
  trash. One file that cannot be taken away where it was is not left
  twice.
- **Shell.** A notification's picture is told by the picture, not by
  where it lay*. A secret that is cleared brings no older text back*,
  and the clipboard's history is written whole or not at all*.
- **Studio.** What was changed before the window manager answered is
  kept as a draft*; a text that is not read as a configuration is not
  replaced*.
- **macOS** (type-checked, not run). A window that could not be watched
  is asked again; a program that does not answer is not asked six
  times over; a frame is read only from a value that is one.

A third round took the rest, so that nothing the pass found is left
without a change (the ones marked * have no test: they are written by
the code alone, and are in the list of section 2 to be seen):

- **Linux.** A capture session says the new size when what it shows
  has another*. The display manager starts its workers from one thread
  that lives as long as it does, so that none is told to end with the
  thread of a connection*, and writes nothing for the greeter through a
  folder that is a link*.
- **Windows.** The table of keys is told to the elevated helper from a
  thread of its own, not from the one that hears the keys*. The task's
  XML is written beside the helper, where only an administrator
  writes, and that folder is the one the system names*. A mouse button
  that is up is not held, whoever heard it come up*. A border that was
  made whole again is cut again*. A folder that cannot be listed is
  gone to once more, not without end.
- **Studio.** With the revision unknown after the line came back,
  nothing is applied until it is known*; a trial's way back that is
  refused is sent once more*.
- **Updates.** The package is the one of the processor gevrek was
  built for.

What the pass could not do on this machine, and is the person's:

- [ ] **person, M** Run anything on macOS. Everything of macOS in this
      pass is type-checked and linted for its target, and no more.
- [ ] **person** Run the workflows: they need a push.

- [x] **any** CI: the test
      `zones::draw::...everything_of_the_desktop_is_painted...` failed now
      and then, on Linux and on Windows (2026-11-22). The zones' clock ran
      by itself in tests: the hiding fade (0.18 s) was over before a slow
      runner had drawn the test's three frames. A test now stops the clock
      (`Zones::stop_the_clock`; only `later` moves it). With a sleep of
      300 ms in each frame it failed at the same line with the clock
      running and passes stopped (2026-11-22).

- [x] **any** `gevrek-system` did not build for macOS, and with it
      `gevrek-shell` (the zones take the Linux half there, whose `notify`
      was Linux's only). Decided: FSEvents. It builds, and
      `scripts/check-macos.sh` passes (2026-11-22). Three tests of the
      Linux zones are Linux's own and left out on macOS (inotify's paired
      renames, Linux's user id).

## Log

| When | Where | What | State |
|---|---|---|---|
| 2026-11-17 | Windows 11, two displays | The roadmap written anew from the archive and from the code. Before it, the same day: a new window comes in as a picture (`61faf40`); the control centre kept when it loses the keyboard (`0cfcd38`); the open items held against the code; CI looked at | The shell's and the backend's tests pass; clippy on the shell. Nothing seen on the desktop. Not pushed |
| 2026-11-17, later | Windows 11, two displays | Workspaces, from the configuration to the panel and the studio, after an audit of all three (asked for: the kept ones and the new ones, their names, how they are edited). A workspace has a name (what keys and rules know it by) and a title (what is shown); a display has its own entry for a workspace before the one for every display; `also_called` is gone (schema 25, old files converted). Workspaces of the moment take the number of their place, are dressed as the display's others, go when left empty. A reload renames, takes out and reorders. The state says title, configured, persistent. The panel names a tab's workspace by display and name, a rename is a title in the configuration, what the window manager refuses is said. The studio's workspaces page is new (`studio/src/workspaces.rs`): display by display, with what runs | 1066 tests pass on Windows; clippy nothing new; model, engine, studio type-check for Linux. The Linux shell (`shell/src/main.rs`) was changed and not built. Not seen on the desktop. Not committed |
| 2026-11-17, night | Windows 11, two displays | What a change of workspace costs the graphics card (asked for). A border that only moves is not cut and painted anew at every frame. Two settings, both off, on the animations page: `workspace_slides_as_pictures` (the windows stay, unseen, and DWM pictures of them slide, as for a window coming in) and `workspace_slides_without_shadows` (shadow and glow left out while they slide) | 1068 tests pass (two new, of the pictures and of the windows moved themselves); clippy nothing new. Not measured, not seen on the desktop. The studio's picture of the animations page is to be drawn anew on Linux. Not committed |
| 2026-11-17, late night | Windows 11, two displays | The studio set one thing in one place (asked for): where a card of the page's own sets something, the form below no longer does. The panel's modules (dragged), the keyboard's layouts, the zones' places (the drag editor has a zone's gap, a layout's name and a display's own layout now), the displays (each display's card has how it is turned, variable refresh and primary; displays written down and not attached are listed under it; the list of outputs is gone), the rules' order (dragged; no arrows in the list). The older list of scales by name is shown only where a file has something in it | 1068 tests pass; clippy nothing new. The studio's pictures of panel, zones, displays, input and rules are to be drawn anew on Linux. Not committed |
| 2026-11-17, towards morning | Windows 11, two displays | Three things asked for. (1) A renamed workspace was on its tab only after a press elsewhere: the window manager published its state on window events and once a second, not after a configuration was applied or a command run, so the panel that asked at once got the old. Now it is published again at the end of every pass in which something was told or done, and before an Apply is answered; the shell asks for it on `Applied`. (2) The overview of the windows (`toggle_expose`) on Windows shows live pictures of the windows, small, over a dark backdrop with titles, mouse and keys (`dwm/overview.rs`, `expose::arrange`); the windows are not resized any more. (3) Hot corners (`desktop.hot_corners`, `engine/hot_corners.rs`, a card of its own in the studio); the first is the top left, for the overview. The installer registers, updates and removes `gevrek-elevated` (the day's earlier work). 1,078 tests and more pass. Not seen on the desktop. Not committed | Linux: the corners are not watched (needs a timer in the compositor's pointer path), the overview there still arranges the windows themselves, and the shell does not ask for the state on `Applied`; the studio's pictures of the gestures page |
| 2026-11-17, midday | Windows 11, two displays | The roadmap is in the repository, with section 2c for the next time on Linux (`29f7ee5`); the installer and the overview, hot corners and state committed (`e25c950`, `d3cd849`). The Windows CI test: the runner is an administrator and `notices` says so; the test asks only for what it is about now. Passes here; not run as an administrator. Not committed | The Windows job on a push; the Linux job |
| 2026-11-17, afternoon (1) | Windows 11, two displays | The three of section 2b that could be done here: a workspace is chosen from a list wherever its name was typed; the rules are held against the workspaces after a reload has rearranged them (the roadmap had the cause wrong: titles never mattered to a rule); the studio offers no Apply while the window manager is away. And, reported meanwhile, borders around nothing: a window that is hidden, or asked to be, gets no border (`kept_out_of_sight`), and the overview's pictures none. 1,084 tests pass. Not seen on the desktop. Not committed | The borders, seen; the rest of 2b |
| 2026-11-17, afternoon (2) | Windows 11, two displays | The rest of section 2b that can be done on Windows: the overview and the switcher by name and title; a saved session finds its display where the handle is another; the panel display by display on Windows, with no room kept where a display has none; the slider ranges within the model's limits; a new entry's terminal; file mode and a file changed elsewhere. 1,089 tests pass. Not seen on the desktop. Not committed | What is left of 2b is Linux's or the person's. Sections 3 (the tutorial) and 5 are not begun |
| 2026-11-17, afternoon (3) | Windows 11, two displays, and WSL (Fedora 44) | Found: the Linux side builds and its tests run in WSL as the distribution's own user (the note that it could not had looked as root only). So, from Windows: the Linux build and tests (all pass but the pictures); the renaming on Linux; hot corners in the compositor, with a test; the overview there placed as on Windows; the workspace protocol's title. The cause of the Linux CI job: the studio's pictures were drawn in the system's own font; the test uses gevrek's own now, and the pictures are to be drawn anew on a Linux that can (WSL's graphics fault in that test; the pictures here are as they were). Reported and mended: a window dragged to the display beside goes to the display the pointer is on. Windows: 1,090 tests pass. Not seen on a desktop. Not committed | The pictures, on Linux; what is the person's to see |
| 2026-11-17, afternoon (4) | Windows 11 and WSL (Fedora 44) | The studio's pictures after all: the fault in the picture test was WSL's graphics, not the test (a larger stack alone did not help; the processor's Vulkan driver alone did). Every page drawn anew in gevrek's own letters, looked at, and the test passed three times running there. Linux: all tests pass. Not committed | Whether CI and the person's Linux draw the same |
| 2026-11-17, afternoon (5) | Windows 11 and WSL (Fedora 44) | The overview of the windows on Linux made as on Windows (a window chosen ends it; titles, a mark, the arrow keys, Enter, Escape), tested with real clients and looked at in a picture drawn by the compositor without a display. That picture found a fault: a hot corner ran at the start of a session, the pointer lying at the top left before it was ever moved; a corner waits now until the pointer has been elsewhere. Windows: 1,091 tests pass; Linux: all, the pictures among them. Not seen on a desktop. Not committed | The person's part on both systems; the job on a push |
| 2026-11-17, afternoon (6) | Windows 11 and WSL (Fedora 44) | Begun on the tutorial's first stage, with what can be tested without a desktop: it runs on the display the person is on; the person's own windows leave its workspace with it; practice windows say how they stand. Windows: 1,094 tests pass. Not seen on a desktop. Not committed | The rest of stage 1 is drawn (the ring, the card's place, the welcome) and wants a desktop to be seen on |
| 2026-11-17, afternoon (7) | Windows 11 and WSL (Fedora 44) | More of the tutorial's first stage: the card's room kept free of tiles; the main key chosen in the welcome, with every binding following it. Windows: 1,096 tests pass. Not seen on a desktop. Not committed | Of stage 1: the ring, the welcome folding into the card, the lessons played by a check |
| 2026-11-17, afternoon (8) | Windows 11 and WSL (Fedora 44) | The tutorial's ring on Windows: around the window with the keyboard, all the windows, or the panel, as four thin surfaces. Windows: 1,097 tests pass. Not seen on a desktop. Not committed | The ring on Linux and around the shell's own surfaces; the welcome folding into the card; the lessons played by a check |
| 2026-11-17, evening | Windows 11 and WSL (Fedora 44) | Committed: the tree as it stood (`ac8f0c5`); gevrek starts with the person's look, without what belongs to one machine (`d08ca9a`); the studio's pages with a list of their own no longer scroll as well, and no page is wider than the window (`f6dc4c4`; looked at in pictures). CI seen for `91859eb`: Windows green to its end, Linux at the pictures only, the dependencies at `chrono`. The plan of the pass for faults and speed written (section 2d) | Section 2d, step 0 first. To push; `chrono` to decide; the studio's changes to be seen in its window |
| 2026-11-17, late evening | Windows 11 | The first benches of the engine (section 2d, step 0), as medians on the person's machine, release build. A layout of 50 windows: 17 to 204 ns by layout (dwindle the slowest). 50 windows opened one after another on the desktop without a windowing system: 0.46 ms; one more beside 49, opened and closed: 19 µs; a workspace changed with 25 windows on each: 6.4 µs. 20 rules against 50 windows: 9.96 ms. The overview's arrangement of 30 windows: 18 µs. The starting configuration read 159 µs, written 70 µs, checked 284 µs; a reload's plan 0.52 ms with nothing changed, 0.64 ms with one setting. Found: the rules' regular expressions are made anew at every match | The rules' expressions made once (step 2). The bench of the published state in the backends; the time of a pass of the runtime; the numbers of a still desktop |
| 2026-11-17, night (2) | Windows 11 and WSL (Fedora 44) | The rules' regular expressions are made once and kept. 20 rules against 50 windows: 9.96 ms before, 0.95 ms after. Windows: every test passes; Linux: the engine, the compositor and the server pass | Step 0: the bench of the published state, the time of a pass, a still desktop's numbers |
| 2026-11-17, night (3) | Windows 11 | Step 0 further: the published state is in the engine's bench (34 µs for 9 workspaces and 30 windows without a windowing system); a pass of the Windows runtime is timed in six parts and told at the log's debug level (`RUST_LOG=info,gevrek::passes=debug`). Windows: the backend's 207 tests pass. Not run on the desktop | The person runs gevrek with that for a while and brings the lines; the still desktop's numbers; the state's way out through the server |
| 2026-11-17, night (4) | Windows 11 and WSL (Fedora 44) | The state's way out timed (`cargo bench -p gevrek-server`): 24 µs in the server and 163 µs at the asking end for 30 windows, most of it the JSON there. The harness of the benches is one file for both. Windows: every test passes; Linux: the engine and the server | Step 0 is done but for what wants the desktop (the passes' lines, a still desktop's numbers) and CI. Step 1 next: files from outside that are cut short or nonsense |
| 2026-11-17, night (5) | Windows 11 and WSL (Fedora 44) | Step 1 begun: configurations and saved sessions spoilt by the thousand and taken through the engine; one fault found and mended (a number of the graph as large as numbers get was counted past). Windows: every test passes; Linux: the model, the engine, the server and the compositor | The other files read from outside; the engine driven by random commands |
| 2026-11-17, night (6) | Windows 11 and WSL (Fedora 44) | The engine driven by runs of chance, 7,200 of them clean in the end. One fault found and mended: Toggle floating put a window that is on a workspace not shown on the shown one as well. One thing found for the person to decide: tiles of no size where many windows dwindle. Windows: every test passes; Linux: the model, the engine, the server and the compositor | The runs with two displays, groups and scratchpads; the other files from outside; the notes of the kind TODO |
| 2026-11-17, night (7) | Windows 11 and WSL (Fedora 44) | The runs of chance with two displays, the rules and the scratchpads, and with nothing of a workspace that is not shown in sight. Two more faults found and mended: a layout command brought a hidden workspace's window into sight; move-and-follow changed the wrong display. Noted for later: the scale toggles on the studio's Displays page (reported). Windows and Linux: every test passes (the studio's not run on Linux this time) | The person sees the two on the desktop; the other files from outside; the notes of the kind TODO; the scale toggles |
| 2026-11-17, night (8) | Windows 11 | Looked into: the scale toggles of the studio's Displays page write the configuration, and the Windows backend does not apply a scale (by design so far); a decision for the person. The notes of the kind TODO: there are none (the count was of a variable's name) | The person decides about the scale on Windows; the other files from outside |
| 2026-11-17, night (9) | Windows 11 and WSL (Fedora 44) | A display's scale is set on Windows as the Settings window sets it (decided by the person), and the studio shows the scale a display has. Asked on this machine without setting anything: the three displays answer rightly. Windows: every test passes; Linux: the studio's pass | The person tries the toggles on the Displays page; the other files from outside |
| 2026-11-17, night (10) | Windows 11 and WSL (Fedora 44) | The panel is as high as its display's scale makes it (reported: at another scale what it shows was cut off). Windows and Linux: every test passes (the studio's not run on Linux this time) | The person sees the panel at several scales, and tries the scale toggles |
| 2026-11-18 | Windows 11 and WSL (Fedora 44) | The person: the scale changes work. Asked for: the layouts follow the scale. The gaps, the border, the tab strip and the unseen borders' room are as many pixels as the display's scale makes them (Windows). Windows and Linux: every test passes (the studio's not run on Linux) | The person sees a layout at two scales and through a change of scale |
| 2026-11-18 (2) | Windows 11 and WSL (Fedora 44) | Commands as JSON, the journal of hidden windows and two of the shell's files spoilt: seventeen sums that were counted past mended (fifteen in the commands, two in the shell). Windows and Linux: every test passes (the studio's not run on Linux) | The helper and the display manager behind a stand-in; the runs of chance with selectors and a reload |
| 2026-11-18 (3) | Windows 11 and WSL (Fedora 44) | The runs of chance with nearly every command, the configuration read anew and the graph's keys: 1,440 runs clean, nothing found. Windows and Linux: every test passes (the studio's not run on Linux) | The studio's top bar at 760 wide; "Where things stand" brought up to date |
| 2026-11-18 (4) | Windows 11 and WSL (Fedora 44) | The studio's top bar has two rows in a narrow window. Windows: every test passes; Linux: the studio's pass, its pictures as they were | Nothing that does not want the desktop or a decision: the helper and the display manager behind a stand-in is the next piece of work of that kind |
| 2026-11-18 (5) | Windows 11 and WSL (Fedora 44) | The elevated helper and the display manager's service behind stand-ins for the system, and what they are asked spoilt: nothing found. Windows and Linux: every test passes | Nothing that does not want the desktop or a decision |
| 2026-11-18 (6) | Windows 11 and WSL (Fedora 44) | `chrono` gives way to `jiff`; the check of the dependencies passes. Windows and Linux: every test passes, the studio's too | The clocks and the night light seen on a desktop; CI on what is pushed |
| 2026-11-18 (7) | Windows 11 and WSL (Fedora 44) | The runs of chance with a graph nobody drew: a node asked for its own value and loops that run each other ended the program, a loop counted past the largest number, a scratchpad's window given a place stayed the scratchpad's, one that was restored was tiled, a window that had gone was given a place by a graph; all mended, 2,400 runs clean. The shell's smaller files spoilt: nothing found. Windows and Linux: every test passes (the studio's not run on Linux) | What a scratchpad's window is after "toggle floating", looked at by the person |
| 2026-11-18 (8) | Windows 11 and WSL (Fedora 44) | Decided by the person and done: a tile is not made smaller than 120 pixels in BSP, dwindle and the spiral (the rest share the last room in rows); the nodes of one key are run in their order; a scratchpad's window given a place stays an ordinary one. Windows and Linux: every test passes (the studio's not run on Linux), 480 runs of chance clean | Ten windows and more seen on a desktop |
| 2026-11-18 (9) | Windows 11 and WSL (Fedora 44) | Asked for by the person: what is playing is read on Windows (the sessions players have with the system; it was never asked there, and the control centre said nothing played), with the piece's picture; the control centre drawn anew (a card for what plays, broad bars for the sound and the brightness, a way to the studio); a panel module that opens it (`control_centre`); the other button on a window of the taskbar brings it to the middle of that display, floating, and puts it back as it was (`toggle_window_centered`). Windows and Linux: every test passes, the studio's too | All of it seen on a desktop: Spotify and a browser playing, the new look, the module, the taskbar's other button |
| 2026-11-18 (10) | Windows 11 and WSL (Fedora 44) | Reported by the person: windows became very small and went out of sight when workspaces were changed. Found in the session: a minimized window (Windows Terminal minimizes itself while it is hidden) was handed the place of its mark, far off every display, as the place to come to and to go back to. Mended in the engine and the Windows backend; a floating window on no display is found; windows minimized behind gevrek's back come back with their workspace. The tests keep out of the person's own folders (they had left 200 copies among the backups). The panel's modules drawn anew as cells that stand along a panel or down a side; down a side the ends keep apart and the clock is in its order. Windows and Linux: every test passes, the studio's too | The panel in all four places and three styles, and the windows after a change of workspace, seen on a desktop. The display that went dark: nothing in any log; the time of the next one |
| 2026-11-22 | macOS 27.2, Apple silicon, one display | The macOS backend begun (asked for). A spike first, then `crates/backend-macos`: the Accessibility, Core Foundation and Core Graphics calls declared by hand, AppKit through the objc2 crates egui already brings (no new crate in the lockfile); windows matched to their elements by the window server's id, moved and sized (a window that cannot be resized is still placed), closed; displays with the room the menu bar and the Dock leave and their scale; events from each program and window, a window that went still named; where a hidden window waits, measured (`park.rs`). Found: `gevrek-system` does not build for macOS (Found on the way). A Finder window of the person's was closed by mistake while trying an AppleScript | 13 tests of the new crate pass; it checks for Windows and Linux too; clippy clean. Tried on the desktop with the probe, on a Calculator window it started. Not committed |
| 2026-11-22, later | macOS 27.2 (Windows and Linux type-checked from it) | The host written once for every backend (asked for: the better design before the macOS backend). `gevrek_host::wm`: `HostState` (what only the host owns) in each backend, and `WmHost`, which a backend implements by lending its configuration, core, desktop and host state and answering hooks; configuration requests and the file, commands, reload, binding ids, the published state are its own, once. The Windows runtime and the Wayland host carry it out: `host.rs` 1318 to 537 lines, `runtime/mod.rs` 3360 to 2676. Where the two had drifted they now do the same: Windows settles after every command and a manual trigger, and clears the graph's values before a dispatch; Linux tells `workspace.destroyed` when a reload hands windows over, publishes a new submap at once, and publishes the state after every event told (as Windows did) | 14 tests of the shared host on the engine's fake desktop pass here (one is Windows' refused-revision test, now on the shared code); the Windows and Linux backends and their tests type-check and are clippy-clean in what changed. Their own tests were NOT run (no Windows or Linux here). Not committed |
| 2026-11-22, evening | macOS 27.2, one display (Windows and Linux type-checked) | The macOS backend made a window manager (asked for): `MacDesktop` (`Desktop`, `WindowQueries`), `MacRuntime` (the loop on the main thread, `WmHost`), programs from NSWorkspace, the journal of parked windows, the keyboard by an event tap; `gevrek-wm` runs it on macOS. Decided by the person: Win is Option on a Mac. Alt was to be Option+Control; a test found that on the same keys as Win+Control in ten pairs of standard bindings, so it is Control+Option+Command. Tried on the desktop with Activity Monitor and Dictionary only: tiled, a window's smallest size learned and room made for it, focus by command and by key, swap and workspaces by key, parked and back, `kill -9` while parked and put back at the next start, quit puts parked windows back | 21 tests of the crate pass, and every check; clippy clean in what is new. Not tried with the whole desktop managed, on two displays, or with many programs. Not committed |
| 2026-11-22, night | macOS 27.2 | gevrek as a macOS application (asked for): `packaging/macos` (Info.plist, `build-app.sh` that builds and signs Gevrek.app with the keychain's Developer ID or Apple Development identity and the hardened runtime, `install.sh` to ~/Applications with `gevrek` in ~/.local/bin, `uninstall.sh`, README). The command line finds the window manager as `gevrek-wm` on macOS, starts it through `open` from the bundle (so the bundle, not the terminal, is responsible for it), passes `GEVREK_INSTANCE`, `GEVREK_MACOS_ONLY` and `RUST_LOG` on, and says why it did not start; `enable-autostart` writes a LaunchAgent. README: macOS in where it runs and how to build | Built and signed here (Developer ID, team N6FA79GV97; the designated requirement is the bundle id and the team). Started through the bundle, gevrek was refused the Accessibility permission the terminal has: the bundle is the one asked. Allowed by the person, then rebuilt and signed again: the new build ran without being asked again, managed Dictionary, took Option+2 and Option+1 by its event tap, and stopped by `gevrek stop wm`. `install.sh` and the LaunchAgent not run. Not committed |
| 2026-11-22, late night | macOS 27.2 | Borders on macOS (`borders.rs`): borderless, clear, click-through windows of gevrek's own with a ring drawn by their layer, ordered above their window by its window server id; `MacDesktop` draws, moves, recolours, hides and removes them as the engine asks, and moves them with a window moved by another. The command line's Windows autostart helpers allowed unused on macOS | Every check passes; clippy clean. Seen in the window server's list with Activity Monitor and Dictionary: each border 3 points around its window and directly above it (a window of another program between them stays between), gone on another workspace and back, following a window another program moved, gone on quit. The colours (focused and not) not seen: no screen recording here. Not committed |
| 2026-11-22, after midnight | macOS 27.2 | Stackbars on macOS (asked for): `stackbars.rs`, non-activating panels with text layers; `MacDesktop::sync_stackbars`; the runtime finishes AppKit's launch and takes clicks from its queue, a click on a tab going to `focus_stackbar_tab`. Borders and bars are put above their window again when the focus changes and every second (a program raises its window after it is asked). The probe can click | Every check passes, 23 tests of the crate; clippy clean. Seen with Activity Monitor and Dictionary grouped: a bar of 30 points over the group, real clicks on each tab show that window and bring its program to the front, the bar kept (the same window) from tab to tab, bar and border above the window shown after each switch. The titles and colours not seen (no screen recording here). Not committed |
| 2026-11-22, early morning | macOS 27.2 | Decided by the person: commit on a branch and let CI run the other systems' tests (PR #1, draft); FSEvents for the zones on macOS (`gevrek-system` and the shell build there, `check-macos.sh` passes). The rest of input on macOS: mouse buttons and wheel steps as bindings, long presses, a held button's repeat (timed each pass, as on Linux), windows moved and resized with the drag modifier (Option; `GEVREK_MACOS_DRAG_MODIFIER`), the drag started only on a window gevrek manages | 28 tests of the crate; every check passes, clippy clean. Seen with Activity Monitor and Dictionary: Option+wheel to the next workspace and back, a tiled window dragged and put back on drop (as the engine has it), a floating one moved by (-200, 60) and resized by (-100, -70) from its corner exactly. Long presses only in tests. Not committed at the time of writing |
| 2026-11-22, morning | macOS 27.2 | The third `shellkit` host, for macOS (`mac/`): `Control`, `Sender`, `Shell` and `run` in the Windows host's shape, AppKit panels (a subclass that may take the keyboard though it has no frame), wgpu on Metal, AppKit's events as egui's input, a run loop source for other threads. The arithmetic of places moved out of the Windows host to be shared (`place.rs`). The examples `spike` and `menu` build on macOS | Every check passes; shellkit's 60 tests pass on macOS and it checks for Windows and Linux; clippy clean. Seen: the spike's surface 40 points high at the top of the free room, 328 frames in 3 s on Metal; in `menu` a real click on the panel's button opened the menu under it, a click on its second entry chose it and closed it, a click on Terminal's title bar closed an open menu, a message from another thread ended the shell; 20 frames in 7 s (nothing drawn while still). Typing not tried. Not committed at the time of writing |
| 2026-11-22, late morning | macOS 27.2 | The shell on macOS (asked for: on with the plan). The Windows half of the shell is the half of both guests' systems; `gevrek_system` got macOS halves (applications, volume, media and screenshot keys, modifiers, the ends of the session; the rest says it is not there); the macOS host got `reach` and `is_open` as on Windows; the window manager keeps the panel's room (`set_bar`); Gevrek.app carries gevrek-shell | Every check passes; the shell's 232 tests and gevrek-system's 139 pass on macOS; Windows and Linux check. Seen with the window manager on Dictionary: the panel in the room kept for it (36 points under the menu bar, Dictionary below it); `gevrek shell launcher` opened the launcher, typed keys narrowed it (Calendar, then Calculator), Return started Calculator from /System/Applications. Not seen: the panel's look, its buttons, notifications, the power menu. Not committed at the time of writing |
| 2026-11-22, evening | macOS 27.2 (Windows and Linux type-checked) | Everything open for macOS but the release workflow (asked for). `gevrek_system`: Core Audio's volume and its changes, the clipboard, network, battery, wifi's radio, bluetooth, brightness (483d4d4). The macOS surface host: the desktop soft behind a surface, drag and drop both ways with the shell's pass made inside AppKit's drag loop (b943d78); input methods and the clipboard (4def974); screen readers (ff2fe68). Double and dashed borders (243980d). Letters by the keyboard's layout, for bindings and the panels (63279bf). Touchpad gestures through MultitouchSupport, and the handling of a recognised gesture moved into the shared host, which Windows uses too (4a1c077). What macOS keeps from the shell written down with its reasons | Every check passes; Windows and Linux check. Tried: the system's numbers printed; a volume change heard once; soft parts made; the example's file dragged onto itself as a move, a copy and with Option a copy; a, Option-E E, s, Backspace, d typed "aéd"; copied and pasted twice "xyzxyz" (the pasteboard put back); the label and the field seen by the AX tree; each border style captured; German, French, Turkish F layouts read. Not tried: a swipe (nobody at the trackpad), bluetooth (only from the bundle), a real input method's list. Committed and pushed |
| 2026-11-22, night | macOS 27.2 (Windows and Linux type-checked) | Asked for: what is left of macOS, and every shell feature technically possible; Spaces left independent (decided). Windows in macOS's own full screen leave the layout and come back, and a border once hidden comes back (6305865); the border's corner measured, 17 points for every kind of window (e9ffa64); the zones' flaky test found and fixed (70ab792). Pictures of windows (923de54), the tray from the menu bar's items (e90daeb), the wifi's networks once the location may be known, and programs started as their own responsible process; build-app.sh fixed (c91529a). What plays not done: the only way is to get past Apple's check, left to the person | Every check passes; CI green to e9ffa64. Tried: full screen in and out with Dictionary; corners measured in captures; a Calculator window pictured; four menu bar items found, pictured, one pressed (its menu opened and was closed); the location question posted for Gevrek (locationd), not answered; the shell responsible for itself. The touchpad tried by the person: 399 frames, five fingers. Committed and pushed |
