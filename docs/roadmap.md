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
- **GPU pipeline.** `compositor` → `glvideomixer`. Compositor blend is
  the dominant per-frame cost at N=48; if a user's CPU can't keep up,
  this is the next lever. The v4l2 boundaries force CPU-side memory
  on either side of the blend, so naive GPU offload often costs more
  than it saves — needs profiling first.
- **More CI coverage.** The shared GitHub/Gitea Actions gate mirrors
  `just check`. Next useful additions would be package smoke tests and
  a release dry-run before tags.

## Probably not

- **Plugin / external effect ABI.** Each effect is a Rust module today;
  promoting to a real ABI is only worth it once 3-4 effects exist and
  a real extension pattern has emerged.
- **macOS / Windows.** v4l2loopback is Linux-specific, so anything
  cross-platform would need a fundamentally different output mechanism.
