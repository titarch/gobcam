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

# CI variant of `docker-package` for runners that spawn the workflow
# inside a Docker container (e.g. act_runner's DinD setup). The
# workspace inside the job container is a Docker volume managed by
# the daemon, not a bind-mounted host path — so `-v $PWD:/...`
# from inside the workflow asks the daemon for a path it can't
# resolve. `--volumes-from "$HOSTNAME"` sidesteps that entirely:
# the new container inherits the workspace volume from the job
# container, source appears at $PWD just like inside the job
# container, no path translation needed.
#
# Generic pattern — applies to any project doing nested Docker on
# this runner. The local `docker-package` keeps the bind-mount
# path because nothing nests it.
docker-package-ci: docker-package-image
    #!/usr/bin/env bash
    set -euo pipefail
    container_id=$(cat /etc/hostname)
    docker run --rm \
        --volumes-from "$container_id" \
        -w "$PWD" \
        -e HOME=/tmp \
        -e CARGO_HOME=/tmp/cargo \
        -e CI=true \
        gobcam-package:dev

# Locally simulate every testable step of .github/workflows/release.yml
# without pushing a tag or hitting any release API. Mirrors the
# workflow's shell step-by-step so anything that would break on the
# runner (version mismatch, build failure, missing artifact, broken
# checksum) breaks here too.
#
# Why not `act_runner exec`? Two reasons specific to this workflow:
#  - `-self-hosted` runs in act's own scratch dir, so steps can't
#    see the source unless `actions/checkout@v4` runs (which needs
#    a working clone URL + auth into Gitea).
#  - Container mode bind-mounts the source at /github/workspace,
#    but `just docker-package` then does `docker run -v $PWD:/...`
#    against the *host* daemon, where /github/workspace doesn't
#    exist — the inevitable Docker-in-Docker bind-mount mismatch.
# The only thing this recipe can't catch is the actual
# `softprops/action-gh-release@v2` upload — that needs a real
# Gitea/GitHub. Everything else is identical.
ci-local:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo_version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

    # Step: Verify tag matches Cargo.toml — copy of the workflow's
    # shell verbatim, with GITHUB_REF_NAME spoofed to the matching
    # value so the happy path runs.
    GITHUB_REF_NAME="v$cargo_version"
    tag="${GITHUB_REF_NAME#v}"
    if [ "$tag" != "$cargo_version" ]; then
        echo "::error::tag $tag does not match Cargo.toml $cargo_version"
        exit 1
    fi
    echo "✓ verify tag matches Cargo.toml ($GITHUB_REF_NAME)"

    # Step: Build .deb + AppImage
    just docker-package

    # Step: Compute SHA256SUMS
    cd target/release/bundle
    (cd deb && sha256sum *.deb) > SHA256SUMS
    (cd appimage && sha256sum *.AppImage) >> SHA256SUMS
    echo "✓ SHA256SUMS:"
    sed 's/^/    /' SHA256SUMS

    echo
    echo "All testable steps passed. The remaining release-upload step"
    echo "needs a real server — push a real tag with \`just release-tag\`"
    echo "to exercise it on the configured runner."

# Cut a release end-to-end: bump Cargo.toml's workspace version,
# refresh Cargo.lock, commit, tag, push commit + tag to origin.
# Triggers .github/workflows/release.yml on the runner.
#
#   just release patch          # 0.1.0 → 0.1.1
#   just release minor          # 0.1.5 → 0.2.0
#   just release major          # 0.2.3 → 1.0.0
#   just release 1.2.3          # explicit
#   just release patch --no-push   # commit + tag locally only
release LEVEL='patch' *FLAGS:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! git diff --quiet HEAD; then
        echo "working tree is dirty — commit or stash first" >&2
        exit 1
    fi

    current=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
    case "{{LEVEL}}" in
        patch|minor|major)
            IFS=. read -r mj mn pt <<<"$current"
            case "{{LEVEL}}" in
                patch) pt=$((pt + 1));;
                minor) mn=$((mn + 1)); pt=0;;
                major) mj=$((mj + 1)); mn=0; pt=0;;
            esac
            new_version="$mj.$mn.$pt"
            ;;
        *)
            if [[ ! "{{LEVEL}}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
                echo "expected patch/minor/major or X.Y.Z, got: {{LEVEL}}" >&2
                exit 1
            fi
            new_version="{{LEVEL}}"
            ;;
    esac

    if git rev-parse "v$new_version" >/dev/null 2>&1; then
        echo "tag v$new_version already exists" >&2
        exit 1
    fi

    echo "→ $current → $new_version"

    # Bump only the [workspace.package].version line; per-crate
    # Cargo.toml files use `version.workspace = true`.
    sed -i 's/^version = "'"$current"'"$/version = "'"$new_version"'"/' Cargo.toml

    # Refresh Cargo.lock so workspace member entries reflect the new
    # version. `cargo update --workspace` only touches our own crates,
    # not third-party deps — keeps the diff minimal.
    cargo update --workspace --offline 2>/dev/null || cargo update --workspace

    git add Cargo.toml Cargo.lock
    git commit -m "release: $new_version"
    git tag -a "v$new_version" -m "Release v$new_version"

    if [[ " {{FLAGS}} " == *" --no-push "* ]]; then
        echo "created v$new_version locally; --no-push, stopping here."
        echo "to push:  git push origin HEAD 'v$new_version'"
    else
        git push origin HEAD "v$new_version"
        echo "pushed v$new_version. CI will build .deb + AppImage and publish a release."
    fi

# Lower-level: tag HEAD at the current Cargo.toml version (no bump,
# no commit). Use when you've manually edited Cargo.toml or want to
# retag a commit. Most users want `just release` instead.
release-tag:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! git diff --quiet HEAD; then
        echo "working tree is dirty — commit or stash first" >&2
        exit 1
    fi
    version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
    if git rev-parse "v$version" >/dev/null 2>&1; then
        echo "tag v$version already exists — bump Cargo.toml first" >&2
        exit 1
    fi
    git tag -a "v$version" -m "Release v$version"
    git push origin "v$version"
    echo "pushed v$version. CI will build .deb + AppImage and publish a release."

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
