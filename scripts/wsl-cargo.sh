#!/usr/bin/env bash
# Run cargo for the Linux side of the workspace inside WSL (Ubuntu-24.04),
# from Git Bash on Windows:   scripts/wsl-cargo.sh test -p gevrek-smithay
# The build output goes to the Linux file system: building on the mounted
# Windows drive is slow.
set -e
# The distribution is GEVREK_WSL_DISTRO (default Ubuntu-24.04); the
# repository is found from where this script is.
here="$(cd "$(dirname "$0")/.." && pwd -W 2>/dev/null || (cd "$(dirname "$0")/.." && pwd))"
inside="$(MSYS_NO_PATHCONV=1 wsl -d "${GEVREK_WSL_DISTRO:-Ubuntu-24.04}" -- wslpath -a "$here" | tr -d '\r\0')"
MSYS_NO_PATHCONV=1 wsl -d "${GEVREK_WSL_DISTRO:-Ubuntu-24.04}" -u root -- bash -lc ". /root/.cargo/env; cd '$inside'; export CARGO_TARGET_DIR=/root/gevrek-target; cargo $*" 2>&1 | tr -d '\0'
