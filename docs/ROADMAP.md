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

## Where things stand (2026-11-17)

- Version 0.6.0 is public, with installers for Windows and Linux and
  updates on Windows. `main` is pushed up to `91859eb` and three
  commits ahead of it (`ac8f0c5`, `d08ca9a`, `f6dc4c4`); nothing is
  left uncommitted but this file. The person is building and trying
  it on Windows.
- What comes next is section 2d: a pass for faults and for speed, in
  which nothing new is built and nothing that works may stop working.
- Everything since 2026-11-17 was written on Windows. What it left to
  do on Linux is gathered in section 2c: start there.
- The window manager, the shell, the studio, the lock screen and the
  display manager work on both systems. Linux was last tested by the
  person on 2026-11-17: all of it passes.
- CI on `91859eb` (run 44074103140): the Windows job is green to its
  end, tests and clippy. The Linux job fails at the studio's picture
  test alone (its remedy is committed in `ac8f0c5`, not pushed). The check of
  the dependencies fails since that day on `chrono` (section 1).
- The check scripts (`tests/desktop`, `tests/nested`, `tests/vm`,
  `tests/wayland`) are not in this repository: the public one leaves
  them out. On this machine only the crates' own tests can be run.
- The tutorial has its tour on both systems, and on Windows its ring
  (not committed, not seen on a desktop). The ring on Linux, the
  chapters and what belongs around it are not written.

Before a session, on the system you are on:

| | Windows | Linux |
|---|---|---|
| Everything quick | `cargo xtask check` | `cargo xtask check` |
| The other system, type-checked | `pwsh scripts/check-linux.ps1` | `scripts/check-windows.sh` |

(On the Windows machine the Linux side builds and its tests run in WSL,
in the distribution `FedoraLinux-44`, as its own user and not as root:

    wsl -d FedoraLinux-44 -- bash -lc '. ~/.cargo/env; cd /mnt/e/source/private/gevrek && CARGO_TARGET_DIR=$HOME/gevrek-target cargo test --workspace --exclude gevrek-win32'

What cannot be had there is the desktop itself. The studio's picture
test ends with a fault in WSL's own graphics (its Direct3D driver, and
OpenGL) unless everything is drawn on the processor; all three of these
are needed:

    LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json cargo test -p gevrek-studio --lib

and with `GEVREK_BLESS=1` before it the pictures are drawn anew.)

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
- [ ] **person, any** The check of the dependencies (cargo-deny) fails
      since 2026-11-17, and passed the day before with the same
      `Cargo.lock`: RUSTSEC-2026-0543 calls `chrono` unmaintained, in
      every version, and `deny.toml` lets a crate nobody maintains fail
      where gevrek uses it directly (`gevrek`, `gevrek-engine` and
      more). To decide: `chrono` gives way to `jiff` or `time` (the
      advisory's two), or it is named under `ignore` in `deny.toml`
      with the reason beside it until then.
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
      `chrono` decided (section 1). From then on a red run stops the
      pass until it is green again.
- [ ] **any** The numbers as they are now, written down in the log, on
      the person's machine: processor and memory of a still gevrek
      (window manager, shell) over five minutes; handles and GDI objects
      on Windows; the time from start to the first layout; the size of
      the programs.
- [ ] **any** Benches (there are none yet) for what runs most: a layout
      of 1, 10 and 50 windows in each layout; the state as it is
      published and written to JSON; a reload of a configuration as
      large as the person's (95 kB, 116 bindings); the rules held
      against 50 windows; the overview's arrangement.
- [ ] **W** The time of a pass of the runtime (an event taken in to the
      windows placed) measured where it runs, at the log's debug level:
      the slowest passes are known by name before one is touched.
- [ ] **any** The large files have their behaviour held by tests before
      they are opened: `window_manager.rs` (8,199 lines),
      `settings.rs` (5,502), the Linux shell's `main.rs` (4,889),
      `server/src/lib.rs` (3,591). Where a part has none, tests that
      say what it does now come first.

**Step 1: faults**

- [ ] **person, W** The two that were reported and mended without being
      seen: borders around nothing; a window dragged to the other
      display (section 2c). Not closed until seen for a day.
- [ ] **any** The 64 notes of the kind TODO and FIXME in the code read,
      each one: done, made an item here, or taken out as no longer so.
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
- [ ] **any** No file from outside ends the program: a configuration,
      a saved session, the hidden journal, a message on the line, cut
      short or filled with nonsense (tested with made-up input by the
      thousand).
- [ ] **any** The engine driven by long random runs of commands and
      events, with what must always hold checked after each: every
      window on one workspace, the focus on a window that is there,
      every tile within its display, none of no size, no two tiles on
      each other.
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
- [ ] **any** The studio's top bar at the smallest window (760 wide):
      the file and the state lie under the search.

**Step 2: speed and what it costs**

By what step 0 measured; the order below is a guess until then.

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
      said a hundred times a second.

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
- [ ] **any** Every lesson played by a check, each step done by its own
      command and seen done: headless on Linux, the tour on the Windows
      desktop. Waits for the check scripts (section 1).

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
- [ ] **person, any** A Mac, for the macOS backend (section 5).

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

- [ ] The backend: the desktop's windows through the Accessibility API,
      the displays, the keys. The shared crates type-check for it
      (`scripts/check-macos.sh`). Waits for a machine.

### Kept an eye on

- [ ] What a still gevrek costs (processor, memory, the battery), with
      a number written down at each release and a check that fails when
      it grows.
- [ ] The two clippy warnings in the shell (`control.rs:243`, a wide
      type; `tutoring.rs:47`, variants of very different sizes).
- [ ] `actions/checkout@v4` runs on a Node the runners are putting
      away; `ubuntu-latest` moves to Ubuntu 26.

---

## Found on the way

(Nothing yet.)

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
