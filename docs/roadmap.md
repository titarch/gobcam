# Roadmap

Forward-looking work, in rough order of likelihood. Nothing here is
committed; open an issue if you want to push something up the list or
discuss a different shape.

## Likely next

- **Per-emoji effect choices.** Today every reaction uses the same
  cascade (drift up, fade out). Some emoji (party popper, fire, heart)
  have natural alternative behaviours — explosion, shake, scale-up —
  that would be worth wiring as opt-in per-emoji effect presets.
- **`tauri-specta` for typed bindings.** With several IPC commands
  now stable, hand-written `invoke` types in the frontend are starting
  to drift. `tauri-specta` would generate them from the Rust handlers.
- **systemd user service.** For users who want Gobcam to come up on
  login without keeping the panel open. Pairs with the existing
  daemon-attach path.
- **AUR `gobcam-bin` publish.** The `PKGBUILD.in` and local install
  path exist. Once the first GitHub release is public, push the
  materialized PKGBUILD to `aur.archlinux.org`. Until then, Arch users
  can build locally with `just aur-install-local`.

## Possible

- **`.rpm` bundle** alongside `.deb` and AppImage. Tauri's bundler
  doesn't support it natively; would need either `nfpm` or a hand-rolled
  conversion from the `.deb`.
- **aarch64 builds.** Cross-compiling Tauri + WebKit is non-trivial;
  realistic only if there's user demand.
- **Code signing** for the `.deb` and AppImage, plus matching
  `SIGSTORE`/`cosign` artifacts on the release.
- **Skin-tone variants.** Today the daemon treats `SkinTone::Default`
  as identical to `None` and ignores other variants. The catalog has
  the metadata; the UI just needs a picker.
- **Additional emoji libraries.** `Library` is a trait — Twemoji,
  OpenMoji, or custom Lottie sources should plug in without touching
  the pipeline.
- **More CI coverage.** The shared GitHub/Gitea Actions gate mirrors
  `just check`. Next useful additions would be package smoke tests and
  a release dry-run before tags.

## Probably not

- **Plugin / external effect ABI.** Each effect is a Rust module today;
  promoting to a real ABI is only worth it once 3-4 effects exist and
  a real extension pattern has emerged.
- **macOS / Windows.** v4l2loopback is Linux-specific, so anything
  cross-platform would need a fundamentally different output mechanism.
- **GPU compositing (`compositor` → `glvideomixer`).** Explored
  2026-04-27 with `just perf-cascade` (`scripts/perf-cascade.sh`) and a
  synthetic `gst-launch` N=48 stress. Findings:
  - Phase 0 baseline (RTX 4080, single-emoji `fire` cascade, 60 s):
    daemon CPU avg **69.5 %** (p95 72.6 %, max 73 %), GPU util 32 %
    avg. CPU is real headroom worth chasing.
  - Phase 3 synthetic gate (48 sinkpads, static-black sources to
    isolate mixer cost): `compositor` 53 % avg, `glvideomixer` 80 %
    avg — GL is **~50 % worse**, not better. Per-frame `glupload`
    overhead at 48 × 30 fps × 256 KB = ~370 MB/s of CPU→GPU upload
    bandwidth dominates; the saved CPU blend doesn't pay for it.
  - Vulkan compositing wasn't a usable alternative — GStreamer 1.28
    has no `vkvideomixer`/`vkcompositor`, and `vulkanoverlaycompositor`
    handles overlay-meta only, not multi-stream blending.
  - The path to a real win would be redesigning slot pumps to produce
    `GLMemory` directly (skipping the upload), which is a multi-day
    refactor of the cached-memory architecture in
    `crates/pipeline/src/assets/mod.rs`. Hard to justify when ML
    inference (smart blur, silhouette-aware emoji placement) is the
    actual unblocker for new features and adds its own GPU work that
    could amortise the upload cost.
  - `scripts/perf-cascade.sh` and the `perf-cascade` recipe survive
    as a regression-monitoring tool for the existing CPU pipeline,
    not as exploration infrastructure.
