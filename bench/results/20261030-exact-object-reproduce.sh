#!/bin/sh
# Run directly ON the hosted Linux x86-64 machine, outside other build activity.
# Prepare a workspace containing bench/{suite,native_cache}.py and the unchanged
# tests/fixtures/full_programs tree from 548be51c4. Use release binaries built
# from 548be51c4 (before) and e53250efa (after), same bootstrap and build flags.
# Usage: sh THIS_SCRIPT WORKSPACE BEFORE_BINARY AFTER_BINARY
# This is scoped native CLI evidence, not the comprehensive Cargo synthetic gate.
set -eu
workspace=$(realpath "$1")
before=$(realpath "$2")
after=$(realpath "$3")
test "$(sha256sum "$before" | cut -d ' ' -f 1)" = 6cf56e24cd878807656a96d87ec650ced327e8d6f4601376d637d28c6d34be02
test "$(sha256sum "$after" | cut -d ' ' -f 1)" = c2a11f2134ee9102b35e384c4eae065e396521e7f652a95f85085bd8d580b5d2
cp "$(dirname "$0")/20261030-exact-object-observe.py" "$workspace/observe.py"
mkdir -p "$workspace/scratch" "$workspace/results"
cd "$workspace"
python3 observe.py "$before" "$workspace/results/before"
python3 observe.py "$after" "$workspace/results/after"
python3 bench/suite.py compare --allow-native-binary-change results/before-warm.json results/after-warm.json
python3 bench/suite.py compare results/after-off.json results/after-warm.json
