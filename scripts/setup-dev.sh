#!/usr/bin/env bash
# One-time developer environment bootstrap.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if ! command -v just >/dev/null 2>&1; then
  echo "[setup-dev] installing 'just' via cargo..."
  cargo install just --locked
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "[setup-dev] cargo missing — install rustup first (https://rustup.rs)" >&2
  exit 1
fi

echo "[setup-dev] wiring cargo-husky pre-commit hook (runs 'just check' on commit)..."
cargo test --workspace --no-run --quiet >/dev/null

echo "[setup-dev] done. Run 'just --list' to see available recipes."
