# Games under gevrek: what is learned from whom

The plan for gevrek asks
that Steam and its games run under gevrek as well as they run anywhere on
Linux, and that the projects which have proven this be looked at first.
This is the list of those projects: what each does that matters here, what
gevrek has taken from it, and what its licence allows.

**No code of any of them is in gevrek so far.** What is written below as
"taken" is a way of doing something, read in their sources or their
documentation and written anew here. Where code is ever copied, its notice
goes with it into the file it is copied to and into this list.

gevrek itself is under the MIT licence (`LICENSE`). A project whose
licence asks that whatever takes its code be under that same licence (the
GPL ones below) can be learned from, and not copied from.

| Project | Licence | May its code be taken? |
|---|---|---|
| gamescope (Valve) | BSD 2-Clause | Yes, with its notice |
| Smithay (the library gevrek's compositor is built on, its `xwm` among it) | MIT | Yes, with its notice; it is a dependency already |
| wlroots | MIT | Yes, with its notice |
| xwayland-satellite | MPL 2.0 | File by file: a file taken stays under the MPL |
| niri | GPL 3.0 or later | No: ways of doing things only |
| cosmic-comp | GPL 3.0 only | No: ways of doing things only |

## gamescope

A compositor for one game: it gives the game an XWayland of its own, a
resolution of its own (scaled to the display's), a frame limiter, and
passes the result on, on a Steam Deck to the display and elsewhere as one
window of whatever compositor it runs in.

- **Used as it is.** A program can be started inside it from the launcher
  (Shift+Enter, where gamescope is installed: `gamescope -f -- <program>`).
  Its window is then an ordinary Wayland window to gevrek, which fills the
  display and is treated as a fullscreen game (the session stays awake,
  its picture may go to the display as it is).
- **What it asks of its host** (read from its Wayland backend,
  `src/Backends/WaylandBackend.cpp`, on 2026-11-13): it must have `xdg_wm_base`, linux-dmabuf, viewporter,
  presentation-time, the relative pointer, pointer constraints, `wl_shm`
  and `wl_seat`, all of which gevrek has. Where they are there it uses
  single-pixel buffers, fractional scale, the clipboard (gevrek has
  these) and colour management for HDR (gevrek has not: the library has
  none yet). It does not use tearing-control as a client (its Wayland
  backend says it cannot tear), nor FIFO, commit-timing or the shortcuts
  inhibitor. So it should run under gevrek as it is, without HDR; it has
  not been started there yet (it wants a graphics card's buffers, which
  the checks' gevrek has not).
- **Taken as a way of doing things:** that a window which covers a display
  keeps the machine awake while it does (games played with a controller
  move neither key nor mouse); that an X11 window which takes its frame
  off and makes itself as large as the display is fullscreen without
  having said so.
- **Not taken:** its own XWayland handling (gevrek's is Smithay's `xwm`),
  its scaling filters, its HDR path.

## Smithay

gevrek's compositor is a Smithay compositor; nothing is "taken", it is
used. What bounds gevrek because Smithay 0.7 bounds it:

- **Tearing.** The library flips a display's picture only in step with the
  display; there is no flip that does not wait. `wp-tearing-control` is
  therefore not offered: it would promise what cannot be done.
- **Colour management and HDR.** Not in the library yet.
- **X11 windows and their surfaces.** The library keeps the surfaces of
  X11 windows by the numbers Xwayland gives them and does not forget them
  when Xwayland ends; gevrek makes that state anew when it starts Xwayland
  again (found 2026-11-14, nested check XR1).
- **The renderer in memory** (the checks') draws a picture without an
  alpha part over black when its window is less than solid. On a graphics
  card it is blended.

## niri

A scrolling compositor on Smithay, with XWayland through
xwayland-satellite.

- **Learned:** leaving overlay planes alone (NVIDIA's driver reports them
  in an order that puts windows above or below where they belong: gevrek's
  `FRAME_FLAGS` allow the primary plane and the pointer's plane only);
  the strip of columns whose view is a place in pixels and follows fingers
  on a touchpad (gevrek's scrolling layout was written from the idea, with
  gevrek's own containers and layouts); a wheel with the modifier moving
  along the strip.
- **Not copied:** any of its code (GPL).

## cosmic-comp

System76's compositor, on Smithay, with XWayland inside the compositor as
gevrek has it.

- **Learned:** an X11 program may hold the keyboard through the
  `xwayland-keyboard-grab` protocol only while its window has the focus
  (gevrek: nested check XG1); X11 programs at the display's real pixels by
  telling Xwayland a scale and translating coordinates at the window
  manager's side. The second is not done in gevrek: X11 programs are
  scaled up on a display that is not at scale 1.
- **Not copied:** any of its code (GPL).

## wlroots

The library sway, river, labwc and others are built on; gamescope uses it
too.

- **Learned:** which X11 window kinds are not ordinary windows and float
  (dialog, utility, splash, toolbar) and that a window whose smallest size
  is its largest is a dialog whatever it says; `_NET_WM_BYPASS_COMPOSITOR`
  as a hint only (a compositor that scans a covering window out directly
  has nothing to bypass).
- **Not copied** so far, though its licence would allow it.

## xwayland-satellite

XWayland outside the compositor: a program of its own that is an X11
window manager on one side and an ordinary Wayland client on the other.

- **Learned:** what it would give gevrek (an Xwayland that dies does not
  take anything of the compositor with it; no X11 code in the compositor)
  and what it would cost (X11 windows are then Wayland windows of one
  program: the window manager's rules see less of them; the X11 clipboard
  and drag and drop are its business). gevrek keeps Xwayland inside for
  now, and starts it again when it ends.

## Where gevrek stands (2026-11-14)

Done: every form
of fullscreen an X11 game uses; the keyboard grab; pointer constraints and
the relative pointer for X11 programs; frames paced by the display (FIFO,
commit-timing, presentation-time); variable refresh rate per display; a
covering window's picture to the display's primary plane, counted per
display in `gevrek health`; the session awake while a window covers a
display; Steam's windows and games floating; a program started inside
gamescope from the launcher; Xwayland started again when it ends.

Open: tearing and HDR (the library); X11 programs at the display's real
pixels; `_NET_WM_BYPASS_COMPOSITOR`; windows that resize the root; another
resolution without a mode change, seen with a game; the Steam overlay and
Big Picture seen with Steam.

Not done by anybody yet: Steam, a native game, a Proton game and a game
inside gamescope run on the first machine with a list of what was tried.
That needs the person at the machine.
