#!/usr/bin/env bash
# package.sh — produce a .deb and an AppImage from the source tree.
#
# Pipeline:
#   1. Build the daemon (`gobcam-pipeline`) in release mode.
#   2. Stage it next to the Tauri config under the target-triple name
#      Tauri's externalBin convention expects.
#   3. Run `tauri build --bundles deb appimage` to produce both
#      artifacts.
#   4. Post-process the .deb to inject the maintainer scripts
#      (`postinst`/`prerm`) — Tauri 2's schema doesn't expose those.
#
# Outputs land under crates/ui/src-tauri/target/release/bundle/.
set -euo pipefail

# Resolve repo root from this script's location (works regardless of cwd).
REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT"

TARGET_TRIPLE=${TARGET_TRIPLE:-$(rustc -vV | awk '/^host:/ {print $2}')}
DAEMON_BIN="$REPO_ROOT/target/release/gobcam-pipeline"
SIDECAR_DIR="$REPO_ROOT/crates/ui/src-tauri/binaries"
SIDECAR_BIN="$SIDECAR_DIR/gobcam-pipeline-${TARGET_TRIPLE}"
BUNDLE_DIR="$REPO_ROOT/target/release/bundle"

echo "[package] target triple: $TARGET_TRIPLE"
echo "[package] building gobcam-pipeline (release)…"
cargo build -p gobcam-pipeline --release

echo "[package] installing pnpm deps…"
pnpm -C crates/ui install --frozen-lockfile

echo "[package] staging daemon sidecar at $SIDECAR_BIN"
mkdir -p "$SIDECAR_DIR"
cp -f "$DAEMON_BIN" "$SIDECAR_BIN"
chmod 0755 "$SIDECAR_BIN"

echo "[package] running tauri build…"
# WEBKIT_DISABLE_DMABUF_RENDERER mirrors the dev recipe — `tauri build`
# probes the local WebKit at AppImage-bundling time and would otherwise
# hit the same NVIDIA GBM failure as `tauri dev` on this dev box.
#
# NO_STRIP=true: linuxdeploy bundles its own `strip` and that copy is
# too old to understand `.relr.dyn` sections emitted by the host's
# modern toolchain (Arch / glibc 2.40+). Skip stripping; AppImage size
# grows ~10–15 MB but the bundle actually completes. Drop this flag
# once linuxdeploy ships a newer binutils.
NO_STRIP=true \
WEBKIT_DISABLE_DMABUF_RENDERER=1 \
    pnpm -C crates/ui exec tauri build --bundles deb appimage

# ── Post-process the .deb to attach maintainer scripts ──────────────
# Tauri 2's tauri.conf.json schema doesn't expose deb maintainer-script
# settings, so we inject postinst/prerm by hand. .deb is just an `ar`
# archive of (debian-binary, control.tar.gz, data.tar.gz); we crack
# control.tar.gz open, drop in our scripts, and repack. This avoids
# requiring `dpkg-deb` on the build host (Arch doesn't ship it by
# default).
DEB_PATH=$(find "$BUNDLE_DIR/deb" -maxdepth 1 -name 'Gobcam_*.deb' -print -quit || true)
if [ -z "${DEB_PATH:-}" ]; then
    echo "[package] WARN: no .deb produced under $BUNDLE_DIR/deb" >&2
else
    echo "[package] injecting postinst/prerm into $(basename "$DEB_PATH")"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    (
        cd "$work"
        ar x "$DEB_PATH"
        mkdir control
        tar -C control -xzf control.tar.gz
        install -m 0755 "$REPO_ROOT/packaging/deb/postinst" control/postinst
        install -m 0755 "$REPO_ROOT/packaging/deb/prerm"    control/prerm
        tar -C control --owner=0 --group=0 -czf control.tar.gz .
        # `ar rcD` rebuilds with no timestamps so the .deb is
        # bit-reproducible across runs. Member order matters: dpkg
        # requires debian-binary first.
        rm -f "$DEB_PATH"
        ar rcD "$DEB_PATH" debian-binary control.tar.gz data.tar.gz
    )
    echo "[package] .deb ready: $DEB_PATH"
fi

APPIMAGE_PATH=$(find "$BUNDLE_DIR/appimage" -maxdepth 1 -name 'Gobcam_*.AppImage' -print -quit || true)
if [ -n "${APPIMAGE_PATH:-}" ]; then
    echo "[package] AppImage ready: $APPIMAGE_PATH"
fi

echo "[package] done."
