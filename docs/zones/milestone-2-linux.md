# Zones: milestone 2, the Linux backend

Written 2026-11-19. The part of milestone 2 that is the Linux half of
`crates/system`: `gevrek_zones::backend::ZonesBackend` answered by the
freedesktop specifications.

**Nothing uses it yet, and nothing of it was seen on a screen.** The
shell does not make a `SystemBackend`; no zone shows an item. What exists
is the backend, tests that run without a display, and an example that
prints what the backend sees of a folder. Everything below was built and
tested in WSL (Fedora 44); what needs a real session is listed under "Not
verified" and in the checklist at the end.

## What exists

`crates/system/src/zones/linux.rs` and the folder beside it:

| Module | What it does | Specification |
|---|---|---|
| `linux` | `System` (what is known of the session; reads items and pictures) and `SystemBackend` (the trait) | |
| `places` | The home, the base directories, the desktop folder from `user-dirs.dirs` | XDG Base Directory; xdg-user-dirs |
| `uris` | A file as an address and back; `text/uri-list`, `x-special/gnome-copied-files`; `DropPayload` from and to them | RFC 8089, RFC 2483 |
| `mime` | A file's type by `globs2`, by `magic` where the name does not decide; aliases, subclasses, icon names | Shared MIME-info |
| `entries` | `.desktop` files in a shown folder: launcher, link, and the trust rule | Desktop Entry |
| `associations` | The default program and the "open with" list | MIME Applications |
| `thumbs` | The shared thumbnail store, read and filled | Thumbnail Managing Standard |
| `trash` | Throwing away and putting back | Trash |
| `files` | Copy, move, rename, new folder; never over anything | |
| `watch` | The kernel's changes (inotify, by `notify`) made into the core's events | |
| `md5` | MD5, for thumbnail names only | RFC 1321 |

`crates/system/src/apps.rs` was extended rather than a second parser
written: `main_group`/`Fields` (the fields of an entry as written),
`parse_as` (an entry read also when it asks not to be listed, which is
what a handler of files usually does), `commands_for`/`Opened` (an `Exec`
line with things to open). `command_of` is now `commands_for` with
nothing to open; its tests are unchanged and pass.

Every module is told its folders through a `Places`; none asks the
environment. `Places::of_session()` reads it once, `Places::under(root)`
is a whole pretend home for a test.

## The trait, method by method

| Method | How | By |
|---|---|---|
| `list` | `read_dir` of the desktop folder (`XDG_DESKTOP_DIR` of `$XDG_CONFIG_HOME/user-dirs.dirs`, else `~/Desktop`) or of the directory. Names beginning with a dot and ending in `~` are left out. An item's id is its file's address (`file:///home/ada/Desktop/a%20b.txt`), escaped as GLib escapes it. | xdg-user-dirs; RFC 8089 |
| `facts` | Extension; MIME type; size; created, modified, accessed; `shortcut_target`: a link's target, a launcher's program, a link entry's address. | Shared MIME-info |
| `watch`, `unwatch`, `drain_events` | One inotify watch per folder (not recursive), on `notify`'s thread, which wakes the host. `drain_events` turns what was told into `ItemAdded`/`ItemRemoved`/`ItemChanged`/`ItemRenamed`, or `Listed` when the kernel lost track. | inotify(7) |
| `request_icon` | On the icon cache's thread (`crate::icons::Cache`): a trusted launcher's `Icon`, else the type's icon names (`icons`, the type with a stroke, `generic-icons`, `<family>-x-generic`, `unknown`), looked up with the existing `icons::find_in`. | Icon Theme; Shared MIME-info |
| `request_thumbnail` | The store first (`normal`, `large`, `x-large`, `xx-large`, by the MD5 of the address; `Thumb::URI` and `Thumb::MTime` must fit). Else, for PNG, JPEG and WebP, made and put into the store. | Thumbnail Managing Standard |
| `open` | A trusted launcher: its `Exec`. A trusted link entry: its address, with the program for `x-scheme-handler/<scheme>`. An untrusted entry: refused. Anything else: the default program of its type. | Desktop Entry; MIME Applications |
| `open_with` | With a program (a desktop id, with or without `.desktop`): that entry's `Exec` with the file. Without: the candidates are kept for the renderer (`take_program_choice`). | MIME Applications |
| `context_menu` | The menu's data is worked out and kept (`take_context_menu`); the host is woken. See below. | |
| `perform` | `Copy`, `Move`, `Rename`, `NewFolder` in `files`; `Trash`, `Restore` in `trash`. The answer is what takes it back, with the names the files really got. | Trash |
| `drop_payload`, `start_drag` | What the surface's side set (`set_drop`), and a drag kept for it (`take_drag`). | RFC 2483 |
| `displays`, `reserved_areas` | What the shell gave with `set_displays` (below). | |
| Explorer's journal | The trait's defaults (nothing to do). | |

### Details that matter

- **MIME: the name is believed.** The specification has strong magic
  (priority 80 and up) looked at before any name. That means opening
  every file of a folder to list it. Here, as in GLib's quick answer, a
  name that decides is not doubted; bytes are read when no pattern fits
  or several do. Text without any hint is `text/plain` by the
  specification's last resort (no control bytes in the first 128).
- **Default program.** Lists are read in the specification's order
  (`$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS`, `$XDG_DATA_HOME/applications`,
  `$XDG_DATA_DIRS/applications`; in each `gevrek-mimeapps.list` before
  `mimeapps.list`). First valid default; else the first association
  (added, then `mimeinfo.cache`; a folder without a cache is read entry
  by entry). Removed associations take a program away from everything
  read later. A type with nothing falls back to what it is a kind of
  (`subclasses`; every `text/*` is `text/plain`).
- **`Exec`.** No shell. `%F`/`%U` take everything in one program; a line
  with only `%f`/`%u` is started once per file; a line with no place for
  a file gets it appended (as GLib and KDE do); the retired codes are
  dropped; `%%`, `%i`, `%c`, `%k` are filled in. A word is filled in one
  pass, so a file named `%c` stays that name.
- **Events.** A rename's two halves become one `ItemRenamed`. A save by
  renaming a hidden file over the real one is one `ItemChanged`. A file
  written many times is told once per drain. A watched folder that is
  moved, removed or lost by the kernel is told whole (`Listed`).
- **Trash.** Home trash for the home's file system. Elsewhere
  `$top/.Trash/$uid` only if `.Trash` is a real, sticky folder; else
  `$top/.Trash-$uid`. `Path` is escaped; relative to `$top` outside the
  home trash. `DeletionDate` is local time (by `jiff`). The note is made
  first, with `O_EXCL`. Where no can is possible the operation fails:
  nothing is ever deleted. The optional `directorysizes` cache is not
  kept.
- **Thumbnails** are written to a file beside the final one and renamed
  (0600; folders 0700), with `Thumb::URI`, `Thumb::MTime`, `Thumb::Size`,
  `Thumb::Mimetype`, `Software`. A picture that cannot be read gets a
  note under `fail/gevrek-<version>/` and is not tried again until it
  changes. Files over 64 MiB or 80 megapixels are not read.

## The trust rule for launchers

A `.desktop` file runs whatever its `Exec` says, under any name and icon.

- GNOME (Nautilus when it drew the desktop; "Desktop Icons NG") wants the
  executable bit and `metadata::trusted`, and refuses a file others may
  write.
- KDE trusts entries in the system's folders; otherwise the executable
  bit and the person or root as owner.
- Xfce wants the executable bit and a checksum it wrote down when the
  person said "trust", because an archive keeps the executable bit.

**The rule here** is these put together:

1. An entry whose real place (links followed) is in an `applications`
   folder of the data directories is trusted.
2. Any other only when: it is a regular file owned by the person or
   root; neither group nor others may write it; its owner may execute
   it; and its bytes are exactly those the person allowed.

"Allowed" is `SystemBackend::allow_launching(item)`: it sets `u+x`,
clears `g+w` and `o+w`, and keeps a copy of the file under
`$XDG_STATE_HOME/gevrek/zones/trusted-launchers/`. The comparison is of
the bytes themselves, so no checksum has to be trusted. A file changed
or replaced afterwards is a plain file again.

An untrusted entry is `ItemKind::File`, named by its file name, with the
icon of its type; `open` refuses it; the menu offers `AllowLaunching`.

`metadata::trusted` is not read: it lives in GVfs's own store. A launcher
allowed under GNOME has to be allowed once more.

## What the shell has to do

Both systems export `SystemBackend`, `SystemBackend::new(wake)`,
`SystemBackend::picture(handle)` and `pixels(IconSize)`.

- **Pixels.** `IconReady` carries an `IconHandle`;
  `backend.picture(handle)` gives `Option<&icons::Picture>`. A handle
  belongs to one item at one pixel size until the item changes; then it
  gives nothing and a new request gets a new handle.
- **Displays.** The shell already has the window manager's state as JSON
  (`gevrek_link::Event::State`). `displays_of_state(&json)` reads
  `monitors[]`: `stable_id` (`port:<connector>` where empty), position,
  size, scale, and the room kept (from the work area, else
  `reserved_*`). `backend.set_displays(displays, reserved)` after every
  `Event::State`; a change becomes `BackendEvent::Displays`.
- **Starting programs.** `backend.set_launcher(|launch| ...)`. A `Launch`
  has `program`, `args`, `directory`, `terminal`, `bus_name`,
  `addresses`. The shell's launcher sends
  `{"command":"exec","program":…,"args":…}` to the window manager and
  wraps a terminal as `launcher::start_command` does; the closure should
  do the same. Until it is set, programs are started as children of the
  shell (`start_directly`), which is right for the example only.
  - `exec` cannot carry a working directory, so an entry's `Path` is
    lost that way today.
  - `crates/shell/src/activate.rs` knows `Activate` and `ActivateAction`
    on the bus, not `Open` with addresses. A `DBusActivatable` program
    opened with a file should be started by its command until it does.
  - There are no activation tokens anywhere in the launcher today; none
    are made here.
- **The menu.** After `context_menu`, `backend.take_context_menu()`
  gives a `ContextMenu`: `at`, the items that are there, `actions` in a
  menu's order (`Open`, `OpenWith { programs }`, `AllowLaunching`,
  `Rename`, `MoveToTrash`, `NewFolder { parent }`, `Properties`) and
  `properties` for each item (the item with its facts, its rights,
  whether it is trusted, what opens it). With no items it is the menu of
  the bare ground. Each action names the trait call that carries it out.
- **Drag and drop** (the data only; the surface is shellkit's):
  - in: `uris::wanted_kind(&offered)`, then
    `uris::payload(kind, &data, allowed)` and `backend.set_drop(Some(…))`;
    `set_drop(None)` when the drag leaves. For a drag of our own:
    `uris::payload_of_items`.
  - out: `backend.take_drag()` gives a `DragOut`; `drag.kinds()` to
    offer, `drag.data(kind)` for each request.

Nothing was added to `crates/zones`.

## Dependencies

| Crate | Where | Why | cargo-deny |
|---|---|---|---|
| `notify` 8 (no default features) | Linux only | inotify | passes |
| `jiff` (workspace) | Linux only | local time for `DeletionDate` | already in the lock file |
| `gevrek-zones`, `gevrek-model`, `serde_json` | all | the trait; the window manager's state | workspace |
| `image` features `jpeg`, `webp` | all | thumbnails | already built for the shell |

`cargo deny check licenses advisories bans sources`: all four ok.

**The `trash` crate is not used.** It finds its folders in the process's
environment (so a test cannot point it at a folder without changing the
environment of every thread) and lists every mounted file system to put
one file back. The trash is written by hand, in `trash.rs`.

**MD5 is written by hand** (`md5.rs`): no MD5 crate is in `Cargo.lock`.
It is checked against the RFC's test suite, the block edges, and the
example of the thumbnail standard.

## Running the tests

From Git Bash, through WSL:

```
MSYS_NO_PATHCONV=1 wsl.exe -d FedoraLinux-44 -- bash -lc '. ~/.cargo/env; cd /mnt/e/source/private/gevrek && export CARGO_TARGET_DIR=$HOME/gevrek-target && cargo test -p gevrek-system'
```

The tests build everything they read under the system's temporary
folder (never the home; never the repository, where the kernel tells
nothing of a Windows drive). They need nothing installed.

The example:

```
cargo run -p gevrek-system --example zones_desktop -- [folder] [--thumbnails] [--watch]
```

## What was verified (WSL, Fedora 44, rustc 1.100)

- `cargo test -p gevrek-system`: 124 passed, 4 ignored (those need a
  session and were so before). 45 are the zones' and 2 the new ones in
  `apps`.
- The zones' tests 25 times in a row without a failure.
- `cargo clippy -p gevrek-system --all-targets`: no warnings.
- `cargo check -p gevrek-shell` on Linux; `cargo check -p gevrek-system
  --all-targets` on Windows.
- By the tests: user directories; the trust rule (each condition taken
  away in turn; a link to an installed entry; a link to one that is
  not); `Exec` codes; MIME by name, by bytes, with a database a test
  writes (nesting, masks, word sizes, ranges, `__NOGLOBS__`,
  `__NOMAGIC__`); the order of `mimeapps.list`; thumbnails (name, made,
  rights, text chunks, found again, another program's, stale, failed);
  trash and restore (home; a pretend top of a file system with `.Trash`
  sticky, not sticky, a link); file operations with their undo; the
  kernel's events for a file made, renamed and removed; the whole
  backend under `gevrek_zones::host::Host` with a real core.
- By the example, with a home under `/tmp` and the distribution's real
  database and themes: the real `globs2` and `magic` are read (a file
  named `paper` holding `%PDF-` is `application/pdf`); icons come from
  Adwaita and hicolor; a link to an installed entry is a launcher and a
  loose entry a file; a thumbnail lands in the store.

## Not verified

- **Nothing was seen**: no icon drawn, no menu shown, no drag made.
- **No program was started** from a zone in a session. The tests see the
  `Launch`; whether a program then opens the file is the shell's
  closure and a real desktop.
- **Displays.** `displays_of_state` is tested against JSON written by
  hand after `gevrek.proto`. Whether the compositor's sizes are logical,
  and whether its work area follows the panel's exclusive zone, was not
  looked at on a running compositor.
- **Other programs' thumbnails.** That the addresses are escaped exactly
  as GLib and Qt escape them is by reading, not by finding a thumbnail
  Nautilus or Dolphin made.
- **A second file system.** Finding the top of one and the trash there
  ran only against a folder playing the part.
- **Network and FUSE folders** (no inotify there), and what a file
  manager makes of a restore of something gevrek trashed, and the other
  way round.
- **Many items.** Listing reads each `.desktop` file and sniffs each
  file whose name says nothing; nothing was measured on a large folder.
- A picture in memory is never let go of except when its item changes
  or goes.

## Manual checklist (a real session under gevrek-wl)

Prepare: a desktop folder with a text file, a picture, a folder, a name
with spaces and one with non-ASCII letters, a `.desktop` file copied
from `/usr/share/applications`, and a link to one there.

**Items and launchers**

- [ ] Every item shows with the right icon; the picture gets a
      thumbnail; `~/.cache/thumbnails/normal/` has a new file of mode
      0600 that Nautilus or Dolphin then uses without making its own.
- [ ] A thumbnail made by a file manager (a film, a PDF) is shown.
- [ ] The copied `.desktop` shows as a file and does not start. After
      "Allow launching" it shows its name and icon and starts.
- [ ] Edit that file: it is a plain file again.
- [ ] The link to an installed entry starts without asking.
- [ ] Double click on a file opens it in the program `xdg-mime query
      default <type>` names; change it with `xdg-mime default` and it
      follows at the next open.
- [ ] A program with `Terminal=true` opens in the configured terminal.

**Context menu**

- [ ] On one file: open, open with (the default first), rename, move to
      trash, new folder, properties.
- [ ] On several of different types: no "open with", no rename.
- [ ] On the bare desktop: new folder only.
- [ ] "Move to trash", then restore it from Nautilus; trash a file in
      Nautilus, and undo a zone's own trashing.
- [ ] Trash a file on a USB stick: `.Trash-$uid` appears on the stick,
      the file is not copied to the home.

**Drag and drop** (each both ways: into a zone or portal, and out of one)

- [ ] Nautilus (GTK 4, Wayland): one file; several; a name with spaces
      and non-ASCII letters; with Ctrl (copy) and Shift (move).
- [ ] Dolphin (Qt, Wayland): the same; its drop menu (move/copy/link)
      when dragging out of a zone.
- [ ] Thunar (GTK 3, Wayland): the same.
- [ ] An X11 file manager through Xwayland (`GDK_BACKEND=x11 thunar`, or
      PCManFM): X11 to zone is the direction the design notes call
      unchecked; zone to X11 goes through `xdnd.rs`.
- [ ] A link from a browser's address bar: arrives as text, not as a
      file.
- [ ] Copy files in Nautilus (Ctrl+C); if paste is built, it reads
      `x-special/gnome-copied-files`, and "cut" moves.
- [ ] Drag a file from a zone onto a terminal: its path is typed.

**Changes from outside**

- [ ] `touch`, `mv`, `rm` in the desktop folder show at once, with no
      flicker of remove-and-add on a rename.
- [ ] Save a file from an editor that saves by renaming (gedit, vim):
      the item stays where it is.
- [ ] Unpack an archive with a thousand files into a portal's folder.
- [ ] Rename the desktop folder itself while gevrek runs.

**Hot-plug**

- [ ] Unplug and replug a display: zones return to it (the id is the
      EDID's, the same on another port).
- [ ] Two displays of one model without serial numbers stay apart.
- [ ] Unmount a stick a portal shows: the zone stays, empty; mount it
      again and the items return.

**Crash recovery**

- [ ] Kill `gevrek-shell` while a thumbnail is being written: no
      half-written `.png` in the store (a `.….tmp` may be left; it is
      never read).
- [ ] Kill it during a move across file systems: the file is whole at
      one end at least.
- [ ] Kill it between trashing and the next start: the file is in the
      trash with its note, and restorable from a file manager.
- [ ] Start again: launchers that were allowed are still allowed.
