#!/usr/bin/env bash
# Install gevrek for this user (no administrator needed), or put this
# build over an older one:
#
#     packaging/macos/install.sh [path/to/Gevrek.app]
#
# The bundle goes to ~/Applications/Gevrek.app, and the command line
# `gevrek` (and `gevrek-studio`) is linked into ~/.local/bin. A gevrek that
# is running is stopped for the update (its parked windows come back) and
# started again. The configuration is not touched.
set -euo pipefail
cd "$(dirname "$0")/../.."

source_app="${1:-target/macos/Gevrek.app}"
[ -d "$source_app" ] || { echo "No $source_app: build it first (packaging/macos/build-app.sh)"; exit 1; }
apps="$HOME/Applications"
app="$apps/Gevrek.app"
bin="$HOME/.local/bin"

was_running=false
if [ -x "$app/Contents/MacOS/gevrek" ] && "$app/Contents/MacOS/gevrek" status 2>/dev/null | grep -qi "wm.*running"; then
    was_running=true
    echo "Stopping the running gevrek for the update..."
    "$app/Contents/MacOS/gevrek" stop wm || true
fi

mkdir -p "$apps" "$bin"
rm -rf "$app"
cp -R "$source_app" "$app"
for program in gevrek gevrek-studio; do
    ln -sf "$app/Contents/MacOS/$program" "$bin/$program"
done
echo "Installed $app"
echo "Linked gevrek and gevrek-studio into $bin"
case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "($bin is not on your PATH: add it, or run $bin/gevrek)";;
esac

if $was_running; then
    "$bin/gevrek" start wm
fi

cat <<'TEXT'

Then:

    gevrek start wm            the window manager (macOS asks, the first
                               time, to allow gevrek in Privacy & Security,
                               Accessibility: allow it, then start it again)
    gevrek enable-autostart    start it when you sign in
    gevrek stop wm             stop it; every window is put back
    gevrek-studio              the settings

The keys: gevrek's Win key is Option, its Alt is Control+Option+Command.
TEXT
