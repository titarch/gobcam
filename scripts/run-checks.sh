#!/usr/bin/env bash
# Pretty-printed pre-commit gate.
#
# Runs every check sequentially, captures stdout+stderr per step, and
# prints one Passed/Failed line per step. On failure the captured
# output is dumped beneath the failure line, and the script exits 1
# after running every check (so a single commit attempt surfaces all
# the things that need fixing).
#
# Set `CHECK_VERBOSE=1` to bypass capture and stream output live —
# useful when a check hangs or you need to see progress.

set -uo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

# label | command
checks=(
  "merge conflicts   |scripts/check-hygiene.sh merge-conflicts"
  "trailing space    |scripts/check-hygiene.sh trailing-whitespace"
  "EOF newline       |scripts/check-hygiene.sh eof-newline"
  "exec shebangs     |scripts/check-hygiene.sh shebangs"
  "rustfmt           |cargo fmt --all -- --check"
  "clippy            |cargo clippy --workspace --lib --bins --tests --all-features -- -D warnings"
  "cargo test        |cargo test --workspace --all-features"
  "pnpm install      |pnpm -C crates/ui install --frozen-lockfile"
  "biome             |pnpm -C crates/ui run lint"
  "svelte-check      |pnpm -C crates/ui run check-types"
  "vitest            |pnpm -C crates/ui run test"
)

if [ -t 1 ]; then
  green=$'\033[32m'
  red=$'\033[31m'
  dim=$'\033[2m'
  reset=$'\033[0m'
else
  green=""
  red=""
  dim=""
  reset=""
fi

row_width=50
verbose="${CHECK_VERBOSE:-0}"
failures=()

print_row() {
  local label="$1"
  local status="$2"
  local color="$3"
  local pad=$((row_width - ${#label}))
  if [ "$pad" -lt 3 ]; then pad=3; fi
  printf '%s ' "$label"
  printf '%*s' "$((pad - 1))" '' | tr ' ' '.'
  printf ' %s%s%s\n' "$color" "$status" "$reset"
}

for check in "${checks[@]}"; do
  label="${check%%|*}"
  label="${label%"${label##*[![:space:]]}"}"
  cmd="${check#*|}"

  if [ "$verbose" = "1" ]; then
    echo "${dim}\$ $cmd${reset}"
    if eval "$cmd"; then
      print_row "$label" "Passed" "$green"
    else
      print_row "$label" "Failed" "$red"
      failures+=("$label")
    fi
    continue
  fi

  tmp=$(mktemp)
  if eval "$cmd" >"$tmp" 2>&1; then
    print_row "$label" "Passed" "$green"
    rm -f "$tmp"
  else
    print_row "$label" "Failed" "$red"
    echo "${dim}--- $label output ---${reset}"
    cat "$tmp"
    echo "${dim}--- end $label ---${reset}"
    rm -f "$tmp"
    failures+=("$label")
  fi
done

if [ "${#failures[@]}" -gt 0 ]; then
  echo
  echo "${red}check failed${reset}: ${failures[*]}"
  exit 1
fi
