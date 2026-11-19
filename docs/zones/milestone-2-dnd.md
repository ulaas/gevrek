# Zones: milestone 2, drag and drop for the shell's surfaces

Written 2026-11-19. The part of milestone 2 that is `crates/shellkit`:
files dragged between a surface of the shell and other programs, on both
systems, behind one API.

**Nothing uses it yet.** No surface of the shell asks for drops or begins
a drag; a surface that does not ask is as it was. What exists is the API,
both halves behind it, tests for everything that needs no hands, and an
example to try by hand (`cargo run -p gevrek-shellkit --example dragdrop`).
**No drag was ever made with a mouse while this was written**: see "Not
verified" and the checklist at the end.

## The API

The types are in `gevrek_shellkit::dnd` (no system is called there); the
calls are on `host::Control`, the same on both systems.

```rust
// gevrek_shellkit::dnd
pub struct DragActions { pub copy: bool, pub moving: bool, pub link: bool }   // NONE, ALL, COPY, COPY_OR_MOVE
pub enum   DragAction  { Copy, Move, Link }
pub struct Held        { pub shift: bool, pub ctrl: bool, pub alt: bool }
pub struct DropAnswer  { pub accept: DragActions, pub prefer: Option<DragAction> }  // REFUSE, accept(..), preferring(..)
pub struct NativeData  { pub kind: String, pub bytes: Vec<u8> }
pub struct DragPayload { pub paths: Vec<String>, pub uris: Vec<String>, pub text: Option<String>,
                         pub native: Vec<NativeData>, pub allowed: DragActions }  // files(paths)
pub struct Offered     { pub kinds: Vec<String>, pub files: bool, pub allowed: DragActions,
                         pub payload: Option<DragPayload> }
pub enum   DragEvent   { Entered { at: egui::Pos2, offered: Offered }, Moved { at: egui::Pos2 }, Left,
                         Dropped { at: egui::Pos2, payload: DragPayload, action: DragAction },
                         Ended { outcome: DragOutcome } }
pub enum   DragOutcome { Taken { action: DragAction, remove: bool }, Cancelled }
pub struct DragImage   { pub width: u32, pub height: u32, pub rgba: Vec<u8>, pub hotspot: (i32, i32), pub scale: f32 }
pub struct DragStart   { pub payload: DragPayload, pub image: Option<DragImage> }
pub fn negotiate(offered: DragActions, held: Held, answer: DropAnswer) -> Option<DragAction>

// gevrek_shellkit::host::Control (both systems)
pub fn accept_drops(&mut self, id: SurfaceId, standing: Option<DropAnswer>)
pub fn answer_drag(&mut self, id: SurfaceId, answer: DropAnswer)
pub fn drag_events(&mut self, id: SurfaceId) -> Vec<DragEvent>
pub fn start_drag(&mut self, from: SurfaceId, drag: DragStart)

// Windows only
pub fn start_drag_of(&mut self, from: SurfaceId, object: IDataObject, allowed: DragActions, image: Option<DragImage>)
pub fn dropped_object(&mut self, id: SurfaceId) -> Option<IDataObject>
// gevrek_shellkit::win::dnd
pub fn payload_of(object: &IDataObject, allowed: DragActions) -> DragPayload
pub fn kinds_of(object: &IDataObject) -> Vec<String>
pub fn object_for(payload: &DragPayload) -> windows::core::Result<IDataObject>
```

How it is used, in a surface's drawing:

```rust
// Once: the surface takes drops; until it says more, anything, as a copy or a move.
control.accept_drops(id, Some(DropAnswer::accept(DragActions::COPY_OR_MOVE)));

for event in control.drag_events(id) {
    match event {
        DragEvent::Entered { at, offered } => { /* show where it would land; offered.files */ }
        DragEvent::Moved { at } => {
            // What would be done here: nothing outside a zone, a move into this one.
            control.answer_drag(id, match zone_at(at) {
                Some(_) => DropAnswer::preferring(DragActions::COPY_OR_MOVE, DragAction::Move),
                None => DropAnswer::REFUSE,
            });
        }
        DragEvent::Left => { /* nothing is over the surface */ }
        DragEvent::Dropped { at, payload, action } => { /* do `action` with payload.paths */ }
        DragEvent::Ended { outcome } => { /* the drag this surface began is over */ }
    }
}

// On egui's `drag_started()` of an icon, with the button still held:
control.start_drag(id, DragStart { payload: DragPayload::files(paths), image: Some(picture) });
```

What to know when using it:

- **Places** are the surface's own points, as egui's, at every scale
  (Windows: pixels of the window's inside divided by the display's scale;
  Wayland: the surface's logical pixels as they come).
- **The answer is one frame behind.** A system wants to know at once what
  a drop would do; a drawing is asked later. So the source's pointer shows
  the surface's `standing` answer at first and what `answer_drag` said
  from the next frame on. The answer holds until another is given or the
  drag is gone; it is not kept for the next drag.
- **`Dropped.action` is the surface's to carry out, a move in full.** The
  source is told to delete nothing (on Windows a move is reported as "done
  by the target", see below), because the surface learns of the drop after
  the system's call has returned: a source that deleted on "move" would
  delete before anything was moved.
- **`Offered.payload`** is there on Windows from the moment a drag comes
  (the data can be read at once); on Wayland it is `None` and the paths
  are known at the drop, when the source is asked for them.
- **When a drag begins, the surface is told the button was let go and the
  pointer is gone** (egui's `PointerButton` released and `PointerGone`):
  the system takes the pointer for the drag and the real release never
  comes. The drawing that called `start_drag` must not take that release
  for the end of its own gesture; the end is `DragEvent::Ended`.
- **`DragOutcome::Taken { remove: true }`** (Windows only) means the
  target copied and left the originals to the source. Whoever acts on it
  should remove only what is still there, and to the recycle bin.
- **To the zones' `DropPayload`**: `paths`, `text` and `allowed` are the
  same fields (`allowed.copy/moving/link`); `items` is the zones' own.
  shellkit does not depend on `gevrek-zones`.
- **A link** exists on Windows only. A Wayland compositor carries copy and
  move; a drag that allows nothing but a link cannot begin there.

## Windows: OLE (`crates/shellkit/src/win/dnd.rs`)

- **The thread.** `OleInitialize` at the first `accept_drops` or
  `start_drag`, on the shell's thread, undone when the shell ends. A thread
  nobody set up becomes single-threaded; one that is so already
  (`gevrek_system`'s `Com`, which asks for the same kind) counts one more
  user; one that was made multi-threaded cannot be changed
  (`RPC_E_CHANGED_MODE`): then there is no drag and drop, said once in the
  log, and nothing else changes. `gevrek_system::com::Com` goes on working
  on the thread either way. What changes for the rest of the shell: from
  the first surface that asks, the thread is single-threaded for good,
  where before it was so only while a `Com` lived.
- **Incoming.** `RegisterDragDrop` with an `IDropTarget` (`implement`
  macro) per window, revoked before the window is destroyed. The target is
  called in the middle of the thread's message handling, so it touches
  only two thread-local lists: what each window would take, and what it
  was told (taken up by the host's next pass). It reads `CF_HDROP` (the
  wide form by `dnd::hdrop::decode`, the older form by `DragQueryFileW`),
  `Shell IDList Array` as bytes into `payload.native` (always, when
  present: things without a path are only there), `UniformResourceLocatorW`
  into `uris`, and `CF_UNICODETEXT` into `text` when there are no files.
  The effect answered is `negotiate(what the source allows, the keys,
  the surface's answer)`: Ctrl copy, Shift move, Ctrl+Shift or Alt link,
  none where a key asks for what cannot be done. The shell's
  `IDropTargetHelper` is told of everything, so Explorer's drag pictures
  show over a surface.
- **A move, as told to the source.** `Drop` answers `DROPEFFECT_NONE` and
  leaves `Performed DropEffect = 0` and `Logical Performed DropEffect =
  MOVE` in the object: "the target did it all". A copy or link is answered
  as itself.
- **Outgoing.** `DoDragDrop` with an `IDropSource` of ours. The data
  object is the caller's (`start_drag_of`) or made here for plain paths:
  **`SHCreateDataObject` with nothing in it**, into which `CF_HDROP` (and
  the paths as text) are put. Chosen over an `IDataObject` written by hand
  because the shell's drag helper keeps the picture in the object, in
  kinds of its own, and only an object that stores whatever it is given
  can hold them. The picture goes through
  `IDragSourceHelper::InitializeFromBitmap` (premultiplied, top row
  first). Escape cancels, a second button cancels, letting the button go
  drops; the pointers are the system's own.
- **The modal loop.** `DoDragDrop` returns when the drag is over. It is
  called from the shell's loop (never from inside a drawing), and the
  shell's whole pass (window news, messages of other threads, drawing) is
  made from inside it, each time Windows asks the source something
  (`QueryContinueDrag`, `GiveFeedback`), at most every 4 ms. So while a
  surface's drag is under way the surfaces go on being drawn and drops on
  the shell's own surfaces are told as any other; what does **not** happen
  until the drag ends: displays coming and going are not acted on, and a
  second `start_drag` waits. How often Windows calls the source while the
  pointer rests is not known for sure; if it calls only on moves, an
  animation stands still while the pointer does.
- **How it ended** is read from `DoDragDrop`'s result, the effect, and the
  two notes in the object (`dnd::outcome`).

## Linux: `wl_data_device` (`crates/shellkit/src/layer/dnd.rs`)

- Through SCTK's `data_device_manager`. The manager and `wl_shm` are bound
  when the shell starts (binding does nothing by itself); the seat's data
  device is made at the first `accept_drops` or `start_drag`.
- **Incoming.** On `enter` over a surface that takes drops: the kinds and
  the source's actions become `Entered`; the offer is accepted in
  `text/uri-list` (else `text/plain;charset=utf-8`, `text/plain`,
  `UTF8_STRING`, `STRING`), with `set_actions(what both allow, the one
  preferred)`; over a surface that takes none, it is refused. The
  compositor chooses the action. At the drop the kind is asked for and
  **read on a thread of its own**, which hands the bytes to the loop
  through a calloop channel; the Wayland dispatch never waits for a
  source. Then `Dropped`, `finish`, `destroy`. A source that sends nothing
  for 10 seconds is given up on (`Left`); at most 16 MiB are read.
  Addresses of this machine's files (`file:///`, `file://localhost/`,
  `file://<its name>/`) become `paths`, all others stay in `uris`.
- **Outgoing.** A data source offering `text/uri-list`,
  `x-special/gnome-copied-files`, `text/plain;charset=utf-8`,
  `text/plain`; `start_drag` with the number of the last press (button or
  finger) on a surface of the shell; the picture on a surface of its own
  (`wl_shm`, premultiplied; shown at its logical size through the
  viewporter, its hotspot through `wl_surface.offset`). Data is **written
  on a thread of its own**. `dnd_finished` is `Taken` with the action the
  compositor last named, `cancelled` is `Cancelled`.
- **Keys.** The protocol lets a source say its actions once, before the
  drag, so a drag out of a surface offers all it allows throughout, and a
  surface under a drag is not told which keys are held. Which action wins
  is the target's preference, unless the source's toolkit or the
  compositor narrows it.

## The compositor (`crates/backend-smithay`), not touched

What was read, and what follows for shellkit (line numbers as of this
writing; the files are being worked on):

- **A layer surface can be dropped on.** The pointer's focus is a
  `WlSurface` (`state.rs:2238`), found by `surface_under`
  (`input.rs:1503`), which knows layer surfaces of every layer
  (`input.rs:1546`, `1556`, `1567`); smithay's drag grab sends
  enter/motion/drop to whatever has that focus.
- **A layer surface can begin a drag.** smithay lets `start_drag` through
  when the pointer (or touch) has a grab with the given serial
  (`smithay-0.7.0/src/wayland/selection/data_device/device.rs:72`), and
  the compositor gives every press on a client a fresh serial through
  `pointer.button` (`input.rs:339`, `416`). The drag's picture is kept and
  drawn (`state.rs:2372`, `render.rs:404`).
- **X11 to Wayland exists** (`xwayland.rs:83`–`130`: an X11 program's drag
  becomes the compositor's own drag over Wayland surfaces), and Wayland to
  X11 too (`xwayland.rs:332`).

What was needed there for everything to behave. **All four are done
since** (the same day): see "Drags" in `milestone-3-surfaces.md`.

1. **Drags and layer surfaces above X11 windows.** `follow_drag_onto_x11`
   (`xwayland.rs:332`–`347`) and `follow_x11_drag` (`xwayland.rs:154`–`166`)
   ask `desktop.window_at` (`desktop.rs:659`), which knows windows only.
   Where a layer surface of the top or overlay layer lies over an X11
   window, a Wayland drag is told to the X11 window underneath as well as
   to the layer surface, and an X11 program's drag is not carried to the
   layer surface. Both should ask what `surface_under` finds. Zones on the
   bottom layer are not affected: there windows are above them and
   `window_at` is right.
2. **An X11 program's drag is always a copy.** `start_dnd` is given
   `DndAction::Copy` alone (`xwayland.rs:125`); a move asked for in the
   X11 program (Shift) arrives as a copy. Needs the X11 side's action
   (`XdndActionMove`) carried into the source's actions.
3. **Keys choose nothing.** Neither smithay's grab nor the compositor
   turns Ctrl/Shift into an action (Mutter does it in the compositor). A
   drag between two Wayland programs follows the keys only where the
   source's toolkit does so itself.
4. **The picture's hotspot.** The drag's picture is drawn with its corner
   at the pointer (`render.rs:408`–`415`); the offset a client sets
   (`wl_surface.offset`, the buffer's delta) is not added.

None of these stops a drag between a zone and a Wayland file manager.

## Tests

`cargo test -p gevrek-shellkit`: 57 on Windows, 17 on Linux (the pure
ones run on both).

- `dnd` (pure): the keys' meaning; `negotiate` as a table and over every
  combination; the two systems' counts of actions; when a drag of ours
  goes on, drops, cancels; how a drag's end is read; pixels to points at
  1, 1.25, 1.5, 1.75 and 2; a picture's logical size and its bytes;
  `file://` addresses both ways (spaces, `%`, `#`, `?`, non-ASCII, host
  forms, Windows drives and shared folders, `\\?\` paths, what is no
  address); `text/uri-list` (CRLF and LF, remarks, a zero byte); the kinds
  offered and what is sent and read in each; `CF_HDROP` written and read
  (wide letters, the two zeros, non-ASCII and a pair of surrogates, paths
  over 400 letters, an empty list, lists cut short, a head that points
  elsewhere, the older form).
- `win::dnd` (real OLE objects, nothing shown): paths, text and a link
  through the shell's data object and back, also read by `DragQueryFileW`;
  the helper's picture kept in the object; the notes about what was done;
  and the drop target called as Windows calls it (enter, over with keys,
  the surface's answer taking effect, a drop as a move and what the source
  is left to read, a drop the keys forbid).

Run on 2026-11-19, all without warnings of this crate's code:

- Windows: `cargo build -p gevrek-shellkit --all-targets`,
  `cargo test -p gevrek-shellkit` (57 passed),
  `cargo clippy -p gevrek-shellkit --all-targets`, `cargo check -p gevrek-shell`.
- Linux (WSL, Fedora): `cargo check -p gevrek-shellkit --all-targets`,
  `cargo test -p gevrek-shellkit` (17 passed),
  `cargo clippy -p gevrek-shellkit --all-targets`, `cargo check -p gevrek-shell`.

## Not verified

Everything that needs a pointer:

- Windows: that Explorer and other programs see the registered target and
  show the right pointers; the drag picture, both ways; that painting
  really continues inside `DoDragDrop` and how often; that Explorer, as
  the source of a move, deletes nothing after `DROPEFFECT_NONE` with the
  two notes (the convention is the documented one for a target that moves
  by itself, and is what the data-safe reading of it says); what Explorer
  leaves in the object when it is the target of a move (`remove`); a
  thread that some other part of the shell made multi-threaded first.
- Linux: the whole exchange with a compositor (only compiled and read
  against SCTK's and smithay's sources): acceptance, the chosen action,
  the pipe, `finish`; the picture and its place; a drag begun by touch;
  other compositors than gevrek's.
- No surface of the shell uses any of it yet, so nothing is known about
  how egui's own drag handling sits with the release that is told when a
  drag begins.

## Manual test checklist

Start `cargo run -p gevrek-shellkit --example dragdrop` (it prints every
event; left half prefers a copy, right half a move).

**Windows, with Explorer**

- [ ] Drag a file from Explorer over the surface: `entered` with the path
      already known, `moved` while it moves, Explorer's faded icon shows.
- [ ] Pointer: plus sign on the left half, plain move arrow on the right,
      one frame after crossing the middle.
- [ ] Hold Ctrl: copy everywhere. Shift: move everywhere. Ctrl+Shift or
      Alt: link. Let the key go: back to the half's own.
- [ ] Drop on the left: `dropped ... Copy` with the full path. Drop on the
      right: `Move`, **and the file is still in Explorer** (the example
      moves nothing; Explorer must not delete it).
- [ ] Drag out and back without dropping: `left`. Press Escape while over
      the surface: `left`.
- [ ] Several files, a folder, a path with non-ASCII letters, a path
      longer than 260 letters.
- [ ] From the desktop or "This PC": the recycle bin or a drive gives no
      path but `1 of the system's own`.
- [ ] Right-button drag from Explorer: it is told as any other (no menu
      of choices is shown; note what is expected here).
- [ ] Drag the example's button into an Explorer folder: the file
      `gevrek-dragdrop-example.txt` is copied there, `ended: Taken { Copy }`;
      with Shift: moved, and note what `remove` says.
- [ ] Drag it out and press Escape, or drop it on the taskbar's empty
      part: `Cancelled`. Press the right button while dragging: `Cancelled`.
- [ ] The picture (a teal card) follows the pointer over Explorer and over
      the surface itself, its middle under the pointer.
- [ ] While dragging out, the surface's line "over the surface at ..."
      keeps changing when the pointer is over it (it is drawn inside the
      drag), and after the drag the button is not stuck pressed.
- [ ] Drag the button onto the surface itself: `entered`, `dropped`, then
      `ended`.

**Windows, other programs**

- [ ] A browser: drag a link (address in `addresses`), selected text
      (`text`), a downloaded file from the downloads bar (`paths`).
- [ ] Drag the example's file into a browser's upload field, into a text
      editor (opens the file or inserts its path), into a mail.
- [ ] A program run as administrator: Windows does not let a drag pass
      from the shell into it (and may not the other way); nothing must
      break, and a drag out over it ends as `Cancelled`.

**Windows, displays**

- [ ] Two displays at different scales (100 % and 150 %): the place told
      matches the pointer on each (drop near a corner and compare with the
      surface's size in points); move the example to the other display
      and repeat; the dragged picture is the right size on both.

**Linux, gevrek's compositor**

- [ ] Nautilus, Dolphin, Thunar each: drag a file over the surface:
      `entered ... files: true`, `moved`; the file manager's pointer shows
      copy on the left and move on the right.
- [ ] Drop: `dropped` with the path (spaces and non-ASCII letters in the
      name), and the file manager is not left waiting (its drag ends).
- [ ] Drag out of the surface and away, and press Escape: `left`.
- [ ] Drag the example's button into each file manager's folder: the file
      arrives, `ended: Taken`. Onto the wallpaper or a panel: `Cancelled`.
- [ ] The picture follows the pointer (its corner at the pointer until the
      compositor adds the offset, see above).
- [ ] After a drag out, the button is not stuck pressed and the surface
      takes clicks at once.
- [ ] A browser (Firefox, Chromium, on Wayland): a link, selected text, a
      file from the downloads list; and the example's file into an upload
      field.
- [ ] A touch screen: begin the drag with a finger.
- [ ] A source that is killed between the drop and its data: `left` after
      ten seconds, and the shell goes on working meanwhile.

**Linux, X11 programs through Xwayland**

- [ ] Drag a file from an X11 file manager (e.g. `GDK_BACKEND=x11 thunar`)
      onto the surface: `entered`, `dropped` with the path; always a copy
      for now.
- [ ] Drag the example's button onto an X11 window: the file arrives.
- [ ] The same with the surface above an X11 window (the example is on the
      top layer): expected to go wrong until the compositor asks
      `surface_under` (point 1 above); note what happens.

**Linux, displays**

- [ ] Scales 1, 1.25, 1.5 and 2, and two displays with different ones: the
      place told matches the pointer; the picture is sharp and of the same
      logical size on each.
