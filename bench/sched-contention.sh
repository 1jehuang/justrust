#!/usr/bin/env bash
# Measure the machine-wide build scheduler (src/sched.rs): small edit->test
# loops while other "agents" run cold builds.
#
# Load: N cold `justrust check -p jcode-desktop-ui` builds of Jcode Desktop in
# fresh target dirs, every cache off, restarted until the probes finish.
# Probe: a one-line edit in a scratch copy of justrust, then `justrust test`.
# Each mode (sched on, JUSTRUST_SCHED=0) runs its own load and probes.
#
# usage: bench/sched-contention.sh [probes=6] [loads=2] [modes="on off"]
set -u
probes=${1:-6}
loads=${2:-2}
modes=${3:-"on off"}
desktop=${DESKTOP:-$HOME/jcode-desktop}
scratch=${SCRATCH:-${JCODE_SCRATCH_DIR:-/tmp}}/jr-sched-bench-$$
src=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$scratch"
trap 'kill $(jobs -p) 2>/dev/null; wait 2>/dev/null; rm -rf "$scratch"' EXIT

rsync -a --exclude target --exclude .git "$src/" "$scratch/probe/"
cd "$scratch/probe" || exit 1
cp src/runs.rs "$scratch/runs.rs.orig"
JUSTRUST_QUIET=1 justrust test >/dev/null 2>&1   # warm

load_loop() { # mode idx
    local sched=1; [ "$1" = off ] && sched=0
    while :; do
        local t="$scratch/load-$1-$2-$RANDOM"
        (cd "$desktop" && JUSTRUST_SCHED=$sched JUSTRUST_DEPCACHE=0 \
            JUSTRUST_BUILD_SCRIPT_CACHE=0 CARGO_TARGET_DIR="$t" JUSTRUST_QUIET=1 \
            justrust check -p jcode-desktop-ui >/dev/null 2>&1)
        rm -rf "$t"
    done
}

for mode in $modes; do
    sched=1; [ "$mode" = off ] && sched=0
    pids=()
    for l in $(seq 1 "$loads"); do load_loop "$mode" "$l" & pids+=($!); done
    sleep 45   # let the loads ramp up and burn CPU
    for i in $(seq 1 "$probes"); do
        python3 - src/runs.rs "$i" <<'EOF'
import sys
p, i = sys.argv[1], int(sys.argv[2])
s = open(p).read()
old = "d if d < 90.0 =>"
assert old in s, "edit anchor missing"
open(p, "w").write(s.replace(old, f"d if d < {90 + i}.0 =>", 1))
EOF
        s=$(date +%s.%N)
        out=$(JUSTRUST_SCHED=$sched justrust test 2>&1)
        e=$(date +%s.%N)
        cp "$scratch/runs.rs.orig" src/runs.rs
        id=$(grep -o 'justrust show [0-9-]*' <<<"$out" | head -1 | cut -d' ' -f3)
        printf '%s probe %d: %.1fs id=%s rustc=%s load=%s\n' "$mode" "$i" \
            "$(awk "BEGIN{print $e - $s}")" "$id" "$(pgrep -c rustc)" "$(cut -d' ' -f1 /proc/loadavg)"
        sleep 5
    done
    for p in "${pids[@]}"; do pkill -P "$p" 2>/dev/null; kill "$p" 2>/dev/null; done
    pkill -f "jr-sched-bench-$$/load" 2>/dev/null
    wait 2>/dev/null
    sleep 10
done
