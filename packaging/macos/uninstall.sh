#!/usr/bin/env bash
# Remove gevrek installed by install.sh: it is stopped (its windows come
# back), its start at sign-in is removed, and so are the bundle and the
# links. The configuration stays (`gevrek config-path` said where).
set -euo pipefail
app="$HOME/Applications/Gevrek.app"
bin="$HOME/.local/bin"
if [ -x "$app/Contents/MacOS/gevrek" ]; then
    "$app/Contents/MacOS/gevrek" stop wm 2>/dev/null || true
    "$app/Contents/MacOS/gevrek" disable-autostart 2>/dev/null || true
fi
for program in gevrek gevrek-studio; do
    if [ -L "$bin/$program" ] && [ "$(readlink "$bin/$program")" = "$app/Contents/MacOS/$program" ]; then
        rm "$bin/$program"
    fi
done
rm -rf "$app"
echo "gevrek is removed. Its entry in Privacy & Security, Accessibility can be taken out there."
