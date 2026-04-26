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

# UI dev loop — opens the Tauri panel; UI auto-spawns the daemon.
# The daemon binary is looked up next to the UI binary at runtime, so
# both must live in the same target dir (debug for `tauri dev`).
#
# WEBKIT_DISABLE_DMABUF_RENDERER=1 works around WebKitGTK's hardware
# compositor failing to allocate GBM buffers under NVIDIA proprietary
# drivers ("Failed to create GBM buffer …"). Harmless on other GPUs.
ui-dev:
    pnpm -C crates/ui install
    cargo build -p gobcam-pipeline
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri dev

# One-launch app — alias for ui-dev. The UI process supervises the
# daemon; close the window and the daemon exits with it.
app: ui-dev

# Build a release UI binary (writes to crates/ui/src-tauri/target/release/).
ui-build:
    pnpm -C crates/ui install
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri build

# Build a .deb (Debian/Ubuntu) and an AppImage (everything else) from
# this source tree. Outputs land under
# target/release/bundle/{deb,appimage}/.
# The .deb's postinst loads v4l2loopback + writes a sudoers drop-in;
# the AppImage is portable but requires `sudo gobcam-setup` first run.
package:
    scripts/package.sh

# Build the Debian-Trixie packaging image (Tauri 2 + GStreamer build
# deps + linuxdeploy prerequisites). One-time + on Dockerfile changes.
docker-package-image:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.package -t gobcam-package:dev .

# Same outputs as `just package`, but built inside the Trixie image.
# Useful when the host distro doesn't match what linuxdeploy/Tauri
# expect (e.g. Arch's modern toolchain emitting `.relr.dyn` that the
# bundled strip can't parse). Mounts the source tree read-write because
# the Tauri build writes target/, src-tauri/binaries/, and crates/ui/
# (pnpm install + vite output) under it.
#
# Outputs land in the host's target/release/bundle/ — same as the
# native recipe — so README install paths apply unchanged.
docker-package: docker-package-image
    # tmpfs mounts shadow the bind-mounted source so pnpm's hoisted
    # modules — whose symlinks point into the container's `$HOME`
    # store — never touch the host. Without these shadows the host's
    # `pnpm install` refuses to run on the next `just check` because
    # the dangling symlinks look like a corrupted modules tree.
    # `--user` keeps every file written through the bind mount
    # (target/, .cargo-docker/, src-tauri/binaries/, …) host-owned.
    docker run --rm \
        --user "$(id -u):$(id -g)" \
        -v "$PWD:/workspace" \
        --tmpfs /workspace/crates/ui/node_modules:exec,uid=$(id -u),gid=$(id -g) \
        -e HOME=/tmp \
        -e CARGO_HOME=/workspace/.cargo-docker \
        -e CI=true \
        gobcam-package:dev

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

# One-time installer: drops /etc snippets so v4l2loopback auto-loads
# at every boot with our options. Prompts for sudo (or graphical via
# pkexec). After this, plain `just app` works on a fresh reboot
# without any sudo prompts.
install-loopback:
    bash scripts/gobcam-setup

# Inverse of `install-loopback`: removes the /etc snippets, the
# /etc/sudoers.d/gobcam rule, and rmmods the v4l2loopback module.
# Useful when testing the AppImage's first-run setup flow on a
# machine that already has Gobcam configured.
uninstall-loopback:
    bash scripts/gobcam-setup --uninstall

# Smoke-test the produced .deb in a clean debian:trixie container.
# Verifies postinst → /etc snippets land, sudoers rule is valid for
# $SUDO_USER, prerm cleans up. Doesn't exercise the kernel-module
# bring-up (containers share the host kernel) — for that, install in
# a real VM. Requires `just package` or `just docker-package` first.
docker-test-deb:
    bash scripts/test-deb.sh

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
