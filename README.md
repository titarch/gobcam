# Gobcam

A goblin in your webcam. Adds animated emoji reactions (and eventually
effects) to a Linux webcam feed via `v4l2loopback`, so any video call app can
use the modified feed as a camera source. Built because Teams won't let you
thumbs-down the all-hands.

> **Status:** Steps 1–6 are in (Step 6.5 included) — webcam → optional
> always-on emoji overlay → triggerable reactions with fade-in /
> drift-up / fade-out animation, driven from stdin, a Unix socket, or a
> Tauri 2 + Svelte 5 floating panel browsing the **whole Fluent emoji
> set** → loopback. Static 3D previews are predownloaded into a local
> cache (~30 s, ~45 MB); animated APNGs are fetched lazily on first
> click of each emoji. Up to 4 stacked reactions. See `CLAUDE.md` for
> the build sequence and architectural rationale, and
> `docs/step3-debug-report.md` for the upstream `gst-plugins-good` bug
> we worked around to get Step 3 shipping.

## How it works

```
/dev/video0  ──►  gobcam-pipeline (GStreamer)  ──►  /dev/video10
  (real cam)                ▲                        (v4l2loopback)
                            │                              ▼
              4 pre-allocated compositor slots      Teams / Meet /
              (APNG frame pump per slot,            Zoom / browsers
               GstController on alpha/ypos)
```

A single-binary daemon (`gobcam-pipeline`) drives a GStreamer graph that reads
your webcam, optionally composites emoji on top, and writes to a
`v4l2loopback` device. Any app that picks a camera from `/dev/videoN` will see
the loopback as "Gobcam".

Emoji come from the bundled `assets/fluent-catalog.json`, generated from
Microsoft's `fluentui-emoji` + `fluentui-emoji-animated` repos. The daemon
predownloads every static 3D preview into `$XDG_CACHE_HOME/gobcam/previews/`
on first run (~30 s, ~45 MB) and lazily fetches each emoji's animated APNG
into `animated/` the first time you trigger it. Animated emoji are decoded
from APNG in-process (no `gst-libav` runtime dep) and pushed into the
compositor as a live RGBA stream; non-animated emoji loop a single 3D frame
through the same machinery. Each slot is a permanent
`appsrc → videoconvert → queue → compositor` chain — triggering swaps the
slot's frame source and toggles its compositor pad's `alpha` from 0 to 1, so
the graph never reshapes at runtime. Triggered reactions fade in / drift up /
fade out via `GstController` interpolation curves on the slot pad.

## Requirements

- Linux with kernel headers (for the DKMS module)
- Rust 1.92+ (toolchain is pinned via `rust-toolchain.toml`)
- GStreamer 1.x runtime + plugins (base, good)
- `v4l2loopback-dkms`
- A webcam at `/dev/video0` (or wherever)

The provided `scripts/setup-host.sh` installs the above on Arch and
Debian/Ubuntu. Other distros: install the equivalent packages by hand.

## Install

Three paths, pick whichever matches your distro:

### `.deb` (Debian / Ubuntu / Mint / Pop!\_OS / WSL)

```bash
sudo apt install ./Gobcam_0.0.1_amd64.deb
```

The package's postinst loads `v4l2loopback`, drops the
`/etc/modules-load.d` and `/etc/modprobe.d` snippets so it auto-loads
on every boot, and writes a narrow `/etc/sudoers.d/gobcam` entry that
lets the auto-reset path skip a password prompt. After install, run
`gobcam` from any application launcher — no further setup needed.

`apt remove gobcam` cleans everything up.

### AppImage (every other distro)

```bash
chmod +x Gobcam_0.0.1_amd64.AppImage
./Gobcam_0.0.1_amd64.AppImage
```

On first run — when `/dev/video10` doesn't exist yet — the panel
opens to a **Set up Gobcam** prompt. Clicking it runs the bundled
`gobcam-setup` script via `pkexec` (graphical password prompt),
which loads the kernel module and drops `/etc/modules-load.d` /
`/etc/modprobe.d` snippets so the loopback comes back on every
reboot. After that, subsequent launches go straight to the panel.

Bundles GStreamer and WebKit (~115 MB) for portability.

### From source

See [Quickstart](#quickstart) below — that's the dev path.

## Quickstart

```bash
# 1. Install runtime prereqs (gstreamer plugins, v4l2loopback-dkms)
./scripts/setup-host.sh

# 2. One-time loopback install: drops /etc snippets so /dev/video10
#    is available with our options on every boot, no sudo prompts.
just install-loopback        # prompts for sudo / pkexec once

# 3. Bootstrap the dev environment (installs `just`, wires the pre-commit hook)
just setup           # or: ./scripts/setup-dev.sh

# 4. (optional) Regenerate the bundled Fluent catalog. The committed
#    assets/fluent-catalog.json already covers every emoji upstream
#    publishes; only run this if you want to refresh it.
just rebuild-catalog

# 4. Run the daemon, with or without an overlay or triggers
just run                                            # plain passthrough
just run -- --overlay fire                          # always-on fire emoji
just run -- --triggers-stdin                        # read emoji ids from stdin
echo fire | just run -- --triggers-stdin            # one-shot trigger
just run -- --overlay fire --triggers-stdin         # always-on + ad-hoc reactions
just run -- --socket "$XDG_RUNTIME_DIR/gobcam.sock"  # IPC trigger surface
```

### IPC

When the daemon is launched with `--socket <path>` it listens for
line-delimited JSON commands. The wire types live in
`crates/protocol/`. One-liner test:

```bash
SOCK="$XDG_RUNTIME_DIR/gobcam.sock"
just run -- --socket "$SOCK" &              # daemon in background
echo '{"type":"trigger","emoji_id":"fire"}' | ncat -U "$SOCK"
# → {"type":"ok"}
```

The daemon replies with `{"type":"ok"}` or
`{"type":"error","message":"..."}` per command, and the socket file is
unlinked when the daemon exits.

### UI

`crates/ui/` is a Tauri 2 + Svelte 5 floating panel that opens the
daemon's IPC socket and fires a reaction per click. **One launch
runs both** — the UI process spawns and supervises the
`gobcam-pipeline` daemon automatically:

```bash
just app   # alias for ui-dev: builds the daemon, runs the UI which spawns it
```

If a daemon is already running on the configured socket (e.g.
launched manually for `--profile-log` debugging), the UI attaches
instead of spawning a duplicate. On window close the UI closes the
daemon's stdin pipe; the daemon's `--exit-on-stdin-eof` watchdog
shuts it down cleanly. SIGKILL of the UI also works — the kernel
closes the pipe and the daemon notices.

The panel is 280×400, always-on-top (`_NET_WM_STATE_ABOVE`), and
non-resizable. Clicks invoke a `trigger` Tauri command which writes
one line to the socket and reads the response; failures (daemon
down, slots busy, unknown emoji) surface as a 3.5-second toast at
the bottom of the window. `just ui-build` produces a release binary
under `crates/ui/src-tauri/target/release/gobcam-ui`.

Tip for i3 users — pin the panel:
```
for_window [class="gobcam-ui"] floating enable, sticky enable, \
    move position 1640 px 80 px
```

Triggered reactions ride a default animation curve (Step 4): fade in over
~120 ms, drift upward by 30 px over the lifetime, fade out over the
final 400 ms. Always-on overlays (`--overlay <id>`) skip the animation
and stay pinned. Stack up to 4 reactions concurrently. The 5th queued
while all slots are busy fails fast with `all 4 slots busy` — wait for
an active reaction's 3-second timer to expire and the slot will be
reusable.

Open Teams / Meet / Zoom / a browser, pick **Gobcam** as the camera. Done.

### Available emoji

Every emoji that Microsoft's `fluentui-emoji` repo ships is in the bundled
catalog (`assets/fluent-catalog.json`, ~1500 entries, ~600 KB). The
`gobcam-ui` panel browses the full catalog with search and group dividers.
Static 3D previews are predownloaded into the cache on first daemon run;
animated APNGs are fetched on demand the first time you trigger each one.
To refresh the bundled catalog after upstream gains new emoji, run
`just rebuild-catalog`.

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

### Persistent loopback (recommended)

`just install-loopback` (one-time, prompts for sudo) drops two
snippets under `/etc/modules-load.d/` and `/etc/modprobe.d/` so the
kernel loads `v4l2loopback` at every boot with our options
(`/dev/video10`, `card_label=Gobcam`, `exclusive_caps=1`). After
that, `just app` works unattended after a reboot.

Uninstall: `bash scripts/gobcam-setup --uninstall`.

### Passwordless loopback reset (optional, dev only)

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

## Releasing

To cut a release:

```bash
# 1. Bump version in Cargo.toml's [workspace.package].
$EDITOR Cargo.toml
git commit -am "release: 0.1.0"

# 2. Tag and push. Triggers .github/workflows/release.yml on the
#    configured remote — same file works on Gitea Actions and
#    GitHub Actions.
just release-tag
```

The CI job runs `just docker-package` on a registered runner, then
attaches `.deb`, `.AppImage`, and `SHA256SUMS` to a release tied to
the tag. The workflow asserts the tag (`v0.1.0` → `0.1.0`) matches
`Cargo.toml`'s version and fails fast on drift.

The runner needs Docker. On Gitea, install `act_runner` natively on
a host with Docker, register against the instance with
`--labels ubuntu-latest`, and supervise via systemd. On GitHub,
hosted runners cover this for free on public repos.

## Development

Every dev action goes through `just`:

| Recipe | What it does |
|---|---|
| `just run [-- ARGS]` | Run the daemon (forwards args after `--`) |
| `just ui-dev` | Run the Tauri panel UI in dev mode (Vite + hot reload) |
| `just ui-build` | Build a release UI binary |
| `just ui-check` | Frontend gate: biome + svelte-check + Vitest |
| `just fmt` / `just fmt-check` | Apply / check `rustfmt` |
| `just lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `just test` | `cargo test --workspace` |
| `just check` | `fmt-check + lint + test + ui-check` — what the pre-commit hook runs |
| `just ci` | `check + docker-build` — full local "CI" gate, run before pushing |
| `just docker-build` | Build the release image via `docker/Dockerfile.build` |
| `just package` | Build a `.deb` and an AppImage under `target/release/bundle/` (host) |
| `just docker-package` | Same outputs, built inside a pinned Debian Trixie container — reproducible, sidesteps host-toolchain quirks (e.g. linuxdeploy strip vs. Arch's `.relr.dyn` sections) |
| `just rebuild-catalog` | Regenerate `assets/fluent-catalog.json` from upstream Microsoft repos |
| `just gst-passthrough` | Shell-level pipeline sanity check |
| `just modprobe-loopback` | Load `v4l2loopback` (`/dev/video10`, `exclusive_caps=1`) |
| `just reset-loopback` | Force-reload the loopback module (clears stuck state) |
| `just install-loopback` | One-time installer: makes v4l2loopback auto-load at boot with our options |
| `just uninstall-loopback` | Inverse: removes the `/etc` snippets + sudoers rule + rmmods the module |
| `just docker-test-deb` | Smoke-test the produced `.deb` in a fresh `debian:trixie` container |
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
│       ├── slots.rs                 N pre-allocated compositor sink pads + pumps
│       ├── reactions.rs             Reactor: trigger an emoji on a free slot
│       ├── effects.rs               GstController curves on the slot pad (Step 4)
│       ├── ipc.rs                   Unix-socket JSON command dispatch (Step 5)
│       ├── firewall.rs              v4l2sink CAPS-query workaround
│       ├── pipeline.rs              camera + compositor + sink topology
│       └── runner.rs                state machine, bus pump, SIGINT handling
├── crates/protocol/                 wire types shared by daemon and IPC clients
├── crates/ui/                       Tauri 2 + Svelte 5 floating panel
│   ├── src-tauri/                   Rust shell (workspace member)
│   └── src/                         Svelte components (TypeScript strict)
├── assets/fluent/manifest.toml      curated emoji list (synced PNGs are gitignored)
├── docker/Dockerfile.build          build-env image producing a release binary
└── scripts/                         setup-host.sh, setup-dev.sh, sync-emoji.sh
```

`CLAUDE.md` is the canonical record of architectural commitments and the
ordered build sequence. Read it before proposing structural changes.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
