# gevrek's display manager

`gevrek-dm` is the login for a machine that runs gevrek and nothing else:
from the first frame after the system has started there is gevrek, it asks
for a name and a password, and the desktop it opens is gevrek. It has no
list of sessions and starts no other desktop.

What is here, and where a package puts it:

| File | Installed as |
|---|---|
| `gevrek-dm.service` | `/usr/lib/systemd/system/gevrek-dm.service` |
| `pam-gevrek-dm` | `/etc/pam.d/gevrek-dm` |
| `pam-gevrek-greeter` | `/etc/pam.d/gevrek-greeter` |
| `sysusers.conf` | `/usr/lib/sysusers.d/gevrek-dm.conf` |
| `tmpfiles.conf` | `/usr/lib/tmpfiles.d/gevrek-dm.conf` |

The programs `gevrek-dm`, `gevrek-wl`, `gevrek-lock` and `gevrek-session`
are in one folder (the service looks for the others beside itself).

On a machine without a package of gevrek, `scripts/install-dm.sh` builds
it and puts the files in place (under `/usr/local/bin` and `/etc`), and
`scripts/install-dm.sh --remove` takes them away again.

To use it instead of the display manager a machine has:

    systemctl disable gdm        # or sddm, lightdm, greetd
    systemctl enable gevrek-dm

and back the same way. Nothing is switched by installing the files.

How it works, in short: the service runs as root and draws nothing. It
starts gevrek's compositor as the user `gevrek-greeter` on the first
console, with `gevrek-lock --greeter` in it. That screen sends the name
and the password to the service (a socket only the greeter's user can
open); the service has PAM check them, opens the session, starts
`gevrek-session` as the person on the second console and shows that
console. When the session ends, or dies, the first console is shown
again: the greeter was never ended. A greeter that does not come up three
times gives the console back to a text login.

A boot splash (plymouth) that is running when the service starts is taken
over from, not cut off: the service tells it to let go of the display
before the greeter starts, the greeter's gevrek begins in black and says
when its login is drawn, and only then is the splash ended, with its
picture kept. The unit takes the place of `plymouth-quit.service` for
that. gevrek's own splash, which ends in black, is in
`resources/plymouth`.

The greeter types on the keyboard layout of the system
(`localectl set-x11-keymap`, kept in `/etc/vconsole.conf`).
