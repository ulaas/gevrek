# gevrek's boot splash

A theme for plymouth, so that a machine that runs gevrek shows gevrek from
the boot loader on: the mark on black with five bars under it (one lit,
going round), and when the login is ready everything goes into black, out
of which gevrek's login comes. With gevrek's display manager
(`resources/dm`) there is no text console, no change of the display's
mode and no cut between the splash and the desktop.

What is here, and where a package puts it:

| File | Installed as |
|---|---|
| `gevrek/gevrek.plymouth` | `/usr/share/plymouth/themes/gevrek/gevrek.plymouth` |
| `gevrek/*.png` | `/usr/share/plymouth/themes/gevrek/` |

On a machine without a package, `scripts/install-plymouth.sh` puts the
theme in place, makes it the one in use and makes the initial ramdisks
again (the splash starts from there); `--remove` brings back the theme
from before. `scripts/install-plymouth.sh --check` changes nothing and
says what stands between this machine and a start without a seam.

A package only installs the files. To use them:

    sudo plymouth-set-default-theme gevrek -R

## How it is made

It is a theme for plymouth's `two-step` module, the one Fedora's own
splash uses, so nothing has to be installed for it that a Fedora does not
have.

- `throbber-NNNN.png` are shown in a loop while the system starts. The
  mark is in these pictures, not a picture of its own under them, because
  of the next point.
- `animation-NNNN.png` are shown once when the splash is told to end:
  two thirds of a second in which the mark and the bars go into black.
  A picture that stood still under them could not go with them.
- `lock.png`, `entry.png`, `bullet.png`, `capslock.png` are the field for
  the password of an encrypted disk.
- The ground is black, not the brand's molasses: black is what the
  firmware leaves on the display and what the login begins with, and a
  ground of another colour would be a step at both ends.

The pictures are drawn by `scripts/make-plymouth-theme.py` from the
brand's files (`assets/brand`) and kept here, so that a package needs
neither ImageMagick nor Python. Run it again when the mark changes.

## The hand-over to the login

1. The display manager's unit takes the place of `plymouth-quit.service`,
   which would end the splash and put the text console on the display.
2. When the service starts it tells the splash to let go of the display
   (`plymouth deactivate`). The splash shows its end, stops drawing and
   leaves black there.
3. The greeter's gevrek takes the display as it is (it sets no mode until
   its first frame, and begins in black).
4. When the login has drawn, it says so to the service, which ends the
   splash with its picture kept (`plymouth quit --retain-splash`). Not
   before: while no gevrek has the display's device open, the kernel
   would put its own console back when the splash closes it.
5. The login comes out of the black.

A greeter that never says that it is shown is not waited for longer than
twenty seconds, and a login that cannot start at all ends the splash the
plain way, so that the text login is there.

## What a machine needs besides

`scripts/install-plymouth.sh --check` looks for these:

- `rhgb quiet` on the kernel's command line (no splash without the
  first; the kernel's lines on the display without the second).
- plymouth's units not masked.
- gevrek's display manager as the login, in a build that takes the
  splash over (the other display managers end the splash their own way).
- The boot loader out of the way: its menu hidden
  (`GRUB_TIMEOUT_STYLE=hidden`; Esc or Shift still shows it) and its
  output on the firmware's console (`GRUB_TERMINAL_OUTPUT=console`,
  Fedora's own setting). A GRUB that draws for itself (`gfxterm`, a
  theme) sets the display's mode and clears it before the kernel starts,
  also with the menu hidden.

## What cannot be helped from here

- **Before the kernel.** With UEFI and the boot loader out of the way,
  the firmware's picture stays until the splash's first frame; the
  splash does not come in softly (the module has no beginning to show,
  only an end). With a BIOS, or a boot loader that shows its menu, that
  part is theirs.
- **A graphics driver that is loaded late.** The splash starts on the
  firmware's framebuffer and moves to the card's own driver when that is
  there. Where the driver sets the mode anew at that moment, the display
  blanks once. Drivers in the initial ramdisk (Intel, AMD) do it before
  anything is shown; NVIDIA's own driver is usually loaded from the
  disk, after the splash has begun.
- **The time.** The splash's end is waited for: the loop is shown to its
  end and then the two thirds of a second into black, so the login is
  there up to two seconds later than it could be.

## Checked

In a virtual machine: a whole start is filmed from outside the
machine, some ten pictures a second, and from the splash's first picture
to the login there is the splash, its going into black over many
pictures, black, and the login coming out of it over many pictures; no
text and no other mode. Not yet on a physical machine.
