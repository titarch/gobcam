# Gobcam

A goblin in your webcam. Adds animated emoji reactions (and eventually
effects) to a Linux webcam feed via `v4l2loopback`, so any video call app can
use the modified feed as a camera source. Built because Teams won't let you
thumbs-down the all-hands.

> **Status:** early days. Step 1 is in — a Rust + GStreamer daemon does plain
> webcam-to-loopback passthrough. Overlays, animations, IPC, and a UI come
> next. See `CLAUDE.md` for the build sequence and architectural rationale.

## How it works

```
/dev/video0  ──►  gobcam-pipeline (GStreamer)  ──►  /dev/video10
  (real cam)         v4l2src ! videoconvert !          (v4l2loopback)
                     v4l2sink                              ▼
                                                       Teams / Meet /
                                                       Zoom / browsers
```

A single-binary daemon (`gobcam-pipeline`) drives a GStreamer graph that reads
your webcam and writes to a `v4l2loopback` device. Any app that picks a
camera from `/dev/videoN` will see the loopback as "Gobcam". Future steps
splice overlay subgraphs into the same pipeline at runtime.

## Requirements

- Linux with kernel headers (for the DKMS module)
- Rust 1.92+ (toolchain is pinned via `rust-toolchain.toml`)
- GStreamer 1.x runtime + plugins (base, good)
- `v4l2loopback-dkms`
- A webcam at `/dev/video0` (or wherever)

The provided `scripts/setup-host.sh` installs the above on Arch and
Debian/Ubuntu. Other distros: install the equivalent packages by hand.

## Quickstart

```bash
# 1. Install runtime prereqs and load the loopback module (one-time / per boot)
./scripts/setup-host.sh

# 2. Bootstrap the dev environment (installs `just`, wires the pre-commit hook)
just setup           # or: ./scripts/setup-dev.sh

# 3. Run the daemon
just run             # defaults: -i /dev/video0 -o /dev/video10
```

Open Teams / Meet / Zoom / a browser, pick **Gobcam** as the camera. Done.

## Manual test playbook

Useful when iterating on the pipeline.

1. **Make sure the loopback exists**
   ```bash
   just modprobe-loopback     # creates /dev/video10 if not already loaded
   ls -l /dev/video10
   ```

2. **(optional) sanity-check the GStreamer graph in shell**
   The fastest feedback loop for "does this topology even make sense":
   ```bash
   just gst-passthrough                    # defaults: /dev/video0 → /dev/video10
   just gst-passthrough D=/dev/video2      # override input
   ```

3. **Run the daemon**
   ```bash
   just run
   just run -- -i /dev/video2 -o /dev/video10
   RUST_LOG=debug just run                  # noisier app logs
   GST_DEBUG=3 just run                     # gstreamer-level tracing
   ```

4. **Confirm something is on the loopback** — second terminal:
   ```bash
   just view-loopback                       # opens a window showing the loopback feed
   ```

5. **Confirm a real app sees it**
   - Browser: <https://webcamtests.com/> — your camera dropdown should list **Gobcam**.
   - Teams-for-Linux, Meet, Zoom: pick **Gobcam** in camera settings.

6. **Diagnose**
   ```bash
   just list-cam-formats                    # what /dev/video0 negotiates
   v4l2-ctl --list-devices                  # confirm /dev/video10 shows as "Gobcam"
   ```

   Common gotcha: if Chromium-based apps don't list Gobcam, the loopback was
   loaded without `exclusive_caps=1`. Reload it:
   ```bash
   sudo rmmod v4l2loopback && just modprobe-loopback
   ```

## Development

Every dev action goes through `just`:

| Recipe | What it does |
|---|---|
| `just run [-- ARGS]` | Run the daemon (forwards args after `--`) |
| `just fmt` / `just fmt-check` | Apply / check `rustfmt` |
| `just lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `just test` | `cargo test --workspace` |
| `just check` | `fmt-check + lint + test` — what the pre-commit hook runs |
| `just ci` | `check + docker-build` — full local "CI" gate, run before pushing |
| `just docker-build` | Build the release image via `docker/Dockerfile.build` |
| `just gst-passthrough` | Shell-level pipeline sanity check |
| `just modprobe-loopback` | Load `v4l2loopback` (`/dev/video10`, `exclusive_caps=1`) |
| `just view-loopback` | Consume the loopback in a viewer window |
| `just list-cam-formats` | `v4l2-ctl --list-formats-ext` for the input device |

`just --list` shows everything.

The pre-commit hook is installed automatically by `cargo-husky` on the first
`cargo test`. It runs `just check` and blocks the commit if anything fails.
There is no hosted CI yet — `just ci` is the source of truth and any future
hosted CI just shells out to it.

## Repository layout

```
gobcam/
├── Cargo.toml              workspace root (lints, profiles, shared deps)
├── rust-toolchain.toml     pinned toolchain
├── justfile                every dev entry point
├── .cargo-husky/hooks/     pre-commit hook installed via cargo-husky
├── crates/pipeline/        GStreamer daemon binary (Step 1)
├── docker/Dockerfile.build build-env image producing a release binary
└── scripts/                setup-host.sh, setup-dev.sh
```

`CLAUDE.md` is the canonical record of architectural commitments and the
ordered build sequence. Read it before proposing structural changes.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
