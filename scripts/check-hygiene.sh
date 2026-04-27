#!/usr/bin/env bash
# Lightweight repo hygiene checks. Each subcommand exits non-zero on
# violation and prints offending paths/lines. Mirrors the high-value
# subset of `pre-commit-hooks` (the Python framework) without taking a
# Python dep.

set -uo pipefail

usage() {
  cat <<EOF
Usage: $0 <check>

Available checks:
  trailing-whitespace   non-empty trailing whitespace in tracked text files
  merge-conflicts       leftover <<<<<<<  / >>>>>>>  markers
  shebangs              executable-bit set, but no #! on first line
  eof-newline           tracked text files that don't end with \n
EOF
  exit 1
}

[ $# -eq 1 ] || usage

case "$1" in
  trailing-whitespace)
    # -I skips binary files
    if git grep -nIE ' +$'; then
      echo "(trailing whitespace above; fix or strip it)" >&2
      exit 1
    fi
    ;;
  merge-conflicts)
    # `<<<<<<< ` / `>>>>>>> ` (with trailing space, then a ref) are
    # unambiguous conflict markers — they don't occur in normal files.
    # Avoid matching `=======` alone since it's a markdown rule.
    if git grep -lE '^(<<<<<<< |>>>>>>> )'; then
      echo "(unresolved merge markers above)" >&2
      exit 1
    fi
    ;;
  shebangs)
    rc=0
    while IFS= read -r entry; do
      mode="${entry%% *}"
      path="${entry#* }"
      [ -f "$path" ] || continue
      if [ "$(head -c 2 "$path" 2>/dev/null)" != '#!' ]; then
        echo "$path: executable bit set but no #! shebang"
        rc=1
      fi
    done < <(git ls-files --stage | awk '$1 == "100755" { sub(/^[^\t]+\t/, ""); print "x " $0 }' | sed 's/^x //' | awk '{ print "100755 " $0 }')
    exit "$rc"
    ;;
  eof-newline)
    rc=0
    while IFS= read -r path; do
      [ -f "$path" ] || continue
      [ -s "$path" ] || continue
      # Skip binary files: git check-attr can't tell us, but `file --mime`
      # is portable enough.
      if file --mime --brief "$path" 2>/dev/null | grep -q 'charset=binary'; then
        continue
      fi
      last=$(tail -c 1 -- "$path" | od -An -tx1 | tr -d ' \n')
      if [ "$last" != "0a" ]; then
        echo "$path: missing trailing newline"
        rc=1
      fi
    done < <(git ls-files)
    exit "$rc"
    ;;
  *)
    usage
    ;;
esac
