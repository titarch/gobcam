#!/usr/bin/env bash
# test-deb.sh — install/uninstall smoke test for the produced .deb.
#
# Spawns a clean `debian:trixie` container, copies in the .deb,
# installs it via dpkg with --force-depends (we don't actually need
# v4l2loopback-dkms / WebKit / GStreamer to verify the maintainer-
# scripts; pulling them in would add ~500 MB and minutes to the test
# for no extra coverage), then asserts:
#   - the modules-load.d / modprobe.d snippets land in /etc
#   - the postinst writes a valid /etc/sudoers.d/gobcam for $SUDO_USER
#   - the binaries land at /usr/bin
#   - `apt-get purge` cleans everything back up
#
# Containers can NOT exercise the kernel-module side: they share the
# host kernel and lack CAP_SYS_MODULE, so postinst's `modprobe` call
# fails (handled gracefully by the script's "DKMS may still be
# building" branch). For a real end-to-end test of the loopback
# bring-up, install the .deb in a Debian/Ubuntu VM.

set -euo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
DEB=$(find "$REPO_ROOT/target/release/bundle/deb" -maxdepth 1 -name 'Gobcam_*.deb' \
        -print -quit 2>/dev/null || true)

if [ -z "${DEB:-}" ] || [ ! -f "$DEB" ]; then
    echo "[test-deb] no .deb under target/release/bundle/deb — run \`just package\`" >&2
    echo "          or \`just docker-package\` first." >&2
    exit 1
fi

echo "[test-deb] testing $(basename "$DEB") in a fresh debian:trixie container"

# `-i` keeps stdin attached so the heredoc actually reaches bash.
# Without it, `docker run` closes stdin immediately and the inner
# script silently runs against an empty stdin (so all output below
# vanishes too).
docker run --rm -i \
    -v "$DEB:/tmp/gobcam.deb:ro" \
    -e DEBIAN_FRONTEND=noninteractive \
    debian:trixie bash -eu <<'CONTAINER'
fail() { echo "  ✗ $*" >&2; exit 1; }
pass() { echo "  ✓ $*"; }

echo "─── install prerequisites ─────────────────────────"
apt-get update -qq >/dev/null
# sudo gives us visudo so the postinst's `visudo -c` validation runs.
apt-get install -y -qq --no-install-recommends sudo >/dev/null

echo "─── dpkg -i (--force-depends) ─────────────────────"
# Pretend the install came in via `sudo apt install`. The postinst
# uses $SUDO_USER to decide whom to install the sudoers rule for.
useradd -m alice 2>/dev/null || true
SUDO_USER=alice dpkg -i --force-depends /tmp/gobcam.deb 2>&1 | tail -8

echo "─── verify install state ──────────────────────────"
[ -f /etc/modules-load.d/gobcam.conf ] && pass "/etc/modules-load.d/gobcam.conf present" \
                                       || fail "modules-load.d snippet missing"
[ -f /etc/modprobe.d/gobcam.conf    ] && pass "/etc/modprobe.d/gobcam.conf present" \
                                       || fail "modprobe.d snippet missing"
[ -f /etc/sudoers.d/gobcam          ] && pass "/etc/sudoers.d/gobcam present" \
                                       || fail "sudoers rule missing"
[ -x /usr/bin/gobcam-ui              ] && pass "/usr/bin/gobcam-ui executable" \
                                       || fail "gobcam-ui not installed"
[ -x /usr/bin/gobcam-pipeline        ] && pass "/usr/bin/gobcam-pipeline executable" \
                                       || fail "gobcam-pipeline (sidecar) not installed"

grep -q 'alice ALL=(root) NOPASSWD' /etc/sudoers.d/gobcam \
    && pass "sudoers rule mentions invoking user (alice)" \
    || fail "sudoers rule missing 'alice ALL=...' line"
visudo -c -q -f /etc/sudoers.d/gobcam \
    && pass "sudoers rule passes visudo" \
    || fail "sudoers rule failed visudo"

# .desktop entry must include the WEBKIT_DISABLE_DMABUF_RENDERER
# prefix or NVIDIA users get a blank window when launching from
# rofi/drun. Drift detection: if package.sh ever stops swapping the
# Tauri default in, this catches it.
[ -f /usr/share/applications/Gobcam.desktop ] \
    && pass "/usr/share/applications/Gobcam.desktop present" \
    || fail "Gobcam.desktop missing"
grep -q '^Exec=env WEBKIT_DISABLE_DMABUF_RENDERER=1 gobcam-ui' \
    /usr/share/applications/Gobcam.desktop \
    && pass ".desktop Exec= has NVIDIA workaround prefix" \
    || fail ".desktop Exec= missing WEBKIT_DISABLE_DMABUF_RENDERER prefix"

# Sanity-check the modprobe.d snippet has the canonical options the
# auto-reset path will issue — they have to match exactly or the
# sudoers NOPASSWD rule won't authorise them.
grep -q 'devices=1 video_nr=10 card_label=Gobcam exclusive_caps=1' \
    /etc/modprobe.d/gobcam.conf \
    && pass "modprobe.d options match what the UI auto-reset issues" \
    || fail "modprobe.d options drift from auto-reset payload"

echo "─── apt purge ─────────────────────────────────────"
apt-get purge -y -qq gobcam >/dev/null
[ ! -e /etc/sudoers.d/gobcam        ] && pass "sudoers rule removed by prerm" \
                                       || fail "sudoers rule survived purge"
[ ! -e /etc/modules-load.d/gobcam.conf ] && pass "modules-load.d snippet removed by purge" \
                                          || fail "modules-load.d snippet survived purge"
[ ! -e /usr/bin/gobcam-ui            ] && pass "gobcam-ui removed" \
                                       || fail "gobcam-ui survived purge"

echo "─── done ──────────────────────────────────────────"
CONTAINER

echo "[test-deb] all checks passed."
