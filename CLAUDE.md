# Gobcam

A Linux virtual webcam tool that adds animated emoji reactions and (eventually) camera effects to a webcam feed, exposing the result as a v4l2loopback device that any video conferencing app (Teams, Meet, Zoom, etc.) can use as a camera source.

Primary motivation: replace missing emoji reactions in the unofficial Teams-for-Linux client.

## Architectural decisions (already made — do not relitigate without asking)

- **Virtual camera mechanism**: v4l2loopback kernel module. We do NOT integrate with Teams directly (no plugin API, fragile fork target). The output is a `/dev/videoN` device any app can consume.
- **Pipeline framework**: GStreamer, not raw ffmpeg. Reason: live, mutable, branching graph with runtime add/remove of elements is a first-class GStreamer operation. ffmpeg is for batch.
- **Language**: Rust for the pipeline daemon using `gstreamer-rs`. Reason: performance of C with sane error handling and lifetime guarantees on pipeline state. The UI layer is a separate process and language-agnostic; current intent is Tauri or GTK4 via gtk-rs, decided later.
- **Process model**: Two processes — a pipeline daemon and a UI client — communicating over a Unix socket with JSON commands. Shared types live in a `protocol` crate consumed by both. UI can restart without dropping the video pipeline.
- **Emoji asset library**: `microsoft/fluentui-emoji` (static, MIT) plus `microsoft/fluentui-emoji-animated` (animated APNG 256×256, MIT, served via Git LFS). NOT cloned wholesale — `assets/fluent/manifest.toml` curates which emoji we ship and `scripts/sync-emoji.sh` fetches each file individually so contributors don't need `git lfs install`. Other libraries (Twemoji, OpenMoji, Lottie sources) plug in behind the same `Library` trait without touching the pipeline.
- **Asset abstraction**: a `Library` returns a `Source` for an `(emoji_id, style, skin_tone)` key. `Source` is an enum: `StaticRaster` (PNG), `Animated` (decoded APNG frame stack), `StaticVector` (reserved for SVG). Library declares a `fallback_chain` so when the requested style isn't available we degrade gracefully (e.g. `Animated → Render3D`). Earlier "WebM/VP9-alpha" plan from v1 is superseded by APNG + in-Rust decode.
- **APNG decoding**: in-Rust via the `image` crate (`png` feature), pushed as raw RGBA into an `appsrc`. No `gst-libav` runtime dependency; we own loop/speed/reverse semantics.
- **Animation mechanism**: two layers compose multiplicatively.
  1. **Internal animation** (the source): for animated emoji, an APNG frame pump thread pushes RGBA buffers with monotonic PTS into an `appsrc`, looping the index. Static emoji use `appsrc + imagefreeze` for the same downstream caps.
  2. **External transform** (the compositor pad): `xpos`, `ypos`, `alpha`, `scale`, eventually rotation, animated via `GstController` interpolation control sources. Do not run a manual tick loop. Step 3 wires the first transforms.
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
├── Cargo.toml                    # workspace root, workspace.lints, profiles
├── rust-toolchain.toml           # pinned toolchain
├── justfile                      # every dev entry point
├── .cargo-husky/hooks/           # pre-commit hook installed via cargo-husky
├── crates/
│   └── pipeline/                 # GStreamer daemon (Step 1, 2 done)
│       └── src/
│           ├── assets/           # Library trait, FluentLibrary, APNG decoder
│           ├── overlay.rs        # gst::Bin builder for static + animated sources
│           ├── pipeline.rs       # camera + compositor + sink topology
│           ├── runner.rs         # state-machine driver, bus pump, SIGINT → EOS
│           ├── cli.rs            # clap CLI
│           ├── lib.rs            # entry point
│           └── main.rs
├── assets/fluent/
│   ├── manifest.toml             # curated emoji list (committed)
│   └── <emoji>/[<tone>/]<style>/ # synced PNGs (gitignored)
├── docker/Dockerfile.build       # build-env image producing a release binary
├── scripts/
│   ├── setup-host.sh             # modprobe v4l2loopback, install runtime libs
│   ├── setup-dev.sh              # install just, wire husky hook
│   └── sync-emoji.sh             # fetch curated assets per manifest.toml
└── (later) crates/{protocol,ui}/, packaging/, assets/animations/
```

## Build sequence (do these in order — each step gates the next)

1. **Native hello-world pipeline** ✅ done: `v4l2src ! videoconvert ! v4l2sink` in `crates/pipeline`. Run with `just run`.
2. **Always-on overlay (static + animated)** ✅ done: `compositor` topology, `Library`/`Source` abstraction, Fluent assets via curated manifest, APNG frame pump for animated emoji, `imagefreeze` chain for static. `just run -- --overlay <id>`.
3. **Triggered overlay with timer**: a CLI command (stdin or signal — no IPC yet) that splices the overlay subgraph in for 3 seconds and unlinks it cleanly. This is the real technical milestone — dynamic relinking is the part with the most failure modes (state changes, pad probes, EOS handling).
4. **Procedural transform layer** via `GstController` interpolation control sources on compositor pad properties (xpos/ypos/scale/alpha/rotation). Composes with internal APNG animation; first concrete effects (bounce, drift, fade-out).
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
- **Compositor caveat**: when mixing live (camera) and non-live (overlay) branches, every branch needs a `queue` between source and compositor and the live source needs a fixated framerate via capsfilter, otherwise the aggregator's latency negotiation deadlocks (`max 0 < min 33ms`). Already wired into `pipeline.rs` — don't remove without understanding why.
- **v4l2loopback can stick after a failed run**: if the daemon enters PLAYING but never pushes buffers (e.g. caps-negotiation deadlock), the loopback stays in OUTPUT-only mode and downstream `v4l2src` consumers see "not a capture device". Reset with `just reset-loopback`. With `scripts/sudoers-gobcam-dev` installed at `/etc/sudoers.d/gobcam-dev`, this runs without a password prompt and is pre-allowed in `.claude/settings.json` so I can run it autonomously when stuck state is detected.
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

Steps 1 (passthrough), 2 (always-on emoji overlay, static + animated), and 7 (Docker build env) done. **Step 3 is in-progress and blocked**: the `--triggers-stdin` machinery (Reactor, stdin reader, IDLE-probe detach) is implemented and the CLI flag works, but every dynamic add to the running pipeline fails with `gst_base_src_loop: streaming stopped, reason not-linked` from the freshly added appsrc. Reproducible across many variations (link/sync ordering, ghost-pad vs flat elements, `is_live` true/false, `need-data` callbacks, `ignore-inactive-pads`, `sink_pad.set_active`). Working hypothesis: dynamic `compositor.request_pad_simple` + cross-element link races the chain's pad activation regardless of how we sequence calls. Likely fix is **pre-allocated overlay slots** — request N compositor sink pads at startup, gate them via per-pad `alpha` (the same property Step 4 will animate). See `.plans/step3-triggered-overlays.md` for the full debug log and next-session plan.
