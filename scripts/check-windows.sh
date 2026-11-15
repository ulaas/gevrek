#!/usr/bin/env bash
# Type-check the Windows side from Linux: the Windows backend, gevrek-wm, the
# CLI and everything they use, for the x86_64-pc-windows-msvc target. No
# linker is needed for a check. It does not run anything: behaviour on
# Windows is still only seen on Windows (CI).
#
#     rustup target add x86_64-pc-windows-msvc    # once
#     scripts/check-windows.sh
set -e
cd "$(dirname "$0")/.."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/win}" \
    cargo check --target x86_64-pc-windows-msvc --workspace --exclude gevrek-smithay --exclude xtask --all-targets "$@"
