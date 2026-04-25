# Gobcam

A Linux virtual webcam tool that adds animated emoji reactions and (eventually) camera effects to a webcam feed, exposing the result as a v4l2loopback device that any video conferencing app (Teams, Meet, Zoom, etc.) can use as a camera source.

Primary motivation: replace missing emoji reactions in the unofficial Teams-for-Linux client.

## Architectural decisions (already made — do not relitigate without asking)

- **Virtual camera mechanism**: v4l2loopback kernel module. We do NOT integrate with Teams directly (no plugin API, fragile fork target). The output is a `/dev/videoN` device any app can consume.
- **Pipeline framework**: GStreamer, not raw ffmpeg. Reason: live, mutable, branching graph with runtime add/remove of elements is a first-class GStreamer operation. ffmpeg is for batch.
- **Language**: Rust for the pipeline daemon using `gstreamer-rs`. Reason: performance of C with sane error handling and lifetime guarantees on pipeline state. The UI layer is a separate process and language-agnostic; current intent is Tauri or GTK4 via gtk-rs, decided later.
- **Process model**: Two processes — a pipeline daemon and a UI client — communicating over a Unix socket with JSON commands. Shared types live in a `protocol` crate consumed by both. UI can restart without dropping the video pipeline.
- **Emoji asset format**: WebM with VP9 alpha channel for v1. Lottie via rlottie is a future option if vector quality is wanted.
- **Animation mechanism**: GstController with interpolation control sources on compositor pad properties (`xpos`, `ypos`, `alpha`, `scale`). Do not run a manual tick loop.
- **GPU usage**: Hardware available is an NVIDIA RTX 4080. For v1 (passthrough + occasional overlay) a CPU pipeline is expected to use <5% CPU at 1080p30 — do NOT add GPU paths until profiling shows they're needed. The v4l2 boundaries force CPU-side memory anyway, so naive GPU offload often costs more than it saves.
- **Build reproducibility**: Docker for the build environment only. Runtime stays native on the host (cameras, GPU, kernel module, display server make containerized runtime more friction than value). The host needs `v4l2loopback-dkms` and the GStreamer runtime plugin packages installed; this is acknowledged and documented in `scripts/setup-host.sh`.
- **Plugin/extensibility system**: Deliberately deferred. Each effect/reaction is a Rust module that knows how to splice itself into the pipeline. Promote to a real plugin ABI only after 3–4 effects exist and a real extension pattern has emerged.
- **Task runner & dev cycle**: `just` is the entry point for every dev action. `just check` (fmt + clippy + test) is the cheap pre-commit gate; `just ci` runs the same plus the docker image build, intended before pushing. No hosted CI is configured — the local `just ci` recipe is the source of truth, and any future hosted CI must shell out to it rather than duplicate check definitions.
- **Pre-commit hook**: `cargo-husky` (dev-dependency) auto-installs `.git/hooks/pre-commit` on first `cargo test`. The hook runs `just check`. No external Python/`pre-commit` framework dependency.
- **Toolchain pin**: `rust-toolchain.toml` pins the channel (currently 1.92) with `rustfmt` + `clippy` components — same toolchain in local, Docker, and any future CI.
- **Lint posture**: `clippy::pedantic` + `clippy::nursery` warned at workspace level, denied via `-D warnings` in `just lint`. Allow-list lives in `[workspace.lints.clippy]` in the root `Cargo.toml` and should grow only when a lint is genuinely noisy, not to silence real findings.

## Project layout (target)

```
gobcam/
├── Cargo.toml              # workspace root, workspace.lints, profiles
├── rust-toolchain.toml     # pinned toolchain
├── justfile                # every dev entry point
├── .cargo-husky/hooks/     # pre-commit hook installed via cargo-husky
├── crates/
│   ├── pipeline/           # GStreamer daemon binary (exists; Step 1 done)
│   ├── protocol/           # shared command/event types (Step 5)
│   └── ui/                 # control panel (Step 6)
├── assets/emoji/           # WebM/VP9 animated reactions (Step 4+)
├── docker/
│   └── Dockerfile.build    # build-env image producing a release binary
├── scripts/
│   ├── setup-host.sh       # modprobe v4l2loopback, install runtime libs
│   └── setup-dev.sh        # install just, wire husky hook
└── packaging/              # .deb / AppImage recipes (Step 8)
```

## Build sequence (do these in order — each step gates the next)

1. **Native hello-world pipeline** ✅ done: `v4l2src ! videoconvert ! v4l2sink` in `crates/pipeline`. Run with `just run`.
2. **Static overlay**: hardcode a `compositor` element with a single PNG always visible. Confirms compositor pad model.
3. **Triggered overlay with timer**: a CLI command (stdin or signal — no IPC yet) that splices the overlay subgraph in for 3 seconds and unlinks it cleanly. This is the real technical milestone — dynamic relinking is the part with the most failure modes (state changes, pad probes, EOS handling).
4. **Animated WebM asset** with motion path via GstController interpolation control sources on compositor pad properties.
5. **IPC layer**: define the `protocol` crate (commands like `TriggerReaction { emoji_id, position }`, events like `ReactionStarted`, `PipelineError`). Unix socket + JSON. Daemonize.
6. **UI**: Tauri or GTK4 panel of buttons that sends commands.
7. **Docker build environment** ✅ scaffolded alongside Step 1: `docker/Dockerfile.build` produces a release binary via `just docker-build`. A `Dockerfile.dev` interactive shell is still a future addition.
8. **Polish**: systemd user service, hotkey support, asset manifest config, multiple simultaneous reactions stacking via separate compositor pads.

## Operating notes for Claude Code

- I'm an experienced software engineer comfortable with both low-level and high-level work. Don't over-explain language fundamentals or basic Cargo usage.
- Push back on me if I'm about to make a decision that conflicts with the architectural commitments above without good reason.
- When implementing a step, write the smallest thing that proves the step works, run it, then expand. Don't write speculative scaffolding for future steps.
- Use `gst-launch-1.0` from the shell to validate pipeline topologies before encoding them in Rust — it's the fastest feedback loop for "does this graph even make sense". The `just gst-passthrough` recipe is the canned form.
- Webcam capture format negotiation is finicky. Use `v4l2-ctl --list-formats-ext -d /dev/video0` (or `just list-cam-formats`) to see what the device actually offers and be explicit in caps filters rather than letting GStreamer guess.
- The `exclusive_caps=1` modprobe flag for v4l2loopback matters — without it, Chromium-based apps (including Teams-for-Linux) won't list the loopback device as a camera source.
- Development cycle expected for every change: implement → add a test (new feature) or regression test (bug fix) → `just check` → if anything pipeline-touching, `just ci` → commit (the husky hook gates this). Update this file when a decision changes.

## Useful one-liners

```bash
# Load v4l2loopback (one-time per boot until we add a modules-load.d entry)
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label="ReactionCam" exclusive_caps=1

# Inspect real webcam capabilities
v4l2-ctl --list-formats-ext -d /dev/video0

# Quick pipeline sanity check (real cam → loopback, no effects)
gst-launch-1.0 v4l2src device=/dev/video0 ! videoconvert ! v4l2sink device=/dev/video10

# Test the loopback shows something (consumer side)
gst-launch-1.0 v4l2src device=/dev/video10 ! videoconvert ! autovideosink
```

## Current status

Step 1 (passthrough) and Step 7 (Docker build environment) done. `just check` and `just ci` green on the initial commit. Next up: Step 2 (static PNG overlay via `compositor`).
