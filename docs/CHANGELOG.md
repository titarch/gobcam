# Changelog

All notable changes to Gobcam are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Pipeline

- Pin the compositor's src caps to `AYUV` (alpha + 4:4:4 YUV) so the
  trailing `videoconvert` only narrows YUV → I420 instead of doing a
  full colour-space matrix conversion from RGBA. Idle CPU at 1080p
  drops from ~50 % to ~17 %; saturated cascade drops from ~70 % to
  ~40 %. Alpha is preserved — emoji surrounds composite transparently
  over the camera, not as black rectangles. `firewall.rs` pins the
  v4l2sink format to `I420` to match the trailing convert.

### UI

- Settings → "Safe mode" toggle hides emojis flagged as suggestive or
  rude from the picker, recents, favorites, and the repeat-last
  hotkey. Triggering a hidden emoji via the hotkey shows a toast
  rather than firing.

### Packaging / CI

- Added a shared GitHub/Gitea Actions check workflow and split the
  release workflow only where the two platforms differ.
- Release builds now upload per-asset `.sha256` sidecars on both
  GitHub and Gitea.
- Marked AUR publication as upcoming and documented
  `just aur-install-local` as the current Arch path.

### Pipeline

- `gobcam-pipeline --input-io-mode {auto|rw|mmap|userptr|dmabuf|dmabuf-import}`
  exposes the `v4l2src` `io-mode` for cases where `auto` doesn't
  negotiate cleanly. Notably, feeding a v4l2loopback device into the
  daemon's input requires `rw`, since loopback's small REQBUFS pool
  starves under the buffer counts AUTO/MMAP request.

## [0.1.0] — 2026-04-26

Initial public release.

### Pipeline

- GStreamer daemon (`gobcam-pipeline`) reads a `v4l2` source, blends
  animated emoji onto the feed, and writes to a `v4l2loopback` device.
- 48 pre-allocated compositor slots; cascading animations with
  per-instance randomness on horizontal position, lifetime, and APNG
  playback rate.
- All animation parameters are live-tunable: lifetime, fade in/out
  curves, drift distance and angle, jitter, max concurrent reactions,
  drop policy on overflow.
- Per-`(emoji, frame)` `gst::Memory` cache so N slots showing the same
  frame share one allocation; idle pumps park on a condvar and emit
  zero buffers; idle pads drop out of the compositor blend entirely
  via `ignore-inactive-pads` + `max-last-buffer-repeat=0`.
- ~1500-emoji catalog bundled as JSON. Static 3D previews predownload
  to `$XDG_CACHE_HOME/gobcam`; animated APNGs are lazy-fetched on first
  trigger and decoded in-process.

### UI

- Tauri 2 + Svelte 5 floating panel (TypeScript strict, Tailwind v4).
- System tray (show/hide/quit), global hotkeys (toggle panel, repeat
  last reaction), recents, search.
- Settings page: pick input device, pick resolution/framerate, optional
  preview thumbnail (MJPEG over localhost), reaction slot count and
  per-slot canvas dimension.
- Animations page with live previews of every parameter.
- Persists to `$XDG_CONFIG_HOME/gobcam/config.json`.
- Supervises the daemon as a child process; `Stdio::piped()` stdin
  ensures clean shutdown on UI exit (or kernel pipe-close on SIGKILL).

### Packaging

- `.deb` and `AppImage` from a single packaging recipe via Tauri 2's
  bundler. The AUR `gobcam-bin` template consumes the `.deb`; public
  AUR publication is planned after the first GitHub release.
- `gobcam-setup` (shipped in both packages) installs the
  `v4l2loopback` modules-load snippet, modprobe options, and a narrow
  `sudoers.d` rule that lets the UI auto-reset the loopback when
  changing mode.
- `just docker-package` reproduces the build inside `debian:trixie` with
  the same script for cross-host parity.
- `just docker-test-deb` smoke-tests the `.deb` in a fresh `debian:trixie`
  container — verifies the maintainer-script snippets land correctly
  and `apt-get purge` removes them cleanly.
- Tag-triggered release workflow (`.github/workflows/release.yml`)
  builds and publishes both bundles plus `.sha256` sidecars.

### Notable workarounds

- **`firewall.rs`** — a CAPS-query pad probe on `v4l2sink.sink` works
  around a thread-safety bug in `gst-plugins-good`'s
  `gst_v4l2_object_probe_caps` that crashes with `free(): invalid
  pointer` when multiple upstream tasks query caps concurrently. See
  [`v4l2sink-thread-safety.md`](v4l2sink-thread-safety.md).
- **`WEBKIT_DISABLE_DMABUF_RENDERER=1`** — required everywhere the UI
  is launched on NVIDIA proprietary drivers, otherwise WebKitGTK
  cannot allocate GBM buffers and the window comes up blank. Set in
  the `justfile` recipes, the `.desktop` file shipped with the `.deb`,
  and the build container.

[Unreleased]: https://github.com/titarch/gobcam/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/titarch/gobcam/releases/tag/v0.1.0
