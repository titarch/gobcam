#!/usr/bin/env bash
# Programmatically capture the README hero GIF.
#
# Pipeline:
#   1. Loop the demo clip into the input loopback (default /dev/video11)
#      with ffmpeg.
#   2. Run gobcam-pipeline reading the input loopback, writing the
#      output loopback (default /dev/video10).
#   3. Trigger a scripted emoji sequence over the IPC socket.
#   4. Capture the output loopback to PNG frames with ffmpeg.
#   5. Encode the frames to a GIF with gifski.
#
# Cleanup runs on EXIT — kills every spawned process. Loopback module
# state is left as-is; this script never touches kernel modules.
#
# Setup the two loopbacks beforehand (one-time):
#
#   sudo modprobe -r v4l2loopback
#   sudo modprobe v4l2loopback devices=2 video_nr=10,11 \
#       card_label="Gobcam,GobcamInput" exclusive_caps=1,1
#
# Restore single-device afterwards:
#
#   sudo modprobe -r v4l2loopback
#   sudo modprobe v4l2loopback devices=1 video_nr=10 \
#       card_label=Gobcam exclusive_caps=1
#
# Requires: ffmpeg, gifski, ncat, awk.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_CLIP="${DEMO_CLIP:-$REPO_ROOT/assets/demo/conference-stand-in.mp4}"
OUT_GIF="${OUT_GIF:-$REPO_ROOT/assets/screenshots/cascade.gif}"
INPUT_DEV="${INPUT_DEV:-/dev/video11}"
OUTPUT_DEV="${OUTPUT_DEV:-/dev/video10}"
EMOJI_SEQUENCE="${EMOJI_SEQUENCE:-fire heart star party-popper sparkles thumbs-up clapping-hands}"
DURATION="${DURATION:-5}"
FPS="${FPS:-15}"
WIDTH="${WIDTH:-1280}"
HEIGHT="${HEIGHT:-720}"
TRIGGER_INTERVAL_MS="${TRIGGER_INTERVAL_MS:-350}"
PREROLL_S="${PREROLL_S:-2}"

WORK="$(mktemp -d -t gobcam-demo-XXXXXX)"
SOCK="$WORK/gobcam.sock"

INPUT_PID=
DAEMON_PID=
CAPTURE_PID=

cleanup() {
    set +e
    for pid in "$CAPTURE_PID" "$DAEMON_PID" "$INPUT_PID"; do
        [[ -n "$pid" ]] && kill "$pid" 2>/dev/null
    done
    sleep 0.5
    for pid in "$CAPTURE_PID" "$DAEMON_PID" "$INPUT_PID"; do
        [[ -n "$pid" ]] && kill -9 "$pid" 2>/dev/null
    done
    wait 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

if [[ ! -f "$DEMO_CLIP" ]]; then
    echo "[capture-demo] missing input clip: $DEMO_CLIP — fetching…" >&2
    OUT="$DEMO_CLIP" "$REPO_ROOT/scripts/fetch-demo-clip.sh"
fi
for cmd in ffmpeg gifski ncat cargo awk; do
    command -v "$cmd" >/dev/null || { echo "[capture-demo] $cmd not on PATH" >&2; exit 1; }
done
for dev in "$INPUT_DEV" "$OUTPUT_DEV"; do
    if [[ ! -c "$dev" ]]; then
        echo "[capture-demo] missing v4l2 device: $dev" >&2
        echo "              load both loopbacks first:" >&2
        echo "                sudo modprobe -r v4l2loopback" >&2
        echo "                sudo modprobe v4l2loopback devices=2 video_nr=10,11 \\" >&2
        echo "                    card_label=\"Gobcam,GobcamInput\" exclusive_caps=1,1" >&2
        exit 1
    fi
done

# Clear stale openers from prior failed runs. v4l2loopback at
# exclusive_caps=1 holds direction state across reopens, so a leftover
# ffmpeg from a crashed run will deterministically break the next one.
for dev in "$INPUT_DEV" "$OUTPUT_DEV"; do
    pids=$(fuser "$dev" 2>/dev/null | tr -s ' ' || true)
    if [[ -n "$pids" ]]; then
        echo "[capture-demo] killing stale openers on $dev:$pids" >&2
        # shellcheck disable=SC2086
        kill -9 $pids 2>/dev/null || true
    fi
done
sleep 0.3

echo "[capture-demo] streaming $DEMO_CLIP → $INPUT_DEV (looping)…"
ffmpeg -nostdin -hide_banner -loglevel error -re -stream_loop -1 \
    -i "$DEMO_CLIP" \
    -vf "scale=${WIDTH}:${HEIGHT},fps=${FPS_OUT:-30},format=yuv420p" \
    -f v4l2 "$INPUT_DEV" &
INPUT_PID=$!
sleep 3

echo "[capture-demo] building + launching gobcam-pipeline…"
cargo build --release -p gobcam-pipeline >/dev/null

# Reading from a v4l2loopback requires `io-mode=rw`; the default
# `auto` (MMAP/DMABUF) starves on loopback's small buffer pool.
"$REPO_ROOT/target/release/gobcam-pipeline" \
    --input "$INPUT_DEV" \
    --input-io-mode rw \
    --output "$OUTPUT_DEV" \
    --width "$WIDTH" \
    --height "$HEIGHT" \
    --socket "$SOCK" &
DAEMON_PID=$!

for _ in {1..30}; do
    [[ -S "$SOCK" ]] && break
    sleep 0.2
done
[[ -S "$SOCK" ]] || { echo "[capture-demo] socket didn't appear" >&2; exit 1; }

echo "[capture-demo] pre-rolling ${PREROLL_S}s before triggers…"
sleep "$PREROLL_S"

echo "[capture-demo] capturing ${DURATION}s @ ${FPS} fps from $OUTPUT_DEV…"
FRAMES_DIR="$WORK/frames"
mkdir -p "$FRAMES_DIR"
# `timeout --kill-after` enforces a wallclock cap regardless of what
# ffmpeg's `-t` decides — protects against /dev/video10 stalling
# mid-capture if the daemon dies, which would otherwise hang the script.
timeout --kill-after=2s "$((DURATION + 2))" \
    ffmpeg -nostdin -hide_banner -loglevel error -y \
        -f v4l2 -framerate 30 -video_size "${WIDTH}x${HEIGHT}" -i "$OUTPUT_DEV" \
        -vf "fps=${FPS},scale=720:-1:flags=lanczos" \
        -t "$DURATION" \
        "$FRAMES_DIR/frame_%04d.png" &
CAPTURE_PID=$!

sleep 0.5
read -ra EMOJIS <<<"$EMOJI_SEQUENCE"
end_time=$(($(date +%s) + DURATION))
i=0
while (( $(date +%s) < end_time )); do
    emoji="${EMOJIS[$((i % ${#EMOJIS[@]}))]}"
    printf '{"type":"trigger","emoji_id":"%s"}\n' "$emoji" \
        | ncat -U "$SOCK" >/dev/null 2>&1 || true
    i=$((i + 1))
    sleep "$(awk -v ms="$TRIGGER_INTERVAL_MS" 'BEGIN{print ms/1000}')"
done

wait "$CAPTURE_PID" || true

frame_count=$(find "$FRAMES_DIR" -maxdepth 1 -name 'frame_*.png' | wc -l)
if (( frame_count < 5 )); then
    echo "[capture-demo] only $frame_count frames captured — bailing." >&2
    exit 1
fi

echo "[capture-demo] encoding $frame_count frames → $OUT_GIF…"
mkdir -p "$(dirname "$OUT_GIF")"
gifski --fps "$FPS" --quality 90 --width 720 \
    -o "$OUT_GIF" "$FRAMES_DIR"/frame_*.png

bytes=$(stat -c%s "$OUT_GIF")
echo "[capture-demo] wrote $OUT_GIF ($((bytes / 1024)) KiB)"
