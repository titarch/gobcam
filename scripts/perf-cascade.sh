#!/usr/bin/env bash
# Drive a sustained, saturated cascade against a running gobcam
# daemon and capture CPU + GPU samples for later comparison
# (Phase 0 of the GPU-compositing exploration).
#
# Attaches to an existing daemon — run `just app` first.
#
# Usage:
#   scripts/perf-cascade.sh [LABEL]
#
# Env knobs:
#   DURATION         seconds to sample (default 60)
#   INTERVAL_MS      gap between trigger sends (default 200 → 5/s)
#   EMOJI            id to spam (default "fire", maximises
#                    cached_memory reuse — single-emoji is the
#                    blend-cost worst case)
#   SOCK             daemon IPC socket
#                    (default $XDG_RUNTIME_DIR/gobcam.sock)
#   OUT_ROOT         where run dirs land (default ./perf-runs)
#
# Requires: ncat, nvidia-smi, awk, ps, date.
set -euo pipefail

LABEL="${1:-compositor=cpu}"
DURATION="${DURATION:-60}"
INTERVAL_MS="${INTERVAL_MS:-200}"
EMOJI="${EMOJI:-fire}"
SOCK="${SOCK:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/gobcam.sock}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_ROOT="${OUT_ROOT:-$REPO_ROOT/perf-runs}"

for cmd in ncat nvidia-smi awk ps date; do
    command -v "$cmd" >/dev/null || {
        echo "[perf-cascade] $cmd not on PATH" >&2
        exit 1
    }
done

if [[ ! -S "$SOCK" ]]; then
    echo "[perf-cascade] daemon socket $SOCK not found — start the app first" >&2
    exit 1
fi
PID="$(pgrep -x gobcam-pipeline | head -n1 || true)"
if [[ -z "$PID" ]]; then
    echo "[perf-cascade] gobcam-pipeline process not found (socket $SOCK exists but no PID)" >&2
    exit 1
fi

ts="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
RUN_DIR="$OUT_ROOT/$ts"
mkdir -p "$RUN_DIR"
CPU_CSV="$RUN_DIR/cpu.csv"
GPU_CSV="$RUN_DIR/gpu.csv"
SUMMARY="$RUN_DIR/summary.txt"

echo "t_s,pcpu,rss_mb"   > "$CPU_CSV"
echo "t_s,gpu_pct,gpu_mem_pct,gpu_mem_used_mb" > "$GPU_CSV"

CPU_PID=
GPU_PID=
TRIG_PID=
cleanup() {
    set +e
    [[ -n "$CPU_PID"  ]] && kill "$CPU_PID"  2>/dev/null
    [[ -n "$GPU_PID"  ]] && kill "$GPU_PID"  2>/dev/null
    [[ -n "$TRIG_PID" ]] && kill "$TRIG_PID" 2>/dev/null
    wait 2>/dev/null
}
trap cleanup EXIT

# CPU sampler: %CPU + RSS (MiB) of the daemon, 1 Hz.
(
    start=$(date +%s)
    while :; do
        now=$(date +%s)
        t=$((now - start))
        line=$(ps -p "$PID" -o pcpu=,rss= 2>/dev/null || true)
        if [[ -z "$line" ]]; then
            # daemon died mid-run; bail the sampler, main loop will notice via the trigger sender
            break
        fi
        pcpu=$(awk '{print $1}' <<<"$line")
        rss_kb=$(awk '{print $2}' <<<"$line")
        rss_mb=$(awk -v k="$rss_kb" 'BEGIN{printf "%.1f", k/1024}')
        echo "$t,$pcpu,$rss_mb" >> "$CPU_CSV"
        sleep 1
    done
) &
CPU_PID=$!

# GPU sampler: nvidia-smi at 1 Hz.
(
    start=$(date +%s)
    while :; do
        now=$(date +%s)
        t=$((now - start))
        row=$(nvidia-smi \
            --query-gpu=utilization.gpu,utilization.memory,memory.used \
            --format=csv,noheader,nounits 2>/dev/null \
            | head -n1 \
            | tr -d ' ')
        if [[ -n "$row" ]]; then
            echo "$t,$row" >> "$GPU_CSV"
        fi
        sleep 1
    done
) &
GPU_PID=$!

# Trigger spammer.
(
    sleep_s=$(awk -v ms="$INTERVAL_MS" 'BEGIN{printf "%.3f", ms/1000}')
    end=$(($(date +%s) + DURATION))
    while (( $(date +%s) < end )); do
        printf '{"type":"trigger","emoji_id":"%s"}\n' "$EMOJI" \
            | ncat -U "$SOCK" >/dev/null 2>&1 || true
        sleep "$sleep_s"
    done
) &
TRIG_PID=$!

echo "[perf-cascade] sampling pid=$PID for ${DURATION}s @ ${EMOJI} every ${INTERVAL_MS}ms (label='$LABEL')"
echo "[perf-cascade] run dir: $RUN_DIR"
wait "$TRIG_PID" 2>/dev/null || true
# Give samplers one more tick to flush.
sleep 1
kill "$CPU_PID" "$GPU_PID" 2>/dev/null || true
wait 2>/dev/null || true

# Summarise: avg / p50 / p95 / max for each numeric column.
summarise() {
    local csv="$1"
    local header
    header=$(head -n1 "$csv")
    IFS=',' read -ra cols <<<"$header"
    # Skip column 0 (t_s).
    for ((i=1; i<${#cols[@]}; i++)); do
        local name="${cols[$i]}"
        # awk is 1-indexed; CSV column index = awk index.
        local awk_idx=$((i + 1))
        awk -F',' -v col="$awk_idx" -v name="$name" '
            NR > 1 { v[NR-1] = $col; sum += $col; if ($col > max) max = $col }
            END {
                n = NR - 1
                if (n == 0) { printf "  %-18s  (no samples)\n", name; exit }
                avg = sum / n
                # sort copy of v[]
                for (i=1; i<=n; i++) s[i] = v[i]
                # naive insertion sort, n is small (<= duration/sec)
                for (i=2; i<=n; i++) {
                    key = s[i]; j = i-1
                    while (j>=1 && s[j]+0 > key+0) { s[j+1] = s[j]; j-- }
                    s[j+1] = key
                }
                p50 = s[int(n * 0.50 + 0.5)]
                p95 = s[int(n * 0.95 + 0.5)]
                printf "  %-18s  avg=%8.2f  p50=%8.2f  p95=%8.2f  max=%8.2f  (n=%d)\n",
                       name, avg, p50, p95, max, n
            }
        ' "$csv"
    done
}

{
    echo "label   = $LABEL"
    echo "ts      = $ts"
    echo "pid     = $PID"
    echo "duration_s = $DURATION  interval_ms = $INTERVAL_MS  emoji = $EMOJI"
    echo
    echo "[CPU]"
    summarise "$CPU_CSV"
    echo
    echo "[GPU]"
    summarise "$GPU_CSV"
} | tee "$SUMMARY"
