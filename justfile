set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# One-time developer setup: install `just` (if missing) and wire git hooks.
setup:
    scripts/setup-dev.sh

# Run the pipeline daemon. Forwards args, e.g. `just run -- -i /dev/video0 -o /dev/video10`.
run *ARGS:
    cargo run -p gobcam-pipeline -- {{ARGS}}

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# Lints the production code; examples (under `crates/pipeline/examples/`) are
# playground scaffolding and are deliberately excluded.
lint:
    cargo clippy --workspace --lib --bins --tests --all-features -- -D warnings

test:
    cargo test --workspace --all-features

# Cheap pre-commit gate: formatting + lints + tests (Rust + frontend).
# Runs every step, captures output, prints a one-line status per step
# (à la pre-commit). Set `CHECK_VERBOSE=1` to stream output live.
# Granular recipes (`fmt-check`, `lint`, `test`, `ui-check`) below
# stay verbose for direct invocation.
check:
    scripts/run-checks.sh

# Heavier local "CI" gate: same checks plus the docker image build.
ci: check docker-build

# UI dev loop — opens the Tauri panel against the daemon's IPC socket.
# Daemon must be running separately (`just run -- --socket ...`).
#
# WEBKIT_DISABLE_DMABUF_RENDERER=1 works around WebKitGTK's hardware
# compositor failing to allocate GBM buffers under NVIDIA proprietary
# drivers ("Failed to create GBM buffer …"). Harmless on other GPUs.
ui-dev:
    pnpm -C crates/ui install
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri dev

# Build a release UI binary (writes to crates/ui/src-tauri/target/release/).
ui-build:
    pnpm -C crates/ui install
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri build

# Frontend gate: install with frozen lockfile, lint, type-check, test.
ui-check:
    pnpm -C crates/ui install --frozen-lockfile
    pnpm -C crates/ui run lint
    pnpm -C crates/ui run check-types
    pnpm -C crates/ui run test

docker-build:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.build -t gobcam:dev .

# Regenerate the bundled Fluent emoji catalog from upstream Microsoft repos.
# Commits the result; daemon embeds it via include_str! at build time.
rebuild-catalog:
    python3 scripts/build-fluent-catalog.py

# Validate the GStreamer graph from the shell (per CLAUDE.md guidance).
gst-passthrough D='/dev/video0' OUT='/dev/video10':
    gst-launch-1.0 v4l2src device={{D}} ! videoconvert ! v4l2sink device={{OUT}}

# Load v4l2loopback (one-shot; not persistent across reboots).
modprobe-loopback:
    sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=Gobcam exclusive_caps=1

# Force-reset the v4l2loopback module (clears stuck OUTPUT-mode state after a
# failed run). Passwordless when scripts/sudoers-gobcam-dev is installed.
reset-loopback:
    -sudo rmmod v4l2loopback
    sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=Gobcam exclusive_caps=1

# Consume the loopback to confirm the daemon's output is visible.
# Tuned for low latency: a leaky size-1 queue clamps the consumer-side
# buffer to a single frame, and `sync=false` on the sink skips clock
# alignment. Without these, GStreamer's default queueing across
# `v4l2src → videoconvert → autovideosink` adds a perceptible delay.
view-loopback:
    gst-launch-1.0 v4l2src device=/dev/video10 io-mode=mmap ! \
        queue max-size-buffers=1 max-size-time=0 max-size-bytes=0 leaky=downstream ! \
        videoconvert ! \
        autovideosink sync=false

# Inspect what the real webcam can negotiate.
list-cam-formats D='/dev/video0':
    v4l2-ctl --list-formats-ext -d {{D}}

# Consumer-side latency probe. Watches the loopback and timestamps the
# first frame whose bottom-right patch differs from a baseline. JSONL
# output is comparable line-for-line with the daemon's --profile-log
# (both use SystemTime::now() since UNIX_EPOCH).
perf-capture *ARGS:
    cargo run -p gobcam-pipeline --release --example perf_capture -- {{ARGS}}
