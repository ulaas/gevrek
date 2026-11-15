# gevrek on a Linux machine

How to build gevrek's Wayland WM (`gevrek-wl`) on a Linux machine and run it
on that machine's own display. Linux is one of the systems gevrek is written
for (see the [README](../README.md)); this backend is younger than the
Windows one. It has run nested inside another session (WSLg on Ubuntu 24.04,
KDE Plasma on Fedora) and on the virtual hardware of a KVM machine, not yet
on physical hardware from a text console: the hardware part below is what was
done in that virtual machine, written for a real one.

## Build

```sh
# Rust: https://rustup.rs
# Fedora:
sudo dnf install gcc pkgconf-pkg-config wayland-devel libxkbcommon-devel mesa-libEGL-devel \
    libglvnd-devel systemd-devel libinput-devel mesa-libgbm-devel libseat-devel libdrm-devel \
    pam-devel pipewire-devel clang
# Ubuntu:
sudo apt-get install -y build-essential pkg-config libwayland-dev libxkbcommon-dev \
    libegl1-mesa-dev libgles2-mesa-dev libudev-dev libinput-dev libgbm-dev libseat-dev libdrm-dev libpam0g-dev \
    libpipewire-0.3-dev clang
git clone https://github.com/ulaas/gevrek.git && cd gevrek
cargo build --release -p gevrek-smithay -p gevrek-studio --bins
cargo build --release --bin gevrek          # the command-line client
cargo test --workspace --exclude gevrek-win32   # optional: some 670 tests, nothing is drawn
```

The binaries are `target/release/gevrek-wl`, `gevrek` and `gevrek-studio`.

## What it needs when it runs

```sh
sudo apt-get install -y seatd xwayland foot fontconfig fonts-dejavu-core   # Ubuntu
sudo dnf install seatd xorg-x11-server-Xwayland foot fontconfig dejavu-sans-fonts   # Fedora
```

- A seat: either a logind session (log in on a text console) or `seatd`
  running (`sudo systemctl enable --now seatd`, and your user in the `seat`
  or `video` group; `LIBSEAT_BACKEND=seatd` picks it).
- `foot` is only the terminal used below; any Wayland program will do.
- `xwayland` for X11 programs (off with `--no-xwayland`).

## Try it inside your current session first

From a terminal of any Wayland or X11 desktop:

```sh
scripts/run-nested.sh
```

This builds gevrek and runs all of it as a 1920x1080 window: the WM, the panel
(`gevrek-shell`), the editor, the list of shortcuts, and a terminal of the
system. It is the way to try and to test a change. The configuration is
`target/nested/config.json`, made from the built-in example Aurora the first
time and kept afterwards (`--fresh` writes it anew, `GEVREK_EXAMPLE` and
`GEVREK_CONFIG` choose another, `GEVREK_SIZE` another size, `--bare` starts
the WM and the terminal only). The head of the script has the rest.

**Keys.** The host session takes its own shortcuts first (KDE: Meta opens the
launcher, Meta+1, Meta+Tab, ...). gevrek therefore asks the host to send it
every key while its window has the keyboard (the `keyboard-shortcuts-inhibit`
protocol), and says in its log whether the host agreed. The bindings of the
configuration then work as they are, with Super as on the hardware: no
other modifier is needed for the nested session. To leave the window, click
another window of the host. `--host-shortcuts` turns this off. The keys
themselves are in the README ("The standard keys"); the script adds Super+E
(the editor). What the host
does below its shortcuts still happens (Ctrl+Alt+F<n> switches consoles).

**Keyboard and pointing devices** come from `desktop.input`:

```json
{ "input": {
    "keyboard": { "layouts": [{ "layout": "us" }, { "layout": "tr" }],
                  "options": ["grp:alt_shift_toggle"], "numlock": true,
                  "repeat_delay_ms": 400, "repeat_rate": 30 },
    "touchpad": { "tap": true, "natural_scroll": true },
    "mouse": { "speed": 0.0, "accel_profile": "flat" } } }
```

Without layouts the system's are used (`XKB_DEFAULT_*`). A binding is a key
by its place on the keyboard: Super+T is the same key on every layout.
Changes apply at once. The device settings need the hardware backend (in a
nested window the host owns the devices).

**Mouse.** With Super held, the left button moves the window under the
pointer and the right button resizes it from the nearest corner; a floating
window stays where it is dropped, a tiled one swaps places with the window
it is dropped on or goes back. A host
such as KDE uses Super with the mouse for its own windows, so the script
also takes Alt (`--drag-modifier super,alt`; the option takes `super`, `alt`,
`ctrl`, or several).

By hand, with any configuration:

```sh
target/debug/gevrek-wl --size 1920x1080 -- foot
```

gevrek opens as a window; the program after `--` is started in it. `--size`
is the size of that window in device pixels (1280x800 logical pixels without
it). Programs started from a terminal inside gevrek are its clients;
`gevrek-shell` and `gevrek-studio` started there show the panel and the
settings program.

## Displays

`desktop.outputs` says how the displays are set up, by the name of their
connector (`gevrek state` lists them):

```json
"outputs": [
  { "name": "DP-1", "mode": { "width": 2560, "height": 1440, "refresh": 144 },
    "position": [0, 0], "primary": true },
  { "name": "DP-2", "position": [2560, 0], "scale": 1.25 },
  { "name": "HDMI-A-1", "enabled": false }
]
```

Without an entry a display has its preferred mode, to the right of the
others. A change is applied while gevrek runs. A new place or scale simply
is; a new mode, or a display switched on or off, is **on trial**: unless
you keep it within 15 seconds it is taken back, in the file too, so a mode
the display cannot show does not leave you in front of a dark screen.

    gevrek output-trial keep      # the new settings stay
    gevrek output-trial revert    # back at once

(Turning the picture, `transform`, and `variable_refresh` are read and not
applied yet. While a display that is there is asked for either, gevrek says
so: `gevrek health` has a notice, and the panel a mark that shows it in full
when pressed.)

## Launcher, notifications, lock screen

`gevrek-shell` runs with the session (its unit is started with gevrek's).
Super+D or Super+Space opens the launcher: type a few letters of an
application and press Enter; after `>` it lists gevrek's own commands.
It is also the notification service: what programs send (`notify-send`)
shows in the top right corner.

Super+Alt+L locks the screen with `gevrek-lock`, which asks for your
password through the system's own check (PAM). Building it needs the PAM
development files (`pam-devel` on Fedora, `libpam0g-dev` on Debian and
Ubuntu). If `gevrek-lock` is not installed, another locker that is
(swaylock, hyprlock, gtklock, waylock) is used.

## Wallpaper

```json
"wallpaper": {
  "image": "~/Pictures/hills.jpg", "fit": "fill", "color": [18, 18, 23],
  "outputs": [{ "name": "DP-2", "image": "~/Pictures/other.png", "fit": "centre" }]
}
```

A PNG, JPEG or WebP file; `fit` is `fill`, `fit`, `stretch`, `centre` or
`tile`; the colour shows where the picture does not. gevrek draws it
itself, so no wallpaper program is needed. The studio has a page for it,
and there is a command:

```sh
gevrek wallpaper                                  # what is set, per display
gevrek wallpaper --json                           # the section and the displays
gevrek wallpaper set ~/Pictures/hills.jpg         # on every display
gevrek wallpaper set other.png --output DP-2 --fit centre
gevrek wallpaper clear --output DP-2              # DP-2 shows the common one again
gevrek wallpaper clear                            # the colour only
gevrek wallpaper apply section.json               # the whole section (- reads standard input)
```

`set` without `--output` makes the picture every display's: displays
that had one of their own lose it. `apply` takes a JSON object shaped
like the section above and puts it in its place, so several displays
change at once; a program that sets wallpapers (walle) uses that.

## Tray

Programs that keep an icon on the panel while they run in the background
(StatusNotifierItem: most do, through libappindicator, Qt or their own
code) show it at the right of the panel, beside the modules. A press on
the icon is passed to the program (most open their window); the other
button opens the program's menu, and a press on a line is passed on.

This is a transitional tray, until the full one is written. It is not a
module: it cannot be placed or switched off in the configuration, and it
is there whenever a program has an icon. A submenu's lines are shown in
the menu itself, under its name. Menus have no pictures and no keys, the
wheel over an icon does nothing, and icons of the older kind (XEmbed, as
some X11 programs still have) do not show. With `--no-panel` there is no
tray.

## Screenshots and recording

Programs that take pictures of the screen themselves work: `grim` (with
`slurp` for a part of the screen), `wf-recorder`, and OBS with its wlroots
capture. gevrek speaks the protocol they use (wlr-screencopy), with
shared-memory buffers.

Sharing the screen from a browser or a meeting program goes through the
desktop portal, which gevrek's shell answers itself: the program asks, a
chooser shows the displays and the windows, and what you choose is handed
on as a PipeWire stream. It needs `xdg-desktop-portal` and PipeWire
running (both are there on a usual desktop installation). While something
is shared a red mark stands on the panel; a press on it ends the sharing.

## Input methods

Programs that turn what is typed into other text (fcitx5, IBus: Chinese,
Japanese, Korean and more) work through the Wayland protocols for them
(text-input v3, input-method v2, and the virtual keyboard). Start the
input method with the session, as an entry of `desktop.startup` in the
configuration (the studio's page for it), for example the command
`fcitx5`. Its list of candidates is drawn under the text cursor of the
window that is typed into. Programs that speak X11 only need the input
method's own way for X11 (`XMODIFIERS=@im=fcitx` in
`desktop.session.environment`).

## Log into it

On Fedora, as a package:

```sh
cargo xtask rpm
sudo dnf install target/rpm/RPMS/x86_64/gevrek-*.rpm
```

That puts the programs in `/usr/bin`, the session's units and its entry
for the login manager, the portal files and the lock screen's PAM file in
the system's places; `sudo dnf remove gevrek` takes it all away again.
gevrek is then in the login manager's list of sessions.

Two more files build the package from source, for a repository:

- `packaging/rpm/gevrek-source.spec` for COPR ("Build from SCM", with
  "Enable internet access during builds" on: cargo fetches the crates
  while it builds). `--define "gevrek_commit <commit>"` names the commit
  to build; without it the branch `master` is built.
- `packaging/arch/PKGBUILD` for Arch (`makepkg -si` in that folder). It
  has not been built on an Arch machine yet.

Or for one user, without a package:

```sh
cargo xtask install --session
```

builds a release, puts the programs in `~/.local/bin`, the session's systemd
units in `~/.config/systemd/user`, and prints two `sudo install` lines for
the files only root can place: the entry in the login manager
(`/usr/share/wayland-sessions/gevrek.desktop`) and which portal backend
answers what (`gevrek-portals.conf`). After that gevrek is in the login
manager's list. `resources/README.md` says what each file is.

- gevrek runs as the service `gevrek.service` of the user; its log is
  `journalctl --user -u gevrek -b`.
- It tells systemd and D-Bus the display once it is up, so portals, the
  authentication agent, autostart entries and anything started over D-Bus
  find it. The agent that is installed (KDE's, GNOME's, ...) is started
  for the session until gevrek has its own.
- **Ways out:** `Super+Shift+E` quits. **Ctrl+Alt+Backspace twice** ends the
  session whatever the configuration says. Ctrl+Alt+F<n> is another
  console. From the login manager, `gevrek-session --safe` (its own entry
  can be made) ignores the configuration file, and `gevrek-session
  --previous` runs the build that was installed before the current one.
- The session you are logged into runs the installed build. Work on gevrek
  goes on in a nested one (`scripts/run-nested.sh`, its own instance),
  which gets its keys from the session; Super+Escape takes them back.

This has not been done on a physical machine yet (see below).

### gevrek's own login (`gevrek-dm`)

For a machine that runs gevrek and nothing else there is a display
manager of gevrek's own: the first thing on the display after the system
has started is gevrek's login (the lock screen's look, with a field for
the name, which it remembers), and what it opens is gevrek. It has no
list of sessions. `resources/dm/README.md` says what it is made of and
where its files go; the packages install them. It is used in place of
the display manager the machine has, and switched back the same way:

    sudo systemctl disable gdm        # or sddm, lightdm, greetd
    sudo systemctl enable gevrek-dm

The login is on the first console and the desktop on the second; when the
session ends, or dies, the login is shown again. A login that does not
come up three times gives the first console back to a text login. It has
been run in a virtual machine and on one physical machine. Under SELinux it is labelled as a display manager (a
small module of the policy, installed by the script), so a session runs
under its person's own label.

### From the boot loader to the login without a seam

gevrek has a boot splash of its own for plymouth: the mark on black, and
when the login is ready the mark goes into black and the login comes out
of it; no text console and no change of the display's mode in between.
`scripts/install-plymouth.sh` installs it and says what else the machine
needs for that (`--check` only says it, `--remove` takes it away);
`resources/plymouth/README.md` has how it is made and what cannot be
helped.

## Run it on the hardware

The first time on a machine, try it beside the desktop you are logged
into, before logging in to it. Switch to a text console (Ctrl+Alt+F3), log
in, and:

```sh
cd gevrek
scripts/try-console.sh
```

That runs the installed build (else `target/release`) on the displays with
your configuration, the shell and a terminal, and touches nothing of the
other desktop's session: Ctrl+Alt+F1 or F2 is that desktop again, as it
was. The log is `~/.local/state/gevrek/console.log`. (`gevrek-session`
itself refuses to start while another graphical session of yours runs: both
would share `graphical-session.target`, and the end of one would take the
other's services down.)

By hand:

```sh
RUST_LOG=info target/release/gevrek-wl --backend drm -- foot > wm.log 2>&1
```

- To leave: Ctrl+Alt+F<n> switches to another console (gevrek pauses and
  resumes when you come back); `pkill gevrek-wl` from there ends it the
  ordinary way.
- If nothing appears, `wm.log` says why (no seat, no display device, ...).

From a terminal inside gevrek:

```sh
target/release/gevrek state            # monitors, workspaces, windows
target/release/gevrek capabilities     # backend "smithay", no Windows-only features
target/release/gevrek-studio &
```

The editor connects to the running WM; its Configure page has a section
"Wayland session" (Xwayland, cursor, output scale, a scale per output by
connector name such as `eDP-1`). Those are read when the WM starts.

## What to look at on a real machine

These have only been seen on virtual hardware:

- A program that draws with the graphics card (a browser, `glxgears`,
  a game): `wm.log` then has "A client draws with the graphics card".
- A touchpad: three-finger swipes run the gesture profiles of the
  configuration (`desktop.gestures`).
- A second display plugged in and unplugged while it runs.
- Switching consoles away and back.
- A high-density display: `desktop.platform.wayland.output_scale`, or
  `output_scales` for one output.

Not written: several graphics cards at once, touch screens, tablets as
tablets. The drawn effects (shadow, glow, gradient border, tint) are drawn, by
the same code as on Windows (a picture around each window), and the
focused window's gradient turns as there. One thing is still Windows-only:
a window's own content is not cut to the rounded corners of its border
(that comes with the shader pipeline). The panel is `gevrek-shell`'s, a layer surface that asks for its own room
where `desktop.shell.panel` says.

## The automated checks

`cargo xtask check` runs the tests of every crate. The Wayland backend is
tested without a display: real Wayland clients in the test process, and
nothing drawn. CI runs them on every push.
