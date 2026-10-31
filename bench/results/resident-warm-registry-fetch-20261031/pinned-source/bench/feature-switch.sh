#!/usr/bin/env bash
# Measure dependency rebuilds when an agent switches between workspace packages.
#
# Usage: bench/feature-switch.sh <repo> <pkg>... 
# Runs `justrust check -p <pkg>` for each package in a fresh target dir, twice
# through the list, with workspace feature unification on and then off, and
# prints compiled units and registry dependencies per step. Uses its own
# depcache dir so the shared cache does not hide rebuilds.
set -euo pipefail
repo="$1"; shift
J="${JUSTRUST:-justrust}"
scratch="${JCODE_SCRATCH_DIR:-$HOME/.jcode/scratch}/feature-switch-$$"
mkdir -p "$scratch"
trap 'rm -rf "$scratch"' EXIT
cd "$repo"
for mode in on off; do
  unify=1; [ "$mode" = off ] && unify=0
  echo "=== unify=$mode"
  total=0
  for pass in 1 2; do
    for p in "$@"; do
      out=$(JUSTRUST_UNIFY_FEATURES=$unify JUSTRUST_SLOTS=0 JUSTRUST_REMOTE=0 \
        JUSTRUST_DEPCACHE_DIR="$scratch/cache-$mode" CARGO_TARGET_DIR="$scratch/t-$mode" \
        JUSTRUST_REPORT=brief "$J" check -p "$p" 2>&1 | grep -o 'justrust show [0-9-]*' | tail -1 | cut -d' ' -f3)
      python3 - "$out" "$pass" "$p" <<'EOF'
import json, os, sys
rid, pss, pkg = sys.argv[1:]
s = json.load(open(os.path.expanduser(f"~/.justrust/runs/{rid}/summary.json")))
u = s["units"]
print(f"  pass {pss} {pkg:28s} {s['wall']:6.1f}s  compiled {u['compiled']:4d}  deps {u['dependencies']:4d}")
EOF
    done
  done
done
