set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# One-time developer setup: install just (if missing) and wire git hooks.
[group('setup')]
setup:
    scripts/setup-dev.sh

# Run the pipeline daemon. Forwards args after `--`.
[group('pipeline')]
run *ARGS:
    cargo run -p gobcam-pipeline -- {{ARGS}}

# Apply rustfmt across the workspace.
[group('dev')]
fmt:
    cargo fmt --all

# Check rustfmt across the workspace (no edits).
[group('dev')]
fmt-check:
    cargo fmt --all -- --check

# Clippy the workspace, deny warnings. Excludes the playground examples.
[group('dev')]
lint:
    cargo clippy --workspace --lib --bins --tests --all-features -- -D warnings

# Run all workspace tests.
[group('dev')]
test:
    cargo test --workspace --all-features

# Pre-commit gate: fmt + lint + test (Rust + frontend) with one-line status per step.
[group('dev')]
check:
    scripts/run-checks.sh

# Local CI gate: `check` plus the docker image build.
[group('dev')]
ci: check docker-build

# Build the dev docker image used by `ci`.
[group('dev')]
docker-build:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.build -t gobcam:dev .

# Run the contributor gate inside the packaging image. Used by hosted CI.
[group('dev')]
docker-check: docker-package-image
    docker run --rm \
        --user "$(id -u):$(id -g)" \
        -v "$PWD:/workspace" \
        --tmpfs /workspace/crates/ui/node_modules:exec,uid=$(id -u),gid=$(id -g) \
        -e HOME=/tmp \
        -e CARGO_HOME=/workspace/.cargo-docker \
        -e CI=true \
        --entrypoint bash \
        gobcam-package:dev scripts/run-checks.sh

# `docker-check` variant for runners spawned inside Docker (act_runner DinD).
[group('dev')]
docker-check-ci: docker-package-image
    #!/usr/bin/env bash
    set -euo pipefail
    container_id=$(cat /etc/hostname)
    docker run --rm \
        --volumes-from "$container_id" \
        -w "$PWD" \
        -e HOME=/tmp \
        -e CARGO_HOME=/tmp/cargo \
        -e CI=true \
        --entrypoint bash \
        gobcam-package:dev scripts/run-checks.sh

# Tauri panel in dev mode. WEBKIT_DISABLE_DMABUF_RENDERER=1 dodges WebKitGTK's GBM failure on NVIDIA.
[group('ui')]
ui-dev:
    pnpm -C crates/ui install
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri dev

# Alias for `ui-dev`. The UI supervises the daemon.
[group('app')]
app: ui-dev

# Build a release UI binary.
[group('ui')]
ui-build:
    pnpm -C crates/ui install
    WEBKIT_DISABLE_DMABUF_RENDERER=1 pnpm -C crates/ui tauri build

# Frontend gate: lockfile install, lint, type-check, test.
[group('ui')]
ui-check:
    pnpm -C crates/ui install --frozen-lockfile
    pnpm -C crates/ui run lint
    pnpm -C crates/ui run check-types
    pnpm -C crates/ui run test

# Build a .deb (Debian/Ubuntu) and an AppImage. Outputs in target/release/bundle/.
[group('release')]
package:
    scripts/package.sh

# Build the Debian-Trixie packaging image. One-time; rebuild on Dockerfile changes.
[group('release')]
docker-package-image:
    DOCKER_BUILDKIT=1 docker build -f docker/Dockerfile.package -t gobcam-package:dev .

# Same outputs as `package`, built inside the pinned Trixie image.
[group('release')]
docker-package: docker-package-image
    docker run --rm \
        --user "$(id -u):$(id -g)" \
        -v "$PWD:/workspace" \
        --tmpfs /workspace/crates/ui/node_modules:exec,uid=$(id -u),gid=$(id -g) \
        -e HOME=/tmp \
        -e CARGO_HOME=/workspace/.cargo-docker \
        -e CI=true \
        gobcam-package:dev

# `docker-package` variant for runners spawned inside Docker (act_runner DinD).
[group('release')]
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

# Locally simulate the release workflow without pushing a tag.
[group('release')]
ci-local:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo_version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

    GITHUB_REF_NAME="v$cargo_version"
    tag="${GITHUB_REF_NAME#v}"
    if [ "$tag" != "$cargo_version" ]; then
        echo "::error::tag $tag does not match Cargo.toml $cargo_version"
        exit 1
    fi
    echo "✓ verify tag matches Cargo.toml ($GITHUB_REF_NAME)"

    rm -rf target/release/bundle/deb target/release/bundle/appimage
    just docker-package

    for artifact in \
        target/release/bundle/deb/Gobcam_*.deb \
        target/release/bundle/appimage/Gobcam_*.AppImage
    do
        if [ ! -f "$artifact" ]; then
            echo "::error::missing release asset matching $artifact"
            exit 1
        fi
        (
            cd "$(dirname "$artifact")"
            sha256sum "$(basename "$artifact")" > "$(basename "$artifact").sha256"
        )
    done
    echo "✓ checksum sidecars:"
    find target/release/bundle -name 'Gobcam_*.sha256' -print -exec sed 's/^/    /' {} \;

    echo
    echo "All testable steps passed. Push a real tag with \`just release-tag\`"
    echo "to exercise the upload step."

# Cut a release: bump version, refresh lock, commit, tag, push. Triggers the release workflow.
# Usage: `just release patch | minor | major | X.Y.Z` — append `--no-push` for local only.
[group('release')]
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

    sed -i 's/^version = "'"$current"'"$/version = "'"$new_version"'"/' Cargo.toml
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

# Tag HEAD at the current Cargo.toml version (no bump). Most users want `release`.
[group('release')]
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

# Materialize PKGBUILD from PKGBUILD.in (pkgver from Cargo.toml, sha256 from the local .deb).
[group('release')]
aur-bump:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v makepkg >/dev/null; then
        echo "makepkg not found — run on Arch (or in an Arch container)." >&2
        exit 1
    fi
    version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
    deb="target/release/bundle/deb/Gobcam_${version}_amd64.deb"
    if [ ! -f "$deb" ]; then
        echo "missing $deb — run \`just package\` (or \`just docker-package\`) first." >&2
        exit 1
    fi
    sha=$(sha256sum "$deb" | cut -d' ' -f1)
    pkgdir=packaging/aur/gobcam-bin
    sed -e "s/@PKGVER@/${version}/g" -e "s/@SHA256@/${sha}/g" \
        "${pkgdir}/PKGBUILD.in" > "${pkgdir}/PKGBUILD"
    ( cd "${pkgdir}" && makepkg --printsrcinfo > .SRCINFO )
    echo "[aur] materialized ${pkgdir}/PKGBUILD (${version}, sha256=${sha:0:12}…)"
    echo "[aur] sync to aur.archlinux.org from your local AUR clone:"
    echo "      cp ${pkgdir}/{PKGBUILD,.SRCINFO} <aur-clone>/"
    echo "      git -C <aur-clone> commit -am 'Bump to ${version}' && git -C <aur-clone> push"

# Install the AUR package straight off the local .deb (no GitHub release needed).
[group('release')]
aur-install-local: package aur-bump
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
    deb="target/release/bundle/deb/Gobcam_${version}_amd64.deb"
    pkgdir=packaging/aur/gobcam-bin
    cp -f "$deb" "$pkgdir/"
    trap 'rm -f "$pkgdir/Gobcam_${version}_amd64.deb"' EXIT
    cd "$pkgdir"
    makepkg -si --noconfirm
    echo "[aur] installed gobcam-bin ${version} via pacman."
    echo "[aur] follow up with \`sudo gobcam-setup\` for the sudoers rule."

# Smoke-test the produced .deb in a clean debian:trixie container.
[group('release')]
docker-test-deb:
    bash scripts/test-deb.sh

# Regenerate the bundled Fluent emoji catalog from upstream Microsoft repos.
[group('assets')]
rebuild-catalog:
    python3 scripts/build-fluent-catalog.py

# Validate a passthrough graph from the shell.
[group('gst')]
gst-passthrough D='/dev/video0' OUT='/dev/video10':
    gst-launch-1.0 v4l2src device={{D}} ! videoconvert ! v4l2sink device={{OUT}}

# One-shot v4l2loopback load (not persistent across reboots).
[group('loopback')]
modprobe-loopback:
    sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=Gobcam exclusive_caps=1

# One-time installer: drops /etc snippets so v4l2loopback auto-loads at boot.
[group('loopback')]
install-loopback:
    bash scripts/gobcam-setup

# Inverse of `install-loopback`: removes /etc snippets, sudoers rule, and rmmods the module.
[group('loopback')]
uninstall-loopback:
    bash scripts/gobcam-setup --uninstall

# Force-reset v4l2loopback (clears stuck OUTPUT-mode state after a failed run).
[group('loopback')]
reset-loopback:
    -sudo rmmod v4l2loopback
    sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=Gobcam exclusive_caps=1

# Consume the loopback to confirm the daemon's output is visible.
[group('gst')]
view-loopback:
    gst-launch-1.0 v4l2src device=/dev/video10 io-mode=mmap ! \
        queue max-size-buffers=1 max-size-time=0 max-size-bytes=0 leaky=downstream ! \
        videoconvert ! \
        autovideosink sync=false

# Inspect what the real webcam can negotiate.
[group('gst')]
list-cam-formats D='/dev/video0':
    v4l2-ctl --list-formats-ext -d {{D}}

# Consumer-side latency probe. Watches the loopback and timestamps the first changed frame.
[group('perf')]
perf-capture *ARGS:
    cargo run -p gobcam-pipeline --release --example perf_capture -- {{ARGS}}

# Drive a saturated cascade against a running daemon for 60 s and capture CPU+GPU samples under perf-runs/.
[group('perf')]
perf-cascade LABEL='compositor=cpu':
    scripts/perf-cascade.sh '{{LABEL}}'

# Capture the README hero GIF: feeds the demo clip through Gobcam, triggers a scripted reaction sequence, encodes via gifski.
[group('demo')]
demo-capture:
    bash scripts/capture-demo.sh

# Capture the README emoji-picker panel screenshot with mocked Tauri APIs.
[group('demo')]
demo-ui-capture:
    bash scripts/capture-ui-screenshot.sh
