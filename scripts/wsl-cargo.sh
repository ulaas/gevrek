#!/usr/bin/env bash
# Run cargo for the Linux side of the workspace inside WSL, from Git Bash
# on Windows:
#
#     scripts/wsl-cargo.sh test --workspace --exclude gevrek-win32
#
# The distribution is GEVREK_WSL_DISTRO, or WSL's default one; cargo is
# the one of that distribution's own user (`~/.cargo`). The build output
# goes to the Linux file system (`~/gevrek-target`): building on the
# mounted Windows drive is slow. `gevrek-win32` does not build on Linux:
# leave it out of anything that names the whole workspace.
#
# What is drawn is drawn on the processor: WSL's own graphics end the
# studio's picture tests with a fault (docs/ROADMAP.md says more).
set -eo pipefail
distro=()
if [ -n "${GEVREK_WSL_DISTRO:-}" ]; then
    distro=(-d "$GEVREK_WSL_DISTRO")
fi
# The repository is found from where this script is.
here="$(cd "$(dirname "$0")/.." && (pwd -W 2>/dev/null || pwd))"
inside="$(MSYS_NO_PATHCONV=1 wsl "${distro[@]}" -- wslpath -a "$here" | tr -d '\r\0')"
# Each argument as it was given, also one with a space in it.
arguments=""
for argument in "$@"; do
    arguments+=" $(printf '%q' "$argument")"
done
# One line for the shell inside: what WSL is handed as several
# arguments it joins again, and their quotes are lost on the way.
# (And no variable of its own in it: WSL's first shell reads the line
# before bash does, and would put nothing in a variable's place.)
inner='. ~/.cargo/env; cd '"$(printf '%q' "$inside")"'; export CARGO_TARGET_DIR="$HOME/gevrek-target" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan; ls /usr/share/vulkan/icd.d/lvp_icd.*.json >/dev/null 2>&1 && export VK_DRIVER_FILES=$(ls /usr/share/vulkan/icd.d/lvp_icd.*.json | head -1); cargo'"$arguments"
# (WSL writes some of its own lines as UTF-16: the noughts are taken out.
# With `pipefail` the answer is cargo's, not that of the filter.)
MSYS_NO_PATHCONV=1 wsl "${distro[@]}" -- bash -lc "$inner" 2>&1 | tr -d '\0'
