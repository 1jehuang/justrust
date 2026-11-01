#!/usr/bin/env bash
# Profile the edit -> test loop on this repo with a real one-line code edit.
#
# Usage: bench/edit-loop.sh [runs]
# Applies a tiny body edit to a source file, runs `justrust test` with rustc
# pass timings, restores the file from a backup (never via git), and repeats.
# Prints each run's breakdown. Requires justrust on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."
runs="${1:-3}"
file=src/runs.rs
backup="$(mktemp)"
cp "$file" "$backup"
trap 'cp "$backup" "$file"; rm -f "$backup"' EXIT

# Warm: make sure the pass-timing mode is already in the incremental cache.
JUSTRUST_PASSES=always JUSTRUST_QUIET=1 justrust test >/dev/null 2>&1 || true

for i in $(seq 1 "$runs"); do
  # Toggle a constant inside a function body: changes codegen, not signatures.
  python3 - "$file" "$i" <<'EOF'
import sys
p, i = sys.argv[1], int(sys.argv[2])
s = open(p).read()
old = "d if d < 90.0 =>"
assert old in s, "edit anchor missing"
open(p, "w").write(s.replace(old, f"d if d < {90 + i}.0 =>", 1))
EOF
  JUSTRUST_PASSES=always justrust test >/dev/null 2>&1
  cp "$backup" "$file"
  echo "=== run $i"
  justrust show | sed -n '/^time/p;/Where/,/^$/p;/UNIT/,/^$/p;/Slowest/,/^$/p'
done
