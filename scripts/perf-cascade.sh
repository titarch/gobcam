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
# Refuse to run with multiple daemons — sampling the wrong PID would
# silently produce numbers attributed to the wrong process.
mapfile -t PIDS < <(pgrep -x gobcam-pipeline)
if (( ${#PIDS[@]} == 0 )); then
    echo "[perf-cascade] gobcam-pipeline process not found (socket $SOCK exists but no PID)" >&2
    exit 1
fi
if (( ${#PIDS[@]} > 1 )); then
    echo "[perf-cascade] multiple gobcam-pipeline processes (${PIDS[*]}) — kill all but one and retry" >&2
    exit 1
fi
PID="${PIDS[0]}"

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
# Stderr counts of swallowed-but-recorded conditions, written by the
# samplers and read in the summary section. Files (not vars) so the
# child subshells can update them without IPC plumbing.
TRIG_FAILS_FILE="$RUN_DIR/.trig-fails"
GPU_FAILS_FILE="$RUN_DIR/.gpu-fails"
DAEMON_DEAD_FILE="$RUN_DIR/.daemon-dead"
echo 0 > "$TRIG_FAILS_FILE"
echo 0 > "$GPU_FAILS_FILE"

cleanup() {
    set +e
    for pid in "$CPU_PID" "$GPU_PID" "$TRIG_PID"; do
        [[ -n "$pid" ]] && kill "$pid" 2>/dev/null
    done
    sleep 0.5
    for pid in "$CPU_PID" "$GPU_PID" "$TRIG_PID"; do
        [[ -n "$pid" ]] && kill -9 "$pid" 2>/dev/null
    done
    wait 2>/dev/null
}
trap cleanup EXIT

# CPU sampler: %CPU + RSS (MiB) of the daemon, 1 Hz. Writes a sentinel
# to DAEMON_DEAD_FILE so the trigger spammer can exit early too.
(
    start=$(date +%s)
    while :; do
        now=$(date +%s)
        t=$((now - start))
        line=$(ps -p "$PID" -o pcpu=,rss= 2>/dev/null || true)
        if [[ -z "$line" ]]; then
            echo "[perf-cascade] daemon pid=$PID gone after ${t}s — sampling stopped" >&2
            echo "$t" > "$DAEMON_DEAD_FILE"
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

# GPU sampler: nvidia-smi at 1 Hz. Counts failures; surfaces the first
# one to stderr so a transient driver hiccup doesn't silently degrade
# percentile accuracy.
(
    start=$(date +%s)
    first_fail_logged=0
    while :; do
        now=$(date +%s)
        t=$((now - start))
        if row=$(nvidia-smi \
                --query-gpu=utilization.gpu,utilization.memory,memory.used \
                --format=csv,noheader,nounits 2>/dev/null \
                | head -n1 \
                | tr -d ' '); then
            if [[ -n "$row" ]]; then
                echo "$t,$row" >> "$GPU_CSV"
            else
                # Empty output, treat as failure.
                count=$(($(cat "$GPU_FAILS_FILE") + 1))
                echo "$count" > "$GPU_FAILS_FILE"
                if (( first_fail_logged == 0 )); then
                    echo "[perf-cascade] nvidia-smi returned empty row at t=${t}s" >&2
                    first_fail_logged=1
                fi
            fi
        else
            count=$(($(cat "$GPU_FAILS_FILE") + 1))
            echo "$count" > "$GPU_FAILS_FILE"
            if (( first_fail_logged == 0 )); then
                echo "[perf-cascade] nvidia-smi failed at t=${t}s — GPU samples will be sparse" >&2
                first_fail_logged=1
            fi
        fi
        sleep 1
    done
) &
GPU_PID=$!

# Trigger spammer. Counts IPC failures; bails if the failure rate is
# such that the run is producing meaningless data (daemon down, socket
# wedged, ncat broken). Also exits early if the CPU sampler flagged
# the daemon as dead.
(
    sleep_s=$(awk -v ms="$INTERVAL_MS" 'BEGIN{printf "%.3f", ms/1000}')
    end=$(($(date +%s) + DURATION))
    sent=0
    fails=0
    first_fail_logged=0
    while (( $(date +%s) < end )); do
        if [[ -f "$DAEMON_DEAD_FILE" ]]; then
            echo "[perf-cascade] daemon died — trigger spammer exiting early" >&2
            break
        fi
        if printf '{"type":"trigger","emoji_id":"%s"}\n' "$EMOJI" \
                | ncat -U "$SOCK" >/dev/null 2>&1; then
            sent=$((sent + 1))
        else
            fails=$((fails + 1))
            if (( first_fail_logged == 0 )); then
                echo "[perf-cascade] first IPC trigger failure at t=$(($(date +%s) - (end - DURATION)))s" >&2
                first_fail_logged=1
            fi
            # If we've sent at least 5 attempts and >50 % failed, the
            # run is producing no load. Bail loudly.
            if (( sent + fails >= 5 && fails * 2 > sent + fails )); then
                echo "[perf-cascade] ${fails}/${sent} IPC triggers failed — aborting run" >&2
                echo "$fails" > "$TRIG_FAILS_FILE"
                exit 1
            fi
        fi
        sleep "$sleep_s"
    done
    echo "$fails" > "$TRIG_FAILS_FILE"
) &
TRIG_PID=$!

echo "[perf-cascade] sampling pid=$PID for ${DURATION}s @ ${EMOJI} every ${INTERVAL_MS}ms (label='$LABEL')"
echo "[perf-cascade] run dir: $RUN_DIR"
wait "$TRIG_PID" 2>/dev/null || true
# Give samplers one more tick to flush.
sleep 1
kill "$CPU_PID" "$GPU_PID" 2>/dev/null || true
wait "$CPU_PID" "$GPU_PID" 2>/dev/null || true

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
                if (n == 0) { printf "  %-18s  *** NO SAMPLES *** — sampler crashed or daemon died early\n", name; exit }
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

cpu_n=$(($(wc -l <"$CPU_CSV") - 1))
gpu_n=$(($(wc -l <"$GPU_CSV") - 1))
trig_fails=$(cat "$TRIG_FAILS_FILE" 2>/dev/null || echo 0)
gpu_fails=$(cat "$GPU_FAILS_FILE" 2>/dev/null || echo 0)
daemon_dead_at=""
if [[ -f "$DAEMON_DEAD_FILE" ]]; then
    daemon_dead_at=$(cat "$DAEMON_DEAD_FILE")
fi

# Warn on sample-count drift between CPU and GPU CSVs (>10 % gap, or
# either ≪ DURATION). A wide gap means percentile comparisons across
# the two are misleading.
warnings=()
if [[ -n "$daemon_dead_at" ]]; then
    warnings+=("daemon died at t=${daemon_dead_at}s — only the prefix is valid")
fi
if (( cpu_n < DURATION * 9 / 10 )); then
    warnings+=("CPU sample count $cpu_n is <90% of DURATION $DURATION")
fi
if (( gpu_n < DURATION * 9 / 10 )); then
    warnings+=("GPU sample count $gpu_n is <90% of DURATION $DURATION")
fi
if (( cpu_n > 0 && gpu_n > 0 )); then
    diff=$((cpu_n > gpu_n ? cpu_n - gpu_n : gpu_n - cpu_n))
    larger=$((cpu_n > gpu_n ? cpu_n : gpu_n))
    if (( diff * 10 > larger )); then
        warnings+=("CPU/GPU sample drift: cpu_n=$cpu_n gpu_n=$gpu_n")
    fi
fi
if (( trig_fails > 0 )); then
    warnings+=("$trig_fails IPC trigger failures during run")
fi
if (( gpu_fails > 0 )); then
    warnings+=("$gpu_fails nvidia-smi failures during run")
fi
rm -f "$TRIG_FAILS_FILE" "$GPU_FAILS_FILE" "$DAEMON_DEAD_FILE"

{
    echo "label   = $LABEL"
    echo "ts      = $ts"
    echo "pid     = $PID"
    echo "duration_s = $DURATION  interval_ms = $INTERVAL_MS  emoji = $EMOJI"
    if (( ${#warnings[@]} > 0 )); then
        echo
        echo "*** WARNINGS — interpret numbers below with care ***"
        for w in "${warnings[@]}"; do echo "  - $w"; done
    fi
    echo
    echo "[CPU]"
    summarise "$CPU_CSV"
    echo
    echo "[GPU]"
    summarise "$GPU_CSV"
} | tee "$SUMMARY"
