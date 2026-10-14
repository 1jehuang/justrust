#!/usr/bin/env bash
# Which edits / flags keep the iface hash for a body-only edit.
# Usage: jr/iface-probe.sh <rustc> <workdir> <src.rs> <sed-anchor> [extra rustc args...]
# Builds <src.rs> twice, inserting `std::hint::black_box(N)` after the line
# matching <sed-anchor> (N = 1, 2), with --emit=metadata and metadata,link,
# prints both interface hashes and the first difference of the dumped streams.
set -euo pipefail
R=$1; W=$2; SRC=$3; ANCHOR=$4; shift 4
mkdir -p "$W"
for emit in metadata metadata,link; do
  for v in 1 2; do
    f="$W/src$v.rs"
    sed "s/^\(.*${ANCHOR}.*\)$/\1\n    std::hint::black_box(${v}u64);/" "$SRC" >"$f"
    o="$W/out-${emit/,/-}-$v"
    mkdir -p "$o"
    JUSTRUST_RUSTC_IFACE_DUMP="$o/dump" "$R" --edition 2024 --crate-type lib --crate-name a \
      --emit=dep-info,$emit -Zjr-iface-hash --out-dir "$o" "$@" "$f"
    echo "$emit $v: $(cat "$o/liba.rmeta.jriface")"
  done
  cmp "$W/out-${emit/,/-}-1/dump" "$W/out-${emit/,/-}-2/dump" || true
done
