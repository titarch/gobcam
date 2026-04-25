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
check: fmt-check lint test ui-check

# Heavier local "CI" gate: same checks plus the docker image build.
ci: check docker-build

# UI dev loop — opens the Tauri panel against the daemon's IPC socket.
# Daemon must be running separately (`just run -- --socket ...`).
ui-dev:
    pnpm -C crates/ui install
    pnpm -C crates/ui tauri dev

# Build a release UI binary (writes to crates/ui/src-tauri/target/release/).
ui-build:
    pnpm -C crates/ui install
    pnpm -C crates/ui tauri build

# Frontend gate: install with frozen lockfile, lint, type-check, test.
ui-check:
    pnpm -C crates/ui install --frozen-lockfile
    pnpm -C crates/ui run lint
    pnpm -C crates/ui run check-types
    pnpm -C crates/ui run test

docker-build:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.build -t gobcam:dev .

# Fetch curated Fluent emoji assets listed in assets/fluent/manifest.toml.
sync-emoji:
    scripts/sync-emoji.sh

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
view-loopback:
    gst-launch-1.0 v4l2src device=/dev/video10 ! videoconvert ! autovideosink

# Inspect what the real webcam can negotiate.
list-cam-formats D='/dev/video0':
    v4l2-ctl --list-formats-ext -d {{D}}
