#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/final-v22.exit"' EXIT
cmp "$ROOT/diagnostics-phases-v22/main.rs" /home/jeremy/justrust/toolchain/resident/crates/ra-jit/src/main.rs
bash "$ROOT/build-test-candidate-v22.sh"
bash "$ROOT/run-suite-v22.sh"
python3 "$ROOT/profile-diagnostics-v22.py" > "$ROOT/profile-diagnostics-v22.log" 2>&1
bash "$ROOT/full-native-v22.sh"
