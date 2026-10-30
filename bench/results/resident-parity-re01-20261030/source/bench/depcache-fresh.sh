#!/usr/bin/env bash
# Fresh-target-dir check of Jcode Desktop, to measure the shared dependency
# cache on hit runs. Each run uses a brand-new CARGO_TARGET_DIR (removed
# afterwards), so every non-local unit is either a depcache hit or a compile.
#
# usage: bench/depcache-fresh.sh [runs=3] [package=jcode-desktop-ui]
# env:   DESKTOP=~/jcode-desktop  SCRATCH=$JCODE_SCRATCH_DIR or /tmp
set -u
runs=${1:-3}
pkg=${2:-jcode-desktop-ui}
desktop=${DESKTOP:-$HOME/jcode-desktop}
scratch=${SCRATCH:-${JCODE_SCRATCH_DIR:-/tmp}}
cd "$desktop" || exit 1
for i in $(seq 1 "$runs"); do
    t="$scratch/jr-depcache-bench-$$-$i"
    out="$scratch/jr-depcache-bench-$$-$i.log"
    others=$(pgrep -c rustc || true)
    load=$(cut -d' ' -f1 /proc/loadavg)
    s=$(date +%s.%N)
    CARGO_TARGET_DIR="$t" justrust check -p "$pkg" >"$out" 2>&1
    code=$?
    e=$(date +%s.%N)
    id=$(grep -o 'justrust show [0-9-]*' "$out" | head -1 | cut -d' ' -f3)
    report=$(justrust show "$id" 2>/dev/null)
    u="$HOME/.justrust/runs/$id/units.jsonl"
    dc="depcache $(grep -c '"depcache":"hit"' "$u") hit/$(grep -c '"depcache":"miss"' "$u") miss"
    gaps=$(grep -o 'build gaps *[0-9.]*s' <<<"$report" | head -1)
    printf 'run %d: %.1fs exit=%d id=%s %s, %s (rustc before: %s, load %s)\n' \
        "$i" "$(awk "BEGIN{print $e - $s}")" "$code" "$id" "$dc" "$gaps" "$others" "$load"
    rm -rf "$t" "$out"
done
