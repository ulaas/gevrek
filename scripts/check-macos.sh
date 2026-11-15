#!/usr/bin/env bash
# Type-check for macOS from Linux: everything that is not one system's own
# (the model, the engine, the server, the command line, the studio, the
# shell's shared parts) for the aarch64-apple-darwin target. No linker is
# needed for a check. Nothing is run: gevrek has no macOS backend yet, and
# this keeps what a backend will stand on from growing Linux- or
# Windows-only without anyone noticing.
#
#     rustup target add aarch64-apple-darwin    # once
#     scripts/check-macos.sh
set -e
cd "$(dirname "$0")/.."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/mac}" \
    cargo check --target aarch64-apple-darwin --workspace \
        --exclude gevrek-smithay --exclude gevrek-win32 --exclude xtask --all-targets "$@"
