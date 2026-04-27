#!/usr/bin/env bash
# Fetch + trim the Big Buck Bunny stand-in clip used by capture-demo.sh.
# ffmpeg's HTTP range-seek (`-ss` before `-i URL`) pulls only the bytes
# for the chosen window, so this is a few-MB transfer rather than the
# full ~170 MB asset.
#
# BBB © Blender Foundation, CC-BY 3.0 — https://peach.blender.org/
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${OUT:-$REPO_ROOT/assets/demo/conference-stand-in.mp4}"
SOURCE_URL="${SOURCE_URL:-https://download.blender.org/peach/bigbuckbunny_movies/big_buck_bunny_720p_h264.mov}"
START="${START:-00:01:35}"
DURATION="${DURATION:-5}"
WIDTH="${WIDTH:-1280}"
HEIGHT="${HEIGHT:-720}"
CRF="${CRF:-26}"

command -v ffmpeg >/dev/null || { echo "[fetch-demo-clip] ffmpeg not on PATH" >&2; exit 1; }

mkdir -p "$(dirname "$OUT")"
TMP="$(mktemp --suffix=.mp4)"
trap 'rm -f "$TMP"' EXIT

echo "[fetch-demo-clip] streaming ${DURATION}s from $START of $SOURCE_URL → $OUT"
ffmpeg -nostdin -hide_banner -loglevel error -y \
    -ss "$START" -t "$DURATION" -i "$SOURCE_URL" \
    -vf "scale=${WIDTH}:${HEIGHT}" \
    -c:v libx264 -preset slow -crf "$CRF" \
    -an -movflags +faststart \
    "$TMP"

mv "$TMP" "$OUT"
trap - EXIT
echo "[fetch-demo-clip] wrote $OUT ($(stat -c%s "$OUT") bytes)"
