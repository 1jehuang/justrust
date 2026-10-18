#!/usr/bin/env bash
# Regenerate/verify stock-rustc oracle results for tests/fixtures/scalar_patterns.
# Stock rustc is an external oracle only (edition 2024, -C panic=abort): accept/*.rs
# must compile without warnings, exit 0 and print <name>.stdout; reject/*.rs must fail.
# Usage: scripts/scalar_patterns_oracle.sh [--write]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/tests/fixtures/scalar_patterns"
work="$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/sp-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
write=0
[ "${1:-}" = "--write" ] && write=1
status=0
echo "# $(rustc --version)"
for src in "$dir"/accept/*.rs; do
  name="$(basename "$src" .rs)"
  if ! rustc --edition 2024 -C panic=abort -o "$work/$name" "$src" 2>"$work/$name.err"; then
    echo "RUSTC-FAIL $name"; cat "$work/$name.err"; status=1; continue
  fi
  # overlap_* fixtures intentionally contain dead arms; only that lint may fire.
  if [ -s "$work/$name.err" ]; then
    if [[ "$name" == overlap_* ]] && ! grep -v -e 'unreachable' -e '^ *|' -e '^ *-->' -e '^ *= note' -e '^$' -e '^[0-9 ]*|' -e 'warnings emitted' "$work/$name.err" | grep -q .; then
      echo "expected-unreachable $name"
    else
      echo "WARN $name"; cat "$work/$name.err"; status=1
    fi
  fi
  set +e; "$work/$name" >"$work/$name.out"; code=$?; set -e
  if [ "$code" != 0 ]; then echo "EXIT $name $code"; status=1; fi
  if [ "$write" = 1 ]; then
    cp "$work/$name.out" "$dir/accept/$name.stdout"
  elif ! cmp -s "$work/$name.out" "$dir/accept/$name.stdout"; then
    echo "MISMATCH $name"; status=1
  fi
  echo "accept $name"
done
for src in "$dir"/reject/*.rs; do
  name="$(basename "$src" .rs)"
  if rustc --edition 2024 -C panic=abort -o "$work/r-$name" "$src" 2>/dev/null; then
    echo "STOCK-ACCEPTS $name"; status=1
  else
    echo "reject $name"
  fi
done
exit $status
