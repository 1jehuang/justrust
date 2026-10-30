#!/usr/bin/env bash
# Run on hosted Linux with the preserved release compiler from 24d03213d.
# This is a scoped cache-state experiment, not a default-promotion benchmark.
# Observed warm aggregate was effectively flat, with substantial contention.
set -euo pipefail
repo=$(git rev-parse --show-toplevel)
root=${2:?existing private experiment directory required}
binary=$(realpath "${1:?preserved compiler binary required}")
mkdir -p "$root/bench" "$root/tests/fixtures" "$root/scratch"
git -C "$repo" show 24d03213d:bench/native_cache.py > "$root/bench/native_cache.py"
git -C "$repo" show 548be51c4:bench/suite.py > "$root/bench/suite.py"
cp -a "$repo/tests/fixtures/full_programs" "$root/tests/fixtures/"
cp "$repo/bench/results/20261030-llc-native-cache-observe.py" "$root/observe-locality.py"
python3 "$root/observe-locality.py" "$binary" llc
python3 "$root/bench/suite.py" compare "$root/locality-results/llc-off.json" "$root/locality-results/llc-warm.json"
