#!/usr/bin/env bash
# stage-sidecar.sh — build the daemon in debug mode and stage it at the
# Tauri externalBin path so `tauri dev` finds a fresh sidecar.
#
# Tauri 2 requires `binaries/gobcam-pipeline-<triple>` to exist when
# externalBin is declared, and copies it over the workspace's
# target/debug/gobcam-pipeline at dev-launch time. Without this script,
# the dev runtime would either fail (no sidecar) or pick up a stale
# release build left over from a prior `just package` (clobbering the
# fresh debug binary). `just package` itself removes its staged sidecar
# on exit, so the only tool maintaining the dev sidecar is this one.
set -euo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}')

cargo build --manifest-path "$REPO_ROOT/Cargo.toml" -p gobcam-pipeline

mkdir -p "$REPO_ROOT/crates/ui/src-tauri/binaries"
cp -f \
    "$REPO_ROOT/target/debug/gobcam-pipeline" \
    "$REPO_ROOT/crates/ui/src-tauri/binaries/gobcam-pipeline-$TRIPLE"
