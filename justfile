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

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

# Cheap pre-commit gate: formatting + lints + tests.
check: fmt-check lint test

# Heavier local "CI" gate: same checks plus the docker image build.
ci: check docker-build

docker-build:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.build -t gobcam:dev .

# Validate the GStreamer graph from the shell (per CLAUDE.md guidance).
gst-passthrough D='/dev/video0' OUT='/dev/video10':
    gst-launch-1.0 v4l2src device={{D}} ! videoconvert ! v4l2sink device={{OUT}}

# Load v4l2loopback (one-shot; not persistent across reboots).
modprobe-loopback:
    sudo modprobe v4l2loopback devices=1 video_nr=10 card_label="Gobcam" exclusive_caps=1

# Consume the loopback to confirm the daemon's output is visible.
view-loopback:
    gst-launch-1.0 v4l2src device=/dev/video10 ! videoconvert ! autovideosink

# Inspect what the real webcam can negotiate.
list-cam-formats D='/dev/video0':
    v4l2-ctl --list-formats-ext -d {{D}}
