#!/bin/sh
# Try gevrek on the hardware without logging out of the desktop you are in.
#
# Switch to a text console (Ctrl+Alt+F3), log in, and run this. gevrek takes
# the displays, keyboard and mouse while that console is in front; the
# other desktop keeps running on its own console and nothing of its session
# is touched (no systemd units, no D-Bus environment). Ctrl+Alt+F<n> goes
# back and forth.
#
#     scripts/try-console.sh              the installed build (~/.local/bin),
#                                         else target/release
#     scripts/try-console.sh --safe       without your configuration file
#     GEVREK_BIN=target/debug scripts/try-console.sh
#
# What starts: the window manager, the shell (panel, launcher: Super+D) and
# a terminal. Super+T is another terminal, Super+F1 the list of shortcuts.
# To leave: Super+Shift+E, or Ctrl+Alt+Backspace twice, or from another
# console `pkill gevrek-wl`.
#
# The log is ~/.local/state/gevrek/console.log (the last run) and is the
# first thing to read when the displays stay dark.
set -e
repo="$(cd "$(dirname "$0")/.." && pwd)"

if [ -n "$WAYLAND_DISPLAY" ] || [ -n "$DISPLAY" ]; then
    echo "This is a terminal of a desktop. Run it from a text console (Ctrl+Alt+F3)," >&2
    echo "or use scripts/run-nested.sh for gevrek as a window." >&2
    exit 1
fi

if [ -n "$GEVREK_BIN" ]; then
    case "$GEVREK_BIN" in /*) bin="$GEVREK_BIN" ;; *) bin="$repo/$GEVREK_BIN" ;; esac
elif [ -x "$HOME/.local/bin/gevrek-wl" ]; then
    bin="$HOME/.local/bin"
else
    bin="$repo/target/release"
fi
if [ ! -x "$bin/gevrek-wl" ]; then
    echo "No gevrek-wl in $bin: cargo xtask install --session" >&2
    exit 1
fi

terminal=
for candidate in kitty foot alacritty konsole gnome-terminal xterm; do
    if command -v "$candidate" >/dev/null 2>&1; then
        terminal="$candidate"
        break
    fi
done

log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/gevrek"
mkdir -p "$log_dir"
log="$log_dir/console.log"
echo "gevrek from $bin; the log is $log"

# The shell and the terminal are clients: started once gevrek is up.
PATH="$bin:$PATH" RUST_LOG="${RUST_LOG:-info}" \
    "$bin/gevrek-wl" --backend drm "$@" -- \
    sh -c "'$bin/gevrek-shell' & exec ${terminal:-true}" > "$log" 2>&1 || status=$?

echo "gevrek ended (${status:-0}). The end of its log:"
tail -n 15 "$log"
exit "${status:-0}"
