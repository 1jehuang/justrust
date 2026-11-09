#!/usr/bin/env bash
# Regenerate/verify stock-rustc oracle stdout for tests/milestones/m2 and m3.
# Single-file programs: <name>.rs -> <name>.stdout.
# Multi-file programs: <name>/src/main.rs -> <name>.stdout.
# Usage: scripts/milestones_oracle.sh [--write]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/ms-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
write=0
[ "${1:-}" = "--write" ] && write=1
status=0
echo "# $(rustc --version)"
for m in m2 m3; do
  dir="$root/tests/milestones/$m"
  for entry in "$dir"/*; do
    case "$entry" in
      *.rs) name="$(basename "$entry" .rs)"; src="$entry" ;;
      *.stdout) continue ;;
      *) [ -f "$entry/src/main.rs" ] || continue
         name="$(basename "$entry")"; src="$entry/src/main.rs" ;;
    esac
    exe="$work/$m-$name"
    if ! rustc --edition 2024 -C panic=abort -o "$exe" "$src" 2>"$exe.err"; then
      echo "RUSTC-FAIL $m/$name"; cat "$exe.err"; status=1; continue
    fi
    if [ -s "$exe.err" ]; then echo "WARN $m/$name"; cat "$exe.err"; status=1; fi
    set +e; "$exe" >"$exe.out"; code=$?; set -e
    if [ "$code" != 0 ]; then echo "EXIT $m/$name $code"; status=1; fi
    if [ "$write" = 1 ]; then
      cp "$exe.out" "$dir/$name.stdout"
    elif ! cmp -s "$exe.out" "$dir/$name.stdout"; then
      echo "MISMATCH $m/$name"; status=1
    fi
    echo "ok $m/$name $(wc -c <"$exe.out") bytes"
  done
done
exit $status
