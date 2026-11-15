#!/usr/bin/env bash
# Build gevrek and run the whole of it as a 1920x1080 window inside the
# current Wayland (or X11) session: the WM (gevrek-wl), the bar
# and launcher (gevrek-shell), the settings (gevrek-studio), the list of shortcuts, and a
# terminal to start programs from.
#
#     scripts/run-nested.sh                 # everything
#     scripts/run-nested.sh --bare          # the WM and the terminal only
#     scripts/run-nested.sh --fresh         # write the configuration anew
#     scripts/run-nested.sh -- konsole      # another program than a terminal
#
#     GEVREK_SIZE=2560x1440     the size of the window, in device pixels
#     GEVREK_EXAMPLE=paper      the built-in example the configuration is
#                               made from (`gevrek example` lists them)
#     GEVREK_CONFIG=my.json     use this configuration file instead
#     GEVREK_INSTANCE=dev       the instance name (its control socket)
#
# The configuration is target/nested/config.json: made from the example the
# first time, then kept, so what the editor saves is still there next time.
# gevrek asks the host session for every key while its window has the
# keyboard, so the bindings work as they are (Super+F1: the shortcuts).
# Super+T starts a terminal; the script adds Super+E for the editor.
# Click another window of the host to leave.
#
# Mouse: with Super or Alt held, the left button moves the window under the
# pointer and the right button resizes it. (Alt as well, because a host such
# as KDE uses Super with the mouse for its own windows. On the hardware it
# is Super alone; GEVREK_DRAG_MODIFIER=super makes it so here.)
#
# Keys and mouse without a person (for checks; this session accepts it):
#     GEVREK_INSTANCE=dev target/debug/gevrek input key Super+T
#     GEVREK_INSTANCE=dev target/debug/gevrek input drag 400 300 800 500 --hold alt
#
# Logs: target/nested/*.log. A picture of what gevrek shows:
#     touch target/nested/frame.ppm.request    # writes target/nested/frame.ppm
#
# Close the window or press Ctrl+C here to stop everything.
set -e
cd "$(dirname "$0")/.."

# Its own instance: its own control socket, so it runs beside a gevrek you
# are logged into, and the session's `gevrek` commands do not reach it.
# From another terminal: GEVREK_INSTANCE=dev target/debug/gevrek state
export GEVREK_INSTANCE="${GEVREK_INSTANCE:-dev}"

size="${GEVREK_SIZE:-1920x1080}"
example="${GEVREK_EXAMPLE:-aurora}"
work=target/nested
config="${GEVREK_CONFIG:-$work/config.json}"
bare=
fresh=
while [ $# -gt 0 ]; do
    case "$1" in
        --bare) bare=1 ;;
        --fresh) fresh=1 ;;
        --) shift; break ;;
        *) echo "unknown argument $1 (see the head of $0)" >&2; exit 2 ;;
    esac
    shift
done
if [ $# -eq 0 ]; then
    for terminal in foot kitty alacritty konsole gnome-terminal xterm; do
        if command -v "$terminal" >/dev/null 2>&1; then
            set -- "$terminal"
            break
        fi
    done
fi

cargo build -p gevrek-smithay -p gevrek -p gevrek-studio -p gevrek-shell -p gevrek-lock --bins
bin="$PWD/target/debug"
mkdir -p "$work"

if [ -z "$GEVREK_CONFIG" ] && { [ -n "$fresh" ] || [ ! -f "$config" ]; }; then
    "$bin/gevrek" example "$example" --out "$config"
fi

# One key more for the test session: Super+E starts the studio (Super+T, a
# terminal, is among the standard keys). Added once; left alone when Super+E
# is already taken.
if [ -z "$GEVREK_CONFIG" ]; then
    python3 - "$config" "$bin/gevrek-studio" "$PWD/$config" <<'PY'
import json
import sys

path, editor, editor_config = sys.argv[1:4]
config = json.load(open(path, encoding="utf-8"))
bindings = config["desktop"]["bindings"]


def plain_super(binding, key):
    return (
        binding["key"].upper() == key
        and binding.get("super", binding.get("win"))
        and not (binding["ctrl"] or binding["alt"] or binding["shift"])
    )


if not any(plain_super(binding, "E") for binding in bindings):
    # A copy of an existing binding has every field the schema wants.
    binding = dict(bindings[0])
    binding.update({
        "name": "Settings",
        "key": "E",
        "ctrl": False,
        "alt": False,
        "shift": False,
        # It finds the window manager it was started by (the instance is
        # in its environment).
        "command": {"command": "exec", "program": editor, "args": []},
    })
    binding["super" if "super" in binding else "win"] = True
    bindings.append(binding)
    json.dump(config, open(path, "w", encoding="utf-8"), indent=2)
PY
fi

if "$bin/gevrek" health >/dev/null 2>&1; then
    echo "A gevrek WM already answers as instance '$GEVREK_INSTANCE': stop it first, or choose another GEVREK_INSTANCE." >&2
    exit 1
fi

pids=()
stop() {
    trap - EXIT INT TERM
    # The bar's starter is no Wayland client: it would outlive the WM.
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap stop EXIT INT TERM

# The size is in device pixels whatever the scale of the host's output.
GEVREK_WL_SCREENSHOT="$PWD/$work/frame.ppm" \
    "$bin/gevrek-wl" --size "$size" --drag-modifier "${GEVREK_DRAG_MODIFIER:-super,alt}" \
    --test-input \
    --config "$config" -- "$@" > "$work/wm.log" 2>&1 &
wm=$!
pids+=("$wm")

# Until the WM answers and has said which Wayland socket is its own.
socket=
for _ in $(seq 100); do
    if ! kill -0 "$wm" 2>/dev/null; then
        echo "gevrek-wl ended; the end of $work/wm.log:" >&2
        tail -n 20 "$work/wm.log" >&2
        exit 1
    fi
    socket=$(sed -n 's/.*gevrek is running on Wayland socket \(wayland-[0-9]*\).*/\1/p' "$work/wm.log" | tail -n 1)
    if [ -n "$socket" ] && "$bin/gevrek" health >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
if [ -z "$socket" ]; then
    echo "gevrek-wl did not come up; see $work/wm.log" >&2
    exit 1
fi
echo "gevrek runs on $socket ($size, $config), instance '$GEVREK_INSTANCE'. Log: $work/wm.log"
echo "Commands from another terminal: GEVREK_INSTANCE=$GEVREK_INSTANCE $bin/gevrek state"

if [ -z "$bare" ]; then
    # The shell: the panel, the launcher (Super+D), notifications. On a
    # message bus of its own when one can be had, so that it does not
    # take the notifications of the session this window is in.
    shell=("$bin/gevrek-shell")
    if command -v dbus-run-session >/dev/null 2>&1; then shell=(dbus-run-session -- "$bin/gevrek-shell"); fi
    WAYLAND_DISPLAY="$socket" RUST_LOG="${RUST_LOG:-warn}" \
        "${shell[@]}" > "$work/shell.log" 2>&1 &
    pids+=("$!")
    WAYLAND_DISPLAY="$socket" RUST_LOG="${RUST_LOG:-warn}" \
        "$bin/gevrek-studio" > "$work/studio.log" 2>&1 &
    pids+=("$!")
    # The list of shortcuts on top, once the studio's window is there.
    sleep 2
    "$bin/gevrek" shortcuts || true
    echo "Bar, editor and the list of shortcuts are started (Super+F1 closes and opens the list;"
    echo "Super+T: a terminal, Super+E: the editor)."
fi

wait "$wm"
