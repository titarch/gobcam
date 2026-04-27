# Architecture

Gobcam adds animated emoji reactions to Linux video calls by drawing
them into the camera stream itself. The daemon reads the real webcam,
blends emoji onto the feed, and publishes the result on a
`v4l2loopback` device that normal camera apps can open.

## Two processes

```
┌────────────────────┐  Unix socket      ┌──────────────────────┐
│ gobcam-ui (Tauri)  │  ◄── JSON ──►     │ gobcam-pipeline       │
│ floating panel +   │                    │ GStreamer daemon     │
│ tray + hotkeys     │                    │                       │
└────────────────────┘                    └──────────────────────┘
                                                    │
                                       /dev/video0  ▼  /dev/video10
                                                    │  (v4l2loopback,
                                       webcam ─────►│   "Gobcam")
                                                    │
                                                    ▼
                                            Teams / Meet / Zoom / …
```

The UI is the normal entry point. On launch it either attaches to an
existing daemon socket or starts `gobcam-pipeline` itself. When it starts
the daemon, it keeps a pipe to the daemon's stdin so daemon shutdown
follows UI shutdown.

The split is deliberate: the pipeline can keep running while the panel
restarts, so rebuilding the UI during development does not drop the
camera mid-call. Running `gobcam-pipeline` by hand is still useful for
debugging, scripted triggers, and minimal GStreamer repros, but users do
not need to start it separately.

## Daemon

`gobcam-pipeline` is a Rust binary that drives this GStreamer graph:

```
v4l2src ──► queue ──► videoconvert ──┐
                                      ├──► compositor ──► videoconvert ──► v4l2sink
appsrc(slot 0)  ──► …  ──────────────┤
appsrc(slot 1)  ──► …  ──────────────┤
…                                     │
appsrc(slot N)  ──► …  ──────────────┘
```

Each slot is a permanent appsrc → videoconvert → queue chain feeding one
compositor sink pad. Triggering a reaction picks a free slot, swaps its
frame source to the requested emoji, and animates the slot pad's
`alpha`/`xpos`/`ypos` via `GstController` interpolation curves. The graph
shape does not change while the daemon is running; slots are pre-allocated.

48 slots is the default. Idle slots are cheap (see "Idle cost" below),
and the count is also the hard limit for simultaneous reactions.

### Asset library

Emoji come from Microsoft's [Fluent UI Emoji][fluent] (MIT). A bundled
catalog (`assets/fluent-catalog.json`, ~1500 entries) is generated once
from the upstream repos and committed. On first run the daemon
predownloads every static 3D preview into `$XDG_CACHE_HOME/gobcam/previews/`
(~30 s, ~45 MB on a fast link). Animated APNGs are downloaded lazily on
first trigger of each emoji (~350 ms typical).

A `Library` trait abstracts over emoji sources; `FluentLibrary` is the
only implementation today. Each emoji resolves to a `Source` enum:
`StaticRaster` (PNG), `Animated` (decoded APNG frames), or
`StaticVector` (reserved). The library declares a fallback chain so
missing styles degrade gracefully (animated → static).

APNG decoding is in-Rust via the `image` crate. No `gst-libav` runtime
dependency.

### Animation model

Two layers compose multiplicatively:

1. **Source animation** — the appsrc-side frame pump. Animated emoji
   loop their APNG frames with monotonic PTS; static emoji push a
   single frame.
2. **Pad animation** — `xpos`, `ypos`, `alpha` driven by `GstController`
   interpolation curves on the compositor sink pad. This is what
   produces the cascade: spawn at bottom-center, drift up, fade out.
   Per-instance horizontal and speed jitter keeps repeated reactions from
   moving in lockstep.

All animation parameters are live-tunable from the UI's animation page —
the daemon swaps an `Arc<RwLock<AnimationConfig>>` snapshot and the next
trigger picks up the new values without a respawn. Curves already in
flight keep their original shape.

### Idle cost

48 idle slots emit zero buffers. Each pump pushes one transparent seed
buffer at startup, then sleeps on a condvar; `try_activate` notifies the
condvar before flipping `alpha=1`, the pump wakes, pushes the emoji's
frames at APNG cadence, and goes back to sleep on deactivate.

The compositor's `ignore-inactive-pads=true` plus each slot pad's
`max-last-buffer-repeat=0` drops idle pads from the blend entirely once
their seed buffer expires. Combined with a per-`(emoji, frame)`
`gst::Memory` cache shared across slots, idle CPU at N=48 sits around
32 % on a Ryzen-class machine — a substantial drop from the naive ~110 %.

## UI

A Tauri 2 shell (Rust) hosting a Svelte 5 frontend. The Rust side keeps
a lazy `IpcClient` (single `UnixStream`, reconnect-on-failure) and a
`DaemonGuard` that supervises the daemon child process. Closing the
guard's stdin triggers the daemon's stdin-EOF watchdog so the process
exits cleanly; `SIGKILL` of the UI hits the same path via the kernel
auto-closing the pipe.

Settings, hotkeys, recents, and animation parameters persist to
`$XDG_CONFIG_HOME/gobcam/config.json`. The settings page lets you pick
the input device, pick a resolution/framerate from what the device
exposes, and enable a low-fps preview thumbnail; changing these
respawns the daemon. Hotkeys go through `tauri-plugin-global-shortcut`
and bypass the UI entirely on the trigger path.

## Output device

[`v4l2loopback`][v4l2lo] is a kernel module that creates virtual
`/dev/videoN` devices. Gobcam ships a one-shot installer (`gobcam-setup`)
that drops a `modules-load.d` snippet so the loopback comes up at boot
and a narrow `sudoers.d` rule that lets the UI auto-reset the loopback
when changing modes (a consumer locks the format until the device is
torn down).

`exclusive_caps=1` is required for Chromium-based apps to list the
device as a camera source.

## Why these choices

- **GStreamer rather than ffmpeg.** A live, mutable graph with runtime
  add/remove of elements is a first-class GStreamer operation; ffmpeg
  is for batch.
- **Rust for the daemon.** The pipeline needs C-level performance with
  sane error handling and lifetime safety on pipeline state.
- **Two processes, not one.** Restarting the UI shouldn't drop the
  webcam mid-call.
- **Emoji on disk, decoded in-Rust.** Avoids the ~150 MB `gst-libav`
  runtime dep and lets us own loop/speed/reverse semantics.
- **No GPU pipeline.** The CPU path stays under 35 % at idle and ~30 %
  during cascades on consumer hardware. `glvideomixer` is the next
  lever if blend cost becomes a bottleneck, but the v4l2 boundaries
  force CPU-side memory anyway.

## Repository layout

```
gobcam/
├── Cargo.toml                     workspace root
├── justfile                       all dev entry points
├── crates/
│   ├── pipeline/                  GStreamer daemon
│   ├── protocol/                  shared IPC types
│   └── ui/                        Tauri 2 + Svelte 5 panel
├── assets/
│   ├── fluent-catalog.json        emoji index (~1500 entries)
│   └── demo/                      demo source video for screenshots
├── docs/                          architecture + reference
├── packaging/                     .deb / AUR / sudoers / modprobe
└── scripts/                       setup, packaging, catalog rebuild
```

[fluent]: https://github.com/microsoft/fluentui-emoji
[v4l2lo]: https://github.com/v4l2loopback/v4l2loopback
