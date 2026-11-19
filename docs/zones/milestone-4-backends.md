# Zones: milestones 4 and 5, the backends

Written 2026-11-19. What the two system backends
(`crates/system/src/zones/`) supply for folder portals, smart zones,
rules that ask the time or move files, and for speed: per feature and
system what there is and how it is done, the functions the shell calls,
what was verified and what was not. The contract with the core is
`core-milestones-3-5.md`; this note is the backends' side of it.

**Nothing of this was seen in a zone.** It was built and tested without a
desktop: against folders the tests make under the system's temporary
folder, a made-up Recent folder, a made-up home, and (on Windows, reading
only) the real shell. The example was run once on the Windows machine
this was written on; it reads and changes nothing.

## What is new, in one table

| | Both | Windows | Linux |
|---|---|---|---|
| Shared code | `zones/pictures.rs` (pictures by handle, bounded), `zones/reach.rs` (there / away), `zones/sources.rs` (a list read on its own thread), `zones/smart.rs` (which sources are asked for; launchers; running programs), `zones/programs.rs` (programs as items, pins) | | |
| Portals | a watched folder is read and watched off the caller's thread | `windows/portal.rs` | `linux/portal.rs` |
| Cloud files | | `windows/cloud.rs` | nothing special |
| Recent files | | `windows/recent.rs`, `windows/places.rs` | `linux/recent.rs`, `Places::downloads` |
| Facts | | `accessed` is "last opened", from the Recent folder | `accessed` is the kernel's; the zones' own reads do not move it |

The trait methods the core added are all answered: `begin_listing`,
`list_source`, `watch_source`, `begin_operation`, `cancel_operation`;
`BackendEvent::ListingFailed`, `SourceListed`, `OperationProgress` and
`OperationDone` are sent. `screen_shared` is left at its default (`None`)
on both: Windows gives no sign of it, and on Linux the compositor knows
and tells the shell.

## 1. Folder portals

### What a portal's folder does now

`watch(Location::Directory)` **always answers at once**, with `Ok`, also
for a folder that is not there or a machine that does not answer. The
folder is gone to on a thread of its own: seen to be there, given to the
system to tell of, read, and then told in one piece
(`BackendEvent::Listed`). `begin_listing` answers `true` for every
directory and never reads on the caller's thread; `list` of a watched
directory does not go to the disk either while the folder is being read
or is away (it answers with what is known: nothing).

| What happens | What the core is told |
|---|---|
| The folder was read | `Listed { location, items }`, whole, in one event (5,000 items are one event) |
| It did not answer within 4 s (`reach::PATIENCE`) | `ListingFailed { location, error: NotFound { what: "<path> does not answer" } }`. Its thread waits on, as long as the system does; when the folder answers after all: `Listed` |
| It is not there | `ListingFailed` with `"<path> is not there"` |
| It was deleted, renamed away or its drive pulled out while watched | `ListingFailed` |
| It came back (made again, renamed back, another folder put in its place) | `Listed`, whole |
| Something in it changed | `ItemAdded` / `ItemRemoved` / `ItemRenamed` / `ItemChanged`, as before |

How a folder's going and coming is noticed, without looking by the clock:

- **Windows.** While a folder is watched, the folder above it is watched
  too, for the one thing: the watched folder's own name (Windows does not
  tell a folder of its own renaming). While a folder is away, the nearest
  folder above it that is there is watched for the next name on the way
  down; a whole path that is made again is followed one step at a time.
  A drive that is pulled out or a share that drops ends the file
  system's telling with an error, which is taken as "go and look".
- **Linux.** The kernel tells a watched folder that it was deleted or
  moved (`IN_DELETE_SELF`, `IN_MOVE_SELF`); the folder is then gone to
  again, which says it is away. While away, the nearest folder above is
  watched, as on Windows.
- **What nothing can announce** (a server that is switched on again, a
  mount or a drive that comes back: neither changes any folder that can
  be watched) is tried again when somebody asks: the core's
  `refresh_zone` → `BackendRequest::Relist` → `begin_listing`, or
  `SystemBackend::retry_unreachable()`. **The backends never retry by
  themselves.** If the host wants away folders to come back unasked, it
  sends `Relist` on a slow schedule for zones whose listing is
  `unavailable` (the core's note allows that).

Changes that happen while a large folder is being read are not lost and
not told stale: the telling begins before the reading, and (Windows)
every item that changed during the reading is read once more at its end;
(Linux) what the kernel told meanwhile is applied after the listing.

### Network folders

A UNC path (`\\server\share\folder`, also written with `/`) or an SMB/NFS
mount is a directory like any other to the backends. What differs is
only time, and that is what the threads are for: nobody who draws waits.

- Icons and small pictures of files in such a folder are made on the
  picture thread, one after the other. A machine that dies while its
  folder is shown holds up the pictures behind it until the system gives
  up on each (the listing itself is told as away, so nothing new is asked
  for). Not measured.
- Linux: inotify tells nothing of changes other machines make on a
  network mount. Such a portal shows what was there when it was read;
  `refresh_zone` reads it again.
- Windows, Recent files: a file on another machine is shown by what the
  shortcut in the Recent folder says of it (name, size, times); the
  machine is not gone to (`recent::far`: a `\\` path, or a drive letter
  that stands for a share).

### Cloud folders (Windows: OneDrive and everything else that syncs through Windows' cloud files)

The rule: showing a folder must never download a file.

| | How | Fetches? |
|---|---|---|
| Listing, facts | names, attributes, sizes, times (`IShellFolder`, `GetFileAttributes`-level metadata); the type from the ending | no |
| Icon of a file that is here | the shell's, as for any file | no |
| Icon of a file that is **not** here | the icon of its *kind*: asked of a made-up name with the same ending, so the shell is never handed the real file (`cloud::picture`; a program's own icon lies inside its file, and reading that would fetch it) | no |
| Small picture of a file that is not here | only what Windows already has in its own store of small pictures (`SIIGBF_THUMBNAILONLY \| SIIGBF_INCACHEONLY`); else none, and the icon is shown | no |
| Opening it | the shell's verb, as from Explorer | yes: that is what the person asked for |

"Not here" is read from the file's attributes
(`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `_RECALL_ON_OPEN`, `_OFFLINE`).
`SystemBackend::cloud_state(&ItemId) -> Option<CloudState>` says
`OnlineOnly`, `AlwaysKept` (pinned) or `Here` (may be sent away again),
and `None` for a file no cloud marks.

**What is shown:** the plain icon, with no cloud or tick on it. Explorer's
signs are its own; a renderer that wants one draws it from `cloud_state`.
It is not in `ItemFacts` (the core has no fact for it), so rules cannot
ask for it.

Linux has no special case: a folder synced by a program holds ordinary
files.

### File operations inside a portal

`perform` is as in milestone 2 (copy, move, rename, new folder, trash,
restore), each answering what takes it back, nothing ever written over,
nothing ever deleted outright.

New: **operations that end later** (`zones/operations.rs`, the same on
both systems). `begin_operation(request, &operation)` answers `true` at
once; the operation is carried out on a thread of its own
(`gevrek-zones-files`), one after the other in the order they were asked
for, and its end is told as `OperationDone { request, .. }` with exactly
what `perform` would have answered. So a large copy holds nobody.

| | Windows | Linux |
|---|---|---|
| Carried out by | the shell's own file operation, as `perform` | the same code as `perform` |
| `OperationProgress { done, total }` | the shell's own count of work (no count of bytes), as the shell reports it | bytes, of the bytes of everything to copy; for copies and for moves to another file system |
| `cancel_operation` before it began | not begun; `OperationDone` with `Cancelled` | the same |
| `cancel_operation` while it runs | the shell is answered "called off" the next time it asks (before each item, and while it copies); it takes away what it had begun to write | the copy stops between two files, and inside a file between two pieces of a megabyte; what was copied so far is removed; a file being moved stays where it was, whole |
| A half-written file under the real name | never by the shell's own operation, as far as its documentation goes | never: everything is written whole under a name beside it that begins with a dot (`.<name>.gevrek-part-<pid>-<n>`, which no zone shows), and only then given its real name, never over anything. A run that is killed leaves such a dot file, not half a file |
| Too late to call off | the real result is told | the same |

What was put into the Recycle Bin by an operation that ended later is
remembered together with what `perform` put there, so either puts back
the other's files exactly.

## 2. Smart zones

`watch_source(&SmartSource)` answers at once; the list is read on a
thread of its own and told whole as `BackendEvent::SourceListed`, in the
source's own order, when it is first asked for and whenever it is other
than it was. `list_source` answers with what is known at that moment
(nothing before the first reading) and has the list read again; so the
host's "watch, then list" gives an empty list first and the real one a
moment later. `Desktop` and `Folder` are answered `Unsupported`, as the
contract says.

| Source | Windows | Linux | Kept current by |
|---|---|---|---|
| `recent` | The shell's Recent folder (`FOLDERID_Recent`): the targets of its shortcuts, the newest shortcut first, at most 100, each once, without what is gone. `facts.accessed` is the shortcut's own time of writing: when the file was last opened | `$XDG_DATA_HOME/recently-used.xbel`, read by hand: the `file://` bookmarks that are still there, the last `visited` first, at most 100; `facts.accessed` is that time | a watch on the folder / on the file's folder |
| `downloads` | `FOLDERID_Downloads` | `XDG_DOWNLOAD_DIR` of `user-dirs.dirs`, else `~/Downloads` | a watch on the folder |
| `pinned` | pinned programs, then the 12 most started | the same | a watch on gevrek's state folder and on the folders programs are installed into |
| `running_apps` | from the window manager's state | the same | `SystemBackend::set_wm_state` |

**Downloads** are told newest first (by the later of made and written,
which for a download is when it arrived), at most 1,000. "From today" is
the core's filter (`date`, `newer_than_days`), on `created` or
`modified`. The whole list is told again whenever a file in the folder
changes; for a folder of a thousand files that is a reading of the folder
per burst of changes (many changes while it reads are one more reading).

**Pinned launchers.** Nothing in gevrek kept pins before: the launcher
only counts starts (`launcher-usage.json`) and remembers the last ones
(`launcher-recent.json`), both in the state folder. Both are read as the
launcher writes them. Pins are the zones' own file,
`<state>/zones/pinned-launchers.json` (a list of program ids in the
person's order). The source is: the pinned programs in that order, then
the 12 most started that are not pinned. An item is `ItemKind::Launcher`
with the id `app:<program id>` (the id the launcher knows the program
by), `path: None`, and `facts.shortcut_target` = what it starts.

**Running programs.** One item for each program that has windows, id
`running:<key>` (the program's file in small letters on Windows, its
`app_id` or `class` on Linux), named as the installed program is where
one fits. The backend keeps which windows belong to it, the one with the
keyboard first.

### What the shell calls (the same names on both systems)

| Call | For |
|---|---|
| `backend.set_wm_state(&json)` | after every `Event::State`, beside `displays_of_state`: the running programs are worked out and told (`SourceListed { running_apps }`) when they are others |
| `backend.set_commander(\|command: serde_json::Value\| ...)` | once: how a command reaches the window manager (the shell's line). `open` of a `running:` item sends `{"command":"focus_window","selector":{"kind":"handle","value":<id>}}` through it; without it `open` answers `Unsupported` |
| `backend.open(&item)` | `app:` starts the program (Windows: as the Start menu does, on a thread of its own; Linux: through `set_launcher`, as every other start); `running:` gives its window the keyboard |
| `backend.pin_launcher(app_id, bool)`, `pinned_launchers()`, `set_pinned_launchers(&[String])` | the person pins, unpins, reorders. `app_id` is the id without `app:` |
| `backend.installed_programs() -> Vec<Item>` | every installed program by name, to choose one to pin. Slow (it reads the Start menu or every desktop entry): not on the thread that draws |
| `request_icon` for `app:` and `running:` items | answered like any other (the program's own icon; on Windows the icon of a running program's file) |
| `programs::running(&json, &apps)`, `programs::focus_command(window)` | the same conversion without a backend, if the shell would rather send `SourceListed` itself |

`context_menu`, `open_with` and `start_drag` answer `Unsupported` for
programs: what can be done with a launcher (unpin, close) is the
renderer's to offer.

## 3. Rules that ask the time, and rules that move files

**The three times.**

| | `created` | `modified` | `accessed` |
|---|---|---|---|
| Windows | the file system's | the file system's | **when the person last opened the file, as the shell wrote it down** (the Recent folder), absent if it never did |
| Linux | the file system's birth time, where it has one | the file system's | the kernel's time of last access |

*Why Windows does not use the file system's time of last access.* NTFS
keeps it only when `NtfsDisableLastAccessUpdate` allows, and since
Windows 10 1803 the system switches it off by itself on large drives;
where it is on it is kept to the hour; and any program that reads the
file moves it, the making of small pictures among them. What the shell
puts into the Recent folder is the person's own opening of the file and
nothing else. So on Windows **"untouched for N days" means: not made, not
written and not opened (through the shell) for N days**
(`ItemFacts::last_touched` = the latest of the three). A file opened in a
way the shell does not record (a program that reads it by itself without
saying so) does not count as touched.

The record is read from the first thing that is watched on, and kept
current by a watch on the Recent folder. When a file that a zone shows
is opened, the core is told `ItemChanged` with the new `accessed`, so a
timed rule sees it.

*Linux.* The kernel moves the time of last access at most once a day
unless the file was written since (`relatime`, the default), which is
exactly fine for a rule that counts days. On a file system mounted
`noatime` it never moves, and "untouched" then counts from made and
written only. So that a zone drawing a file does not count as the person
using it, the zones' own reads (telling a file's type from its bytes,
making a small picture) open files with `O_NOATIME`; the kernel grants
that to the file's owner only, and anybody else's file is read the plain
way.

**Moves by rule** go through `perform(FileOperation::Move)` as before:
the answer is the way back, with the name the file really got; a file is
never written over (it gets another name); trash is the system's (the
Recycle Bin by the shell's own operation on Windows, the freedesktop
trash on Linux).

## 4. Rules by shortcut target and by type, on real samples

Verified by tests that make the samples:

| Sample | Windows | Linux |
|---|---|---|
| a `.lnk` to a folder | `Shortcut`, target the folder's path | |
| a `.lnk` to a program | `Shortcut`, target the program's path; extension `lnk` | |
| a `.lnk` whose target is gone | `Shortcut`, target the path it had | |
| a `.url` | `Shortcut`, target the address | |
| a symlink to a file | | `Shortcut`, target the file, type the target's |
| a symlink to a folder | | `Shortcut`, type `inode/directory` |
| a symlink to nothing | | `Shortcut`, target where it would lead, no size |
| a `.desktop` of `Type=Link`, allowed | | `Shortcut`, target the address, name the entry's |
| a `.desktop` launcher, allowed / not allowed | | `Launcher` with what it starts / a plain `File` |

The type on Windows is told from the ending (the registry's content
type, else Windows' "perceived type"); on Linux from the name, and from
the bytes where the name does not decide.

## 5. Privacy

Nothing from the backends: `screen_shared()` is `None` on both.

## 6. Speed and memory

**No polling.** How each thing that waits, waits:

| Thread | Sleeps in | Woken by |
|---|---|---|
| `notify`'s (one per watched folder on Windows; one per portal and one for the desktop on Linux) | the system (`ReadDirectoryChangesW` completion / `epoll` on inotify) | a change |
| `gevrek-zones-watch` (Windows, the hidden window) | `GetMessageW` | the shell, a display change |
| `gevrek-icons` (pictures) | a channel | a request |
| `gevrek-zones-source` (one per smart source in use) | a channel | the file system's telling, or `refresh` |
| `gevrek-zones-files` (file operations that end later) | a channel | an operation |
| `gevrek-zones-portal`, `-portal-read`, `-list` | they do one going and end | |

Nothing is looked at by the clock anywhere in the backends. On Windows,
watching the folder above a portal's folder means that a busy folder
above (the person's profile folder above Downloads, say) costs one
comparison of names for each change in it.

**Pictures are lazy and bounded.** A picture is made only when it is
asked for, once, off the drawing thread (as before). New: the pixels
that are kept are bounded in bytes, 48 MiB unless
`SystemBackend::set_picture_budget(bytes)` says otherwise. When more is
held, pictures the renderer already took go first (it has its own
texture), the one taken longest ago first; then those nobody took yet,
the oldest first. What one `drain_events` answers is never let go of by
that same drain.

**For the renderer:** `picture(handle)` gives `None` for a handle whose
picture was let go of, exactly as after its item changed. It then has to
ask again (`request_icon` / `request_thumbnail`), and must not remember
"there is none" for that item. A renderer that takes the pixels when it
is told `IconReady` and keeps its own texture never meets this.

**Listing 5,000 files** (debug builds, the tests'
`five_thousand_files_are_listed_without_holding_the_caller`, files of one
byte, on the machine this was written on):

| | The caller was held | Listed after |
|---|---|---|
| Windows 11 (NTFS, `%TEMP%`, through the shell's namespace) | 40 µs | 220 to 260 ms |
| Linux (WSL 2, ext4 `/tmp`) | 55 µs | 46 ms |

The tests assert only that the caller is held under 100 ms and that the
listing comes within a minute; the numbers above are what was printed.

## 7. The gaps milestone 2 left to the backends

- **Windows: a thing on the desktop that is no file and has no value in
  "Desktop icon settings".** Decided: it is shown when it is registered
  under Explorer's `Desktop\NameSpace` key (the person's or the
  machine's), which is how a program that is no part of Windows puts
  such a thing on the desktop, and is not hidden by a
  `HideDesktopIcons` value. Windows' own places are registered there
  too and are shown only by the setting, so they are named in a list
  (`facts::OF_WINDOWS`: This PC, the person's folder, Network, Control
  Panel, Libraries, Home, Gallery, OneDrive, Linux and some older ones).
  On this machine the shell lists nine things that are no file directly
  under the desktop, and the rule shows the same two Explorer shows; the
  example's comparison with Explorer's own 26 icons is empty both ways.
  A root that a later Windows adds without setting the value would
  appear on the desktop until it is added to the list.
- **Windows: `perform` is synchronous.** It still is, and beside it
  there is now `begin_operation`, which is not (see "File operations
  inside a portal"). The host uses that for whatever may take long.
- **Linux: New folder, Properties, "open with".** The backend supplies
  all three already, and they were verified by the existing test of the
  menu: `take_context_menu()` after `context_menu` gives `actions` with
  `NewFolder { parent }` (always, also on the bare ground: the desktop
  folder), `Properties` (with `ContextMenu::properties`: the item with
  its facts, its rights as `mode`, `trusted` for a desktop entry,
  `opens_with`), and `OpenWith { programs }` with **every** candidate
  (`Program { id, name, app, exec }`, the default first; not six).
  `open_with(item, None)` + `take_program_choice()` gives the same list
  for a chooser. What is missing is in the shell's drawing of that menu.
- **Linux: the working directory and activation tokens.** See the next
  section.

### What the window manager's `exec` would need (Linux)

Today the shell starts a program with
`{"command":"exec","program":…,"args":[…]}` (`DispatchCommand::Exec`).
The backend hands the shell a `Launch` with more than that can carry:

| `Launch` field | From | Lost today because |
|---|---|---|
| `directory: Option<String>` | the entry's `Path=` | `Exec` has no field for it |
| (none yet) | | no activation token is made |

1. **Working directory.** `DispatchCommand::Exec` needs an optional
   `directory: Option<String>` (`#[serde(default)]`, left out when
   absent, so old bindings read as before); the engine passes it on to
   whatever starts the program, which calls
   `Command::current_dir(directory)` when it is there and is a folder.
   The shell's closure for `set_launcher` then adds
   `"directory": launch.directory` to the command. The same field serves
   the launcher (`launcher::start_command`), which loses `Path=` in the
   same way.
2. **Activation token.** The compositor already implements
   `xdg_activation_v1` (`backend-smithay/src/protocols.rs`) and gives
   the focus to a client that presents a fresh token. What is missing is
   making one at `exec`: when `Exec` comes from a person's action (it
   carries, or implies, the seat's last serial), the compositor makes a
   token in its `XdgActivationState` for that moment and that seat, and
   starts the program with `XDG_ACTIVATION_TOKEN=<token>` (and
   `DESKTOP_STARTUP_ID=<token>` for X11 and older toolkits) in its
   environment. The toolkit of the started program hands the token back
   with its first window, which is then raised and focused even where
   focus stealing is otherwise refused. Nothing changes in the command's
   JSON for this; it is the compositor's side of `exec`. An optional
   `"activate": false` could let a caller start something in the
   background.
3. `DBusActivatable` programs opened with files: `activate.rs` in the
   shell knows `Activate` and `ActivateAction`, not `Open(uris,
   platform_data)`. Until it does, such a program is started by its
   command (the `Launch` carries both `bus_name` and `addresses`). With
   `Open`, the token goes into `platform_data["activation-token"]`.

## Tests

Unit tests only use folders they make (`%TEMP%\gevrek-zones-*` on
Windows, the system's temporary folder on Linux). The person's Desktop,
Documents, Downloads and Recent are never read by a test, with these
read-only exceptions on Windows, which existed before or only list:
the desktop is listed by the shell; `places::tests` asks Windows where
Downloads and Recent are (the paths only); `recent::tests::
the_real_recent_folder_can_be_listed` counts the shortcuts in the real
Recent folder and opens none. Nothing is recycled by the new tests.

Seams: `SystemBackend::with(places::Places::under(root), explorer, wake)`
on Windows (a made-up `Downloads`, `Recent` and state folder),
`Places::under(root)` on Linux as before; `set_programs(...)` for a
made-up list of installed programs; `set_reach(portal::How { .. })` to
play a machine that does not answer.

New tests, by what they show:

- `pictures`: handles; a changed item gets new ones; **no more bytes are
  kept than the bound**, pictures already taken go first, two thousand
  asked for in a row never pass the bound, what one drain answers is all
  there.
- `operations`, and one test of each backend: operations end later, in
  order, with what takes them back; what goes wrong is told the same
  way; one that waits is called off without being begun; (Linux) a copy
  called off half way leaves nothing, and says how far it was in bytes.
- `sources`, `smart`, `programs`: a list is told whole when it differs
  and only then; launchers (pins first, then by starts), pins written
  and read, the launcher's files read; running programs out of a state
  (grouped, the focused window first, named by the installed program);
  focus as a command.
- Windows `portal`: a folder that does not answer is told as away at
  once and shown when it answers; a folder that is not there, is made
  step by step, renamed away, replaced, deleted and made again is
  followed; 5,000 files.
- Windows `recent`, `cloud`, `facts`, `tests`: the Recent folder read
  (order, each file once, gone ones left out, when each was opened, only
  changed shortcuts read again); a file on another machine taken from
  the shortcut without going there; the attributes of cloud files; the
  icon of a kind drawn without a file; things on the desktop that a
  program registered; recent files and downloads through the backend;
  **a file that is opened changes its item in a portal**; programs as
  items; shortcuts to a folder, a program, an address, and a broken one.
- Linux `recent`, `tests`: the record parsed (entities, times, other
  tags); recent files and downloads through the backend; a watched
  folder followed as it goes and comes; a folder that does not answer;
  5,000 files; programs started and brought to the front; links to a
  folder, to nothing, and an entry that is a link; **a file read by the
  zones keeps its time of last access**.

## Running it

```
cargo build
cargo test -p gevrek-system
cargo clippy -p gevrek-system --all-targets
cargo deny check licenses advisories bans sources
cargo run -p gevrek-system --example zones_desktop
```

and from Git Bash for the Linux half:

```
MSYS_NO_PATHCONV=1 wsl.exe -d FedoraLinux-44 -- bash -lc '. ~/.cargo/env; cd /mnt/e/source/private/gevrek && export CARGO_TARGET_DIR=$HOME/gevrek-target && cargo test -p gevrek-system && cargo clippy -p gevrek-system --all-targets'
```

No dependency was added. (`notify`, already in, is now used by the
shared code as well; `jiff`, already in for Linux, reads the times of
the recent files' record.)

## What was verified

- Windows 11 Pro 26300: `cargo test -p gevrek-system` 149 passed, 6
  ignored (as before); `cargo clippy -p gevrek-system --all-targets`
  clean; `cargo deny check licenses advisories bans sources` all four
  ok; `cargo build` (the default members) builds.
- WSL, Fedora 44: `cargo test -p gevrek-system` 146 passed, 4 ignored;
  clippy clean.
- The example on Windows (reading only): the desktop's 26 items are the
  26 icons Explorer shows; the Recent folder gave 100 files and the
  Downloads folder its files, newest first.

## What was not verified

- **Anything in a zone on a screen.**
- **A real network share.** A machine that does not answer is played by
  a test (the going to the folder blocks until the test lets it
  through). How long Windows or the kernel really waits, what the file
  system's telling does when an SMB session drops, and whether a share
  that comes back is noticed without `Relist`, were not seen.
- **A real cloud placeholder.** That no file is fetched follows from
  what is asked (attributes, the icon of a made-up name, the store of
  small pictures only); it was not watched on an online-only OneDrive
  file. Whether Windows marks every provider's placeholders with the
  attributes read here was not checked beyond the documentation.
- **The Recent folder being written by a real program.** The tests
  write shortcuts as the shell does. That Explorer's own opening of a
  file changes the shortcut's time of writing every time was not
  watched.
- **Linux in a session:** GTK writing `recently-used.xbel`, inotify on a
  real second file system, an unmount under a portal, `O_NOATIME` on a
  file system other than the temporary one.
- **Starting programs** from `app:` items and focusing windows from
  `running:` items for real: the tests see the launch and the command.
- **Memory** as the system counts it. The bound is on the bytes of
  pixels the backend holds, and is tested as arithmetic.

## Not done

- **Calling off a copy on Windows in the middle of a file was not seen.**
  That the shell stops when it is answered "called off" before an item
  is tested (nothing arrives, the answer is `Cancelled`). That the same
  answer while it copies one large file ends that copy and leaves no
  half file is what the interface's documentation implies; no test
  copies something large enough to be caught in the middle. On Linux
  every case is tested, also a disk that runs full half way (played)
  and a folder that is taken away under the copy.
- **Progress on Windows is not in bytes,** and the shell tells it only
  for operations long enough that it would show its own window.
- **No retry by the clock** for folders that are away, by decision (see
  portals). A network change or a drive's arrival could be listened for
  on Windows (`NotifyIpInterfaceChange`, `WM_DEVICECHANGE`) to try away
  folders again by themselves; not built.
- **Listing in parts.** A folder is told in one event. For 5,000 files
  that is a quarter of a second off-thread on Windows; a folder of a
  hundred thousand would be told late, and whole.
- **No cloud sign on icons, and no fact for it.**

## What is needed of the other layers

- **Shell:** call `set_wm_state` beside `displays_of_state`; call
  `set_commander` once; offer pin and unpin (`pin_launcher`); ask for a
  picture again when `picture(handle)` gives `None` (do not keep "none"
  for the item); send `Relist` (`refresh_zone`) for zones whose listing
  is `unavailable` when the person asks, and on a slow schedule if away
  folders should come back unasked; on Linux draw `NewFolder`,
  `Properties` and the whole `OpenWith` list of `take_context_menu()`.
- **Core / host:** nothing more; `begin_operation` and
  `cancel_operation` are answered as the contract says.
- **Window manager (Linux):** `directory` on `exec`, and an activation
  token for what it starts (above).

## Manual checklist

Nothing of this was done. Each line is one thing to look at.

**A portal on a large folder**

- [ ] A portal on a folder of 5,000 files: the zone says it is loading,
      then shows them; the panel goes on drawing meanwhile.
- [ ] Scroll through it: icons and small pictures come as they are
      scrolled to; the shell's memory stays level (task manager) after
      scrolling to the end and back.
- [ ] Unpack an archive of a thousand files into it: they appear, and
      nothing is shown twice or left out (compare the count with
      Explorer / the file manager).

**A network share**

- [ ] A portal on `\\server\share\folder` (Windows) or a mounted share
      (Linux): listed; the shell never stalls while it is.
- [ ] A portal on a share whose server is off when gevrek starts: within
      about four seconds the zone says unavailable; the panel was never
      stuck; other zones work.
- [ ] Unplug the network cable while the portal shows the share: the
      zone says unavailable (how soon?), the shell does not stall, the
      icons already drawn stay or go without a hang.
- [ ] Plug it in again: does the zone come back by itself? If not,
      refresh the zone (`refresh_zone`): it does.
- [ ] Recycle a file on the share from the portal: the shell asks before
      deleting (a share has no Recycle Bin); "No" leaves the file.
- [ ] A USB stick shown by a portal, pulled out and put back.

**A cloud folder (Windows, OneDrive with Files On-Demand)**

- [ ] A portal on a OneDrive folder that holds online-only files (the
      cloud sign in Explorer): after the zone has shown them, **every one
      still has the cloud sign in Explorer**, and OneDrive's activity
      list shows no downloads.
- [ ] The same after scrolling through all of them with small pictures
      on; the same for an online-only `.exe`, `.lnk`, picture and film.
- [ ] An online-only picture that Explorer has shown before has its
      small picture in the zone; one that was never shown has its plain
      icon.
- [ ] Double click on an online-only file: it downloads and opens, as
      from Explorer.
- [ ] A file marked "Always keep on this device" and one freed again
      ("Free up space"): both listed with the right sizes.

**Recent files**

- [ ] A smart zone on `recent`: it shows what Explorer's "Recent" shows
      (Windows) / what GTK's file chooser shows under "Recent" (Linux),
      the newest first.
- [ ] Open a document by double click in Explorer or a file manager: it
      is first in the zone within a second or two, without anything
      being pressed.
- [ ] Open the same file from a program's own "Open" window: the same.
- [ ] Delete a file that is in the list: it goes from the zone when the
      list is next read (when anything else is opened); a refresh makes
      it go at once.
- [ ] A recent file on a share whose server is off (Windows): it is
      still listed, with its name and size, and the zone did not stall.

**Downloads from today**

- [ ] A smart zone on `downloads` with "newer than 1 day": download a
      file in a browser; it appears when the download is complete (and
      the browser's part file while it runs, if the query lets it).
- [ ] The zone is empty the next morning without anything being done.

**Launchers and running programs**

- [ ] A zone on `pinned`: the programs started most from the launcher,
      with their icons; pin one, it is first; unpin, it goes or falls
      back among the most started.
- [ ] Start a program from the launcher a few times: it enters the zone.
- [ ] Install and remove a program: the zone follows.
- [ ] A zone on `running_apps`: one icon for each program with windows;
      open and close programs, it follows at once; a press brings the
      program's last used window to the front.

**Rules that ask the time**

- [ ] Windows: a file on the desktop not written for a month but opened
      yesterday by double click is not caught by "untouched for 30
      days"; one nobody opened is.
- [ ] Linux: the same; and after a zone has drawn the small picture of
      an old picture file, `stat` still shows its old time of last
      access.
- [ ] A rule that moves old files to a folder: the files arrive, a file
      of the same name there is not written over, `undo_rules` brings
      each back.

**Idle**

- [ ] With zones shown and nothing happening for a minute, the shell's
      row in the task manager (or `top`) stays at nothing.
