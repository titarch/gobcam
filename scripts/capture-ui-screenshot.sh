#!/usr/bin/env bash
# Programmatically capture the README screenshot of the emoji picker panel.
#
# This runs the real Svelte UI through Vite with mocked Tauri commands, serves
# cached Fluent preview PNGs from the local Gobcam cache, and screenshots the
# panel with headless Chromium. No daemon, Tauri window, or loopback device is
# required.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_PNG="${OUT_PNG:-$REPO_ROOT/assets/screenshots/emoji-panel.png}"
WIDTH="${WIDTH:-420}"
HEIGHT="${HEIGHT:-760}"
DEVICE_SCALE="${DEVICE_SCALE:-1}"
CAPTURE_CACHE_ROOT="${GOBCAM_CAPTURE_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/gobcam}"
CHROMIUM="${CHROMIUM:-}"
PORT="${PORT:-}"

WORK="$(mktemp -d -t gobcam-ui-shot-XXXXXX)"
VITE_LOG="$WORK/vite.log"
VITE_PID=

cleanup() {
    set +e
    [[ -n "$VITE_PID" ]] && kill "$VITE_PID" 2>/dev/null
    [[ -n "$VITE_PID" ]] && wait "$VITE_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

for cmd in pnpm curl; do
    command -v "$cmd" >/dev/null || { echo "[capture-ui] $cmd not on PATH" >&2; exit 1; }
done

if [[ -z "$CHROMIUM" ]]; then
    for candidate in chromium chromium-browser google-chrome; do
        if command -v "$candidate" >/dev/null; then
            CHROMIUM="$(command -v "$candidate")"
            break
        fi
    done
fi
[[ -n "$CHROMIUM" ]] || { echo "[capture-ui] chromium not on PATH" >&2; exit 1; }

if [[ ! -d "$REPO_ROOT/crates/ui/node_modules" ]]; then
    echo "[capture-ui] installing frontend dependencies…"
    pnpm -C "$REPO_ROOT/crates/ui" install --frozen-lockfile
fi

port_open() {
    local port="$1"
    (echo >/dev/tcp/127.0.0.1/"$port") >/dev/null 2>&1
}

pick_port() {
    for candidate in 1420 1421 1422 1423 1424 1425; do
        if ! port_open "$candidate"; then
            echo "$candidate"
            return
        fi
    done
    echo "[capture-ui] no free Vite port found in 1420-1425" >&2
    exit 1
}

if [[ -z "$PORT" ]]; then
    PORT="$(pick_port)"
fi

echo "[capture-ui] launching mocked UI on http://127.0.0.1:$PORT…"
(
    cd "$REPO_ROOT/crates/ui"
    GOBCAM_CAPTURE_UI=1 GOBCAM_CAPTURE_CACHE_ROOT="$CAPTURE_CACHE_ROOT" \
        pnpm exec vite --host 127.0.0.1 --port "$PORT" --strictPort
) >"$VITE_LOG" 2>&1 &
VITE_PID=$!

for _ in {1..80}; do
    if curl -fsS "http://127.0.0.1:$PORT/" >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$VITE_PID" 2>/dev/null; then
        echo "[capture-ui] Vite exited early:" >&2
        sed 's/^/[vite] /' "$VITE_LOG" >&2
        exit 1
    fi
    sleep 0.25
done

if ! curl -fsS "http://127.0.0.1:$PORT/" >/dev/null 2>&1; then
    echo "[capture-ui] Vite did not become ready:" >&2
    sed 's/^/[vite] /' "$VITE_LOG" >&2
    exit 1
fi

mkdir -p "$(dirname "$OUT_PNG")"
echo "[capture-ui] writing $OUT_PNG (${WIDTH}x${HEIGHT})…"
"$CHROMIUM" \
    --headless \
    --no-sandbox \
    --disable-gpu \
    --hide-scrollbars \
    --force-device-scale-factor="$DEVICE_SCALE" \
    --window-size="${WIDTH},${HEIGHT}" \
    --screenshot="$OUT_PNG" \
    "http://127.0.0.1:$PORT/"

bytes=$(stat -c%s "$OUT_PNG")
echo "[capture-ui] wrote $OUT_PNG ($((bytes / 1024)) KiB)"
