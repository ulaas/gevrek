# What a gevrek session is made of

`cargo xtask install --session` puts these in place for one user and prints
the two commands that need root.

| File | Goes to | What it is |
|---|---|---|
| `gevrek-session` | `<prefix>/bin` | What the login manager runs: starts `gevrek.service` and waits for it |
| `systemd/gevrek.service` | `~/.config/systemd/user` | gevrek as the session's service (`gevrek-wl --session`) |
| `systemd/gevrek-safe.service` | same | The same without the user's configuration (`gevrek-session --safe`) |
| `systemd/gevrek-previous.service` | same | The build installed before the current one (`gevrek-session --previous`) |
| `systemd/gevrek-shutdown.target` | same | Takes the session's targets down when gevrek ends |
| `systemd/gevrek-polkit-agent.service` | same | The authentication agent that is installed (KDE's, Hyprland's, GNOME's, ...), until gevrek has its own |
| `gevrek.desktop` | `/usr/share/wayland-sessions` (root) | The entry in the login manager |
| `gevrek-studio.desktop` | `~/.local/share/applications` (every `cargo xtask install`) | The settings program in the launcher's list |
| `gevrek-portals.conf` | `/usr/share/xdg-desktop-portal` (root) | Which portal backend answers what |

In the units `@BIN@` is the folder of the installed programs and
`@PREVIOUS@` that of the build before; `xtask` fills them in.

The session's log: `journalctl --user -u gevrek -b`.
Ctrl+Alt+Backspace twice ends the session whatever the configuration says.
