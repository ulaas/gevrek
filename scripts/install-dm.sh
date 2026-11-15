#!/usr/bin/env bash
# Install gevrek's display manager on this machine (resources/dm/README.md
# says what it is). It builds as you and asks for sudo only to put the
# files in place. Nothing is switched: the machine keeps the display
# manager it has until you enable this one (the last lines say how).
#
#     scripts/install-dm.sh            build and install
#     scripts/install-dm.sh --remove   take it away again (and give the
#                                      machine back a display manager first,
#                                      if this one was enabled)
#
# The programs go to /usr/local/bin (the greeter runs as a user of its own
# and cannot read your home), the unit to /etc/systemd/system, the PAM
# files to /etc/pam.d. The session a login opens is the one installed for
# the person (`cargo xtask install --session`), else /usr/local/bin's.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN=/usr/local/bin
PROGRAMS=(gevrek-dm gevrek-wl gevrek-lock gevrek)

if [ "${1:-}" = "--remove" ]; then
    if [ "$(readlink -f /etc/systemd/system/display-manager.service 2>/dev/null)" = /etc/systemd/system/gevrek-dm.service ]; then
        echo "gevrek-dm is this machine's display manager. Enable another one first, for example:" >&2
        echo "    sudo systemctl disable gevrek-dm && sudo systemctl enable plasmalogin" >&2
        exit 1
    fi
    sudo systemctl stop gevrek-dm 2>/dev/null || true
    for file in /etc/systemd/system/gevrek-dm.service /etc/pam.d/gevrek-dm /etc/pam.d/gevrek-greeter \
                /etc/sysusers.d/gevrek-dm.conf /etc/tmpfiles.d/gevrek-dm.conf "$BIN/gevrek-dm"; do
        [ ! -e "$file" ] || sudo rm -f "$file"
    done
    sudo systemctl daemon-reload
    echo "Removed. (Left in place: $BIN/gevrek-wl, gevrek-lock, gevrek, gevrek-session; the user gevrek-greeter"
    echo "and /var/lib/gevrek-greeter. Remove them by hand if nothing else uses them.)"
    exit 0
fi

[ "$(id -u)" != 0 ] || { echo "Run it as yourself: it asks for sudo where it needs it." >&2; exit 1; }
# The shared part of every login's PAM files has one name on Fedora and
# Arch and others on Debian and Ubuntu: the file that names this system's.
if [ -f /etc/pam.d/system-auth ]; then
    PAM_FILE="$REPO/resources/dm/pam-gevrek-dm"
elif [ -f /etc/pam.d/common-auth ]; then
    PAM_FILE="$REPO/resources/dm/pam-gevrek-dm.debian"
else
    echo "This system has neither /etc/pam.d/system-auth nor /etc/pam.d/common-auth:" >&2
    echo "write resources/dm/pam-gevrek-dm with this system's own names first." >&2
    exit 1
fi

echo "== building (release)"
(cd "$REPO" && cargo build --release -p gevrek-dm -p gevrek-smithay -p gevrek-lock -p gevrek)
for name in "${PROGRAMS[@]}"; do
    [ -x "$REPO/target/release/$name" ] || { echo "not built: $name" >&2; exit 1; }
done

echo "== installing (sudo)"
for name in "${PROGRAMS[@]}"; do
    sudo install -m 0755 "$REPO/target/release/$name" "$BIN/$name"
done
sudo install -m 0755 "$REPO/resources/gevrek-session" "$BIN/gevrek-session"
sudo install -m 0644 "$PAM_FILE" /etc/pam.d/gevrek-dm
sudo install -m 0644 "$REPO/resources/dm/pam-gevrek-greeter" /etc/pam.d/gevrek-greeter
sudo install -Dm 0644 "$REPO/resources/dm/sysusers.conf" /etc/sysusers.d/gevrek-dm.conf
sudo install -Dm 0644 "$REPO/resources/dm/tmpfiles.conf" /etc/tmpfiles.d/gevrek-dm.conf
sed "s|/usr/bin/gevrek-dm|$BIN/gevrek-dm|" "$REPO/resources/dm/gevrek-dm.service" \
    | sudo tee /etc/systemd/system/gevrek-dm.service >/dev/null
sudo systemd-sysusers gevrek-dm.conf
sudo systemd-tmpfiles --create gevrek-dm.conf
# (Where SELinux is on: gevrek-dm is labelled as a display manager, by a
# small module of the policy, and the places get the labels they have by
# the system's policy and by that module.)
if command -v semodule >/dev/null 2>&1 && command -v selinuxenabled >/dev/null 2>&1 && selinuxenabled; then
    sudo semodule -i "$REPO/resources/dm/gevrek-dm.cil" \
        || echo "The SELinux module was not installed: gevrek-dm runs as a service nobody confines." >&2
fi
if command -v restorecon >/dev/null 2>&1; then
    sudo restorecon -R "$BIN" /etc/pam.d/gevrek-dm /etc/pam.d/gevrek-greeter \
        /etc/systemd/system/gevrek-dm.service /var/lib/gevrek-greeter 2>/dev/null || true
fi
sudo systemctl daemon-reload

now="$(basename "$(readlink -f /etc/systemd/system/display-manager.service 2>/dev/null || echo none)" .service)"
cat <<TEXT

Installed, not enabled. This machine's display manager is still: $now

To start the machine into gevrek's login from the next boot:

    sudo systemctl disable $now
    sudo systemctl enable gevrek-dm
    sudo systemctl reboot

And back, from a text console (Ctrl+Alt+F3, log in there) if the login does not come up:

    sudo systemctl disable gevrek-dm
    sudo systemctl enable $now
    sudo systemctl reboot

What it says while it runs:  journalctl -b -u gevrek-dm     (the greeter's own lines: journalctl -b -t gevrek-dm)
TEXT
