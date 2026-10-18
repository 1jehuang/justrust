#!/usr/bin/env bash
# Record the stock-rustc oracle for tests/fixtures/trait_objects (edition 2024,
# -C panic=abort). Writes stock.tsv: path, stock status, first error code, and
# the stdout file for accepted programs. Stock is an oracle only.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/tests/fixtures/trait_objects"
work="$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/to-oracle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
out="$dir/stock.tsv"
{
  echo "# $(rustc --version) --edition 2024 -C panic=abort"
  echo "# path	stock	code"
  for src in "$dir"/positive/*.rs "$dir"/negative/*.rs "$dir"/boundary/*.rs; do
    rel="${src#$dir/}"
    if rustc --edition 2024 -C panic=abort -o "$work/bin" "$src" 2>"$work/err"; then
      if grep -q '^warning' "$work/err"; then echo "warning in $rel" >&2; exit 1; fi
      "$work/bin" >"${src%.rs}.stdout"
      echo "$rel	accept	-"
    else
      code=$(grep -oE '^error\[E[0-9]+\]' "$work/err" | head -1 | grep -oE 'E[0-9]+' || echo "-")
      echo "$rel	reject	$code"
    fi
  done
} >"$out.tmp"
mv "$out.tmp" "$out"
cat "$out"
