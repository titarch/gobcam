# Contributing to Gobcam

The notes below are the short version of [`AGENTS.md`](AGENTS.md),
which has the repo map, load-bearing invariants, and operating notes.

## Setup

```sh
./scripts/setup-host.sh   # GStreamer plugins + v4l2loopback-dkms
just install-loopback     # /dev/video10, persistent across reboots
just setup                # cargo-husky pre-commit hook + just (if missing)
just app                  # build everything, launch the panel
```

The Rust toolchain is pinned in `rust-toolchain.toml`.

- `just check` — fmt + clippy + tests + frontend lint. The
  pre-commit hook installed by `cargo-husky` runs this on every
  commit and blocks if anything fails.
- `just ci` — the above plus the dev docker image build. Run
  before pushing.
- Hosted Actions run the same check gate on pushes and pull requests.

## Before you open a PR

- Add a test (or regression test for bug fixes). Pipeline-touching
  changes especially: the `crates/pipeline/examples/repro_*.rs`
  family is the playground for minimal reproducers — use it before
  debugging in the main app.
- Update [`docs/CHANGELOG.md`](docs/CHANGELOG.md) under `[Unreleased]`
  for user-visible changes.
- If you're touching anything called out in the
  [load-bearing invariants section](AGENTS.md#load-bearing-invariants)
  of AGENTS.md (the v4l2sink caps-query firewall, the slot
  queue tuning, etc.) — please link the regression test in your PR
  description.
- Run `just ci` locally before pushing anything pipeline-touching.

## Reporting bugs and proposing features

Use the issue templates under `.github/ISSUE_TEMPLATE/`. They ask for
the GStreamer version, kernel, distro, and camera setup, which is the
context most bugs need.

If you've already got a minimal reproducer, drop it under
`crates/pipeline/examples/repro_*.rs` in your branch and link it
from the issue.

## Conduct

Keep discussion technical and respectful. Assume people are here to
solve the problem, and move disagreements back to reproducible facts.
