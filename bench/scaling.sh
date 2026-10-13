#!/usr/bin/env bash
# Scaling curve: how far the toolchain uses cores. Runs ON a many-core build
# host (bench/on-remote.sh start -- bench/scaling.sh), not the laptop.
#
# For each core count N, the whole build (cargo, rustc, linker, tests) is
# pinned to the first N cores with taskset, CARGO_BUILD_JOBS=N, and the
# workspace frontend threads set to min(N, T). Every point uses the same
# box, the same warm target dir and the same pinned sources, so the curve is
# the toolchain's, not the hardware's.
#
#   bench/scaling.sh [CORES...]          default: 1 2 4 8 16 24 32 48 64 96
#   SCENARIOS="desktop-test-edit desktop-body-check" N=5 WARMUP=2 bench/scaling.sh
#
# Output: bench/results/scaling-<host>-<time>-c<N>.json per point plus a
# table on stdout (median wall per scenario per core count, and speedup vs 1).
set -euo pipefail
cd "$(dirname "$0")/.."
ncpu=$(nproc)
cores=("$@")
[[ ${#cores[@]} -gt 0 ]] || cores=(1 2 4 8 16 24 32 48 64 96)
SCENARIOS=${SCENARIOS:-desktop-test-edit desktop-body-check desktop-noop-test}
N=${N:-5} WARMUP=${WARMUP:-2}
TOOLCHAIN=${TOOLCHAIN:-nightly-2026-10-08}
stamp=$(date +%Y%m%d-%H%M%S)
host=$(hostname -s)
out=()
for c in "${cores[@]}"; do
  (( c <= ncpu )) || { echo "skip $c: only $ncpu cores" >&2; continue; }
  wt=$(( c < 16 ? c : 16 ))
  # suite.py accepts only these workspace-thread values.
  for v in 16 12 8 6 4 2 1; do (( wt >= v )) && { wt=$v; break; }; done
  f=bench/results/scaling-$host-$stamp-c$c.json
  echo "== $c cores (jobs $c, frontend threads $wt)" >&2
  CARGO_BUILD_JOBS=$c taskset -c 0-$((c - 1)) \
    python3 bench/suite.py run $SCENARIOS --toolchain "$TOOLCHAIN" \
      --workspace-threads "$wt" --warmup "$WARMUP" -n "$N" -o "$f" >&2
  out+=("$c:$f")
done
python3 - "${out[@]}" <<'EOF'
import json, statistics, sys
rows = {}
for arg in sys.argv[1:]:
    c, f = arg.split(":", 1)
    d = json.load(open(f))
    for name, sc in d.get("scenarios", {}).items():
        med = (sc.get("stats") or {}).get("median")
        if med:
            rows.setdefault(name, {})[int(c)] = med
for name, pts in rows.items():
    base = pts.get(min(pts))
    print(f"\n{name}")
    print(f"{'cores':>6} {'wall s':>8} {'speedup':>8}")
    for c in sorted(pts):
        print(f"{c:>6} {pts[c]:>8.2f} {base / pts[c]:>7.2f}x")
EOF
