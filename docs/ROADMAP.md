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
  updates on Windows. `main` is five commits ahead of what is pushed
  (`61faf40`, `0cfcd38`, `8314a98`, `8d93932`, `4e8350c`), and the tree
  holds more that is not committed (the last lines of the log).
- Everything since 2026-11-17 was written on Windows. What it left to
  do on Linux is gathered in section 2c: start there.
- The window manager, the shell, the studio, the lock screen and the
  display manager work on both systems. Linux was last tested by the
  person on 2026-11-17: all of it passes.
- CI is red on every push so far (section 1).
- The check scripts (`tests/desktop`, `tests/nested`, `tests/vm`,
  `tests/wayland`) are not in this repository: the public one leaves
  them out. On this machine only the crates' own tests can be run.
- The tutorial has its tour on both systems. Its ring, its chapters and
  what belongs around it are not written.

Before a session, on the system you are on:

| | Windows | Linux |
|---|---|---|
| Everything quick | `cargo xtask check` | `cargo xtask check` |
| The other system, type-checked | `pwsh scripts/check-linux.ps1` | `scripts/check-windows.sh` |

(The Windows machine's WSL has no cargo: what is only Linux's is not
checked from there.)

---

## 1. The ground under the work

Do these first. Until they are done, a fault can come in unseen.

- [ ] **W** CI, the Windows job:
      `conflicts::tests::nothing_is_said_when_the_zones_are_not_on_shift`
      (`backend-win32`) fails on the runner and passes here. `notices`
      says something there that it does not say on this machine. Find
      what it reads of the machine and keep the test from it.
- [ ] **L** CI, the Linux job: the studio's
      `every_page_looks_as_its_picture` fails with every page 3 to 10 %
      off its picture. To find: what the runner draws otherwise (the
      letters, most likely), and whether the test brings its own fonts
      or is left out on CI.
- [ ] **any** CI green once, to the end: the clippy step on Windows and
      the nested checks on Linux have never been reached there.
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

- [ ] **person, L** The Linux shell builds and the panel's tabs, their
      renaming and the hot-plugging of a display behave as on Windows
      (`shell/src/main.rs` was changed without being built).
- [ ] **L** The studio's picture of the workspaces and displays pages
      drawn anew (`GEVREK_BLESS=1`): both pages changed.
- [ ] **person, W** Seen on the desktop: a tab renamed twice, on each
      display; a workspace taken out, put in another order and added in
      the studio while windows are open; a workspace of the moment kept.
- [ ] **person** The person's own configuration tidied in the studio
      (`Saturn`, `Gevrek`, `Web` are names there, with `1`, `Gevrek2`,
      `4` as titles on one display; `Gitem-2` and `gitem` are strays).
- [ ] **any** Key bindings, rules and the graph choose a workspace from
      those there are, instead of a typed name.
- [ ] **any** A title given by the configuration re-evaluates the rules
      that go by a workspace's name, as a renaming by command does.
- [ ] **any** The overview and the window switcher name a workspace by
      display and name, not by its place (`overview.rs`).
- [ ] **L** The compositor's workspace protocol tells the title.
- [ ] **any** A saved session finds its workspaces by name on a display
      that has another handle than before.
- [ ] **W** The panel's modules display by display (`bar.on()` is never
      asked on Windows).
- [ ] **any** The studio does not offer Apply while the window manager
      is away (a first try broke the trial's test: the session there
      has no line).
- [ ] **any** From the studio's audit: slider ranges that are not the
      model's limits; a new startup program or scratchpad is Windows
      Terminal on Linux too; `--file` mode writes over a change made
      elsewhere.

## 2c. Next time on Linux

Everything the Windows sessions since 2026-11-17 changed without being
able to build or see it on Linux, in the order to take it.

First, that it builds and still passes:

- [ ] **L** `cargo xtask check` and `scripts/check-windows.sh`. The
      shared crates changed a great deal (`model`, `engine`, `protocol`,
      `server`, `shell/panel.rs`, `studio`); `shell/src/main.rs` and
      nothing else of Linux's own was edited, unbuilt.
- [ ] **person, L** The desktop as on 2026-11-17 (the person's own
      round of tests), again: nothing of it should be different.

Then what is known to be missing or unseen there:

- [ ] **L** The studio's pictures drawn anew (`GEVREK_BLESS=1 cargo
      test -p gevrek-studio every_page_looks_as_its_picture`) and looked
      at before they are kept: workspaces, displays, animations, panel,
      snap zones, input, window rules, mouse and gestures all changed.
- [ ] **person, L** Workspaces (section 2b, its first item): the
      panel's tabs, a tab renamed, a display plugged in and out, each
      display with its own workspaces.
- [ ] **L** A renamed workspace is on its tab at once. On Windows the
      window manager published its state too rarely and the shell did
      not ask on `Applied` (`runtime/mod.rs`, `shell/windows.rs`). To
      find: when the compositor publishes its state
      (`backend-smithay`), and `Event::Applied` in `shell/main.rs`,
      which only notes the revision.
- [ ] **L** Hot corners (`desktop.hot_corners`) are not watched. The
      logic is shared and tested (`engine/hot_corners.rs`,
      `Watch::step`); the compositor has to call it with the pointer's
      place from `pointer_moved` (`backend-smithay/src/input.rs`) and
      again when the moment is over though the pointer rests (a timer:
      no motion event comes then). Then the sentence in the guide that
      says they are not watched on Linux goes.
- [ ] **L** The overview of the windows (`toggle_expose`). On Windows
      it shows pictures (`Desktop::toggle_window_overview`,
      `expose::arrange`). The compositor does not answer that call, so
      there the windows themselves are still sized into a grid. A
      compositor can draw a window at any size: it should answer the
      call and draw them small where `arrange` says, with the titles,
      the mark and the keys of `backend-win32/src/dwm/overview.rs`.
- [ ] **L** The two settings of the workspace slide
      (`workspace_slides_as_pictures`, `workspace_slides_without_shadows`)
      do nothing on Linux, where the compositor slides pictures anyway.
      Said so in the studio, or not shown there.
- [ ] **L** The compositor's workspace protocol tells the title
      (section 2b).
- [ ] **L** CI, the Linux job (section 1).

Not Linux's, but waiting for the person on Windows from the same days:

- [ ] **person, W** The overview: how it looks and feels with many
      windows, on both displays, with a program that draws its own
      transparency; that a window is never left unseen after it.
- [ ] **person, W** The hot corner at the top left, with the panel
      there; the moment it takes (250 ms).
- [ ] **person, W** A workspace renamed from its tab: there at once.
- [ ] **person, W** The setup program with the helper's checkbox, an
      update over it, and the uninstall (never run on this machine);
      the release workflow's new step on a runner.
- [ ] **person, W** The two settings of the workspace slide: what they
      do to the graphics card's load (nothing was measured).

**This file is in the repository** (since 2026-11-18): a pull on the
Linux system brings it. Its archive, `docs/ROADMAP-archive-2026-11.md`,
is not (`.git/info/exclude`); nothing here needs it.

## 3. The tutorial

The designs are in the archive under "The welcome and the tutorial". The
code: `crates/tutorial`, `shell/src/tutoring.rs`, `shell/src/coach.rs`,
`studio/src/welcome.rs`, `studio/src/sample.rs`.

### Stage 1: the tour, finished

- [ ] **any** Practice windows say how they stand: the keyboard,
      floating, a tab of how many. Today they show one sentence that
      never changes.
- [ ] **any** The ring around what a step is about (a window, every
      window, the panel, a surface). Lessons name it already (`Spot`);
      nothing draws it. On Windows a see-through surface as large as the
      display is black unless the graphics device draws with DirectX 12:
      four thin surfaces for the ring's sides, for which a `Place` must
      be able to say where on the display it is.
- [ ] **any** The tutorial on the display the person is on, not always
      the main one.
- [ ] **any** A window of the person's own on the tutorial's workspace
      no longer keeps that workspace from going at the end.
- [ ] **any** The card does not lie over the lower right window's foot.
- [ ] **any** The welcome window: the main key chosen there (every
      binding rewritten, with what collides said); and it folds into the
      coach's card instead of closing.
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
