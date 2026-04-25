# Gobcam

A goblin in your webcam. Adds animated emoji reactions (and eventually
effects) to a Linux webcam feed via `v4l2loopback`, so any video call app can
use the modified feed as a camera source. Built because Teams won't let you
thumbs-down the all-hands.

> **Status:** Steps 1, 2, and 3 are in — webcam → optional always-on emoji
> overlay → triggerable reactions with timer → loopback. Animated APNG and
> static PNG emoji from Microsoft's Fluent set, up to 4 stacked
> reactions. Procedural transforms, IPC, and a UI come next. See
> `CLAUDE.md` for the build sequence and architectural rationale, and
> `docs/step3-debug-report.md` for the upstream `gst-plugins-good` bug
> we worked around to get Step 3 shipping.

## How it works

```
/dev/video0  ──►  gobcam-pipeline (GStreamer)  ──►  /dev/video10
  (real cam)                ▲                        (v4l2loopback)
                            │                              ▼
              optional emoji overlay bin             Teams / Meet /
              (APNG frame pump or                    Zoom / browsers
               imagefreeze on PNG)
```

A single-binary daemon (`gobcam-pipeline`) drives a GStreamer graph that reads
your webcam, optionally composites an emoji on top, and writes to a
`v4l2loopback` device. Any app that picks a camera from `/dev/videoN` will see
the loopback as "Gobcam".

Emoji come from the curated set in `assets/fluent/manifest.toml`. Animated
emoji are decoded from APNG in-process (no `gst-libav` runtime dep) and pushed
into the compositor as a live RGBA stream; static emoji ride `imagefreeze`.
Both paths converge on the same compositor sink pad so triggering, stacking,
and procedural transforms (Step 3+) compose cleanly.

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

# 3. Sync the curated emoji set (~10 MB; one-time, idempotent)
just sync-emoji

# 4. Run the daemon, with or without an overlay or triggers
just run                                            # plain passthrough
just run -- --overlay fire                          # always-on fire emoji
just run -- --triggers-stdin                        # read emoji ids from stdin
echo fire | just run -- --triggers-stdin            # one-shot trigger
just run -- --overlay fire --triggers-stdin         # always-on + ad-hoc reactions
```

Stack up to 4 reactions concurrently. The 5th queued while all slots are
busy fails fast with `all 4 slots busy` — wait for an active reaction's
3-second timer to expire and the slot will be reusable.

Open Teams / Meet / Zoom / a browser, pick **Gobcam** as the camera. Done.

### Available emoji

The curated set is listed in `assets/fluent/manifest.toml`. The current
defaults are: `thumbs_up`, `red_heart`, `fire`, `party_popper`,
`smiling_face_with_smiling_eyes`. Add an entry to the manifest and rerun
`just sync-emoji` to expand the set.

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
   just run -- --overlay fire               # animated overlay
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

   Common gotchas:
   - **Chromium-based apps don't list Gobcam.** The loopback was loaded without
     `exclusive_caps=1`. Reload: `just reset-loopback`.
   - **`v4l2src: not a capture device` from a consumer.** The loopback got stuck
     in OUTPUT mode after a failed daemon run. Same fix: `just reset-loopback`.
   - **Emoji not found.** Run `just sync-emoji`. If it's missing, add the entry
     to `assets/fluent/manifest.toml` and rerun.

### Passwordless loopback reset (optional)

`just reset-loopback` requires `sudo` for `rmmod`/`modprobe`. To skip the
password prompt, install the included sudoers drop-in (one-time):

```bash
sudo install -m 0440 -o root -g root \
    scripts/sudoers-gobcam-dev /etc/sudoers.d/gobcam-dev
sudo visudo -c -f /etc/sudoers.d/gobcam-dev   # validate

# Edit the username inside the file first if you're not `bparsy`.
# Uninstall:  sudo rm /etc/sudoers.d/gobcam-dev
```

The rule grants the exact `modprobe v4l2loopback ...` and `rmmod v4l2loopback`
invocations only — anything else still prompts.

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
| `just sync-emoji` | Fetch curated Fluent assets per `assets/fluent/manifest.toml` |
| `just gst-passthrough` | Shell-level pipeline sanity check |
| `just modprobe-loopback` | Load `v4l2loopback` (`/dev/video10`, `exclusive_caps=1`) |
| `just reset-loopback` | Force-reload the loopback module (clears stuck state) |
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
├── Cargo.toml                       workspace root (lints, profiles, shared deps)
├── rust-toolchain.toml              pinned toolchain
├── justfile                         every dev entry point
├── .cargo-husky/hooks/              pre-commit hook installed via cargo-husky
├── crates/pipeline/                 GStreamer daemon binary
│   └── src/
│       ├── assets/                  Library trait + FluentLibrary + APNG decoder
│       ├── overlay.rs               gst::Bin builder for static + animated sources
│       ├── pipeline.rs              camera + compositor + sink topology
│       └── runner.rs                state machine, bus pump, SIGINT handling
├── assets/fluent/manifest.toml      curated emoji list (synced PNGs are gitignored)
├── docker/Dockerfile.build          build-env image producing a release binary
└── scripts/                         setup-host.sh, setup-dev.sh, sync-emoji.sh
```

`CLAUDE.md` is the canonical record of architectural commitments and the
ordered build sequence. Read it before proposing structural changes.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
