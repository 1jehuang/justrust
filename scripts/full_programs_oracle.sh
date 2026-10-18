#!/usr/bin/env bash
# Regenerate/verify stock-rustc oracle outputs for tests/fixtures/full_programs.
# Stock rustc is an external oracle only (edition 2024, -C panic=abort).
# Usage: scripts/full_programs_oracle.sh [--write]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/tests/fixtures/full_programs"
work="$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/fp-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
write=0
[ "${1:-}" = "--write" ] && write=1
status=0
echo "# $(rustc --version)"
for src in "$dir"/*.rs; do
  name="$(basename "$src" .rs)"
  if ! rustc --edition 2024 -C panic=abort -o "$work/$name" "$src" 2>"$work/$name.err"; then
    echo "RUSTC-FAIL $name"; cat "$work/$name.err"; status=1; continue
  fi
  if [ -s "$work/$name.err" ]; then echo "WARN $name"; cat "$work/$name.err"; status=1; fi
  set +e; "$work/$name" >"$work/$name.out"; code=$?; set -e
  if [ "$code" != 0 ]; then echo "EXIT $name $code"; status=1; fi
  if [ "$write" = 1 ]; then
    cp "$work/$name.out" "$dir/$name.stdout"
  elif ! cmp -s "$work/$name.out" "$dir/$name.stdout"; then
    echo "MISMATCH $name"; status=1
  fi
  echo "ok $name $(wc -c <"$work/$name.out") bytes"
done
exit $status
