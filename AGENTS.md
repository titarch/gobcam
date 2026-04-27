# AGENTS.md

Notes for contributors working on this repo.

## What this is

Gobcam is a Linux virtual webcam tool that adds animated emoji
reactions to a webcam feed via `v4l2loopback`. Two processes — a
GStreamer daemon (`crates/pipeline`) and a Tauri 2 + Svelte 5 panel
(`crates/ui`) — talking over a Unix socket with line-delimited JSON
(types in `crates/protocol`).

For the design rationale, read [`docs/architecture.md`](docs/architecture.md).
For the in-flight roadmap, [`docs/roadmap.md`](docs/roadmap.md).
For user-facing problems, [`docs/troubleshooting.md`](docs/troubleshooting.md).

## Development

```sh
just check     # fmt + clippy + tests + lint, the contributor gate
just ci        # the above + Docker build, run before pushing
just app       # build everything and launch the panel
```

`cargo-husky` installs a pre-commit hook on first `cargo test` that
runs `just check`. Don't bypass it without a reason.

The Rust toolchain is pinned in `rust-toolchain.toml` (currently
1.92). The same toolchain runs in local, Docker, and CI.

Lint posture is `clippy::pedantic` + `clippy::nursery` warned at
workspace level, denied via `-D warnings` in `just lint`. Allow-list
lives in `[workspace.lints.clippy]` in the root `Cargo.toml` and
should grow only when a lint is genuinely noisy.

## Load-bearing invariants

Touch these only with a reason and a regression test:

- **`firewall::install`** in the pipeline. Caps-query pad probe on
  `v4l2sink.sink` that works around an upstream thread-safety bug —
  removing it crashes the pipeline with `free(): invalid pointer`
  when the compositor has more than one sink pad. See
  [`docs/v4l2sink-thread-safety.md`](docs/v4l2sink-thread-safety.md).
- **`(ZERO, 0.0)` α-pre-key** in `effects::apply_cascade`. Without
  it, `InterpolationControlSource` returns "no value" for `t < start`
  and the manual `α=1` from `Slot::try_activate` flickers visible
  for 1-2 frames before the curve takes over.
- **Slot queue tuning** — `max-size-buffers=1`, `max-size-time=0`,
  no `leaky`. The default queue holds 1 s of buffers and was the
  source of a "1 second click-to-screen lag" for a long time. Don't
  add `leaky` here — backpressure is what stops the pump from
  CPU-spinning.
- **`WEBKIT_DISABLE_DMABUF_RENDERER=1`** for the UI on NVIDIA. Set
  in every launcher (`justfile`, `.desktop`, build container).
- **The `unsafe_code = "forbid"` workspace lint.** Means no
  `std::env::set_var` and friends in `main.rs` — environment must be
  set before the process starts.

## Operating notes

- Use `gst-launch-1.0` from the shell to validate pipeline topologies
  before encoding them in Rust. `just gst-passthrough` is the canned
  form for the trivial passthrough.
- Webcam capture format negotiation is finicky. `v4l2-ctl --list-formats-ext`
  (`just list-cam-formats`) shows what the device actually exposes; be
  explicit in caps filters.
- When a `v4l2loopback` device gets stuck after a failed run (locked
  in OUTPUT-only mode), `just reset-loopback` recovers it. With the
  `gobcam-setup` sudoers rule installed, this needs no password.
- Library-level bugs (GStreamer aggregator quirks, v4l2 thread safety,
  WebKit/NVIDIA interop) — web-search the exact error string first
  and build a minimal reproducer in `crates/pipeline/examples/` (the
  `repro_*.rs` family) before debugging in the main app. The
  `examples/` directory is the playground.

## Development cycle expected

implement → add a test (new feature) or regression test (bug fix) →
`just check` → if anything pipeline-touching, `just ci` →
commit. Update `docs/CHANGELOG.md` for user-visible changes.
