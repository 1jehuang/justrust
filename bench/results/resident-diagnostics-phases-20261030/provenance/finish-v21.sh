#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/final-v21.exit"' EXIT
cmp "$ROOT/diagnostics-phases-v21/main-v21b.rs" /home/jeremy/justrust/toolchain/resident/crates/ra-jit/src/main.rs
bash "$ROOT/build-test-candidate-v21.sh"
bash "$ROOT/run-suite-v21.sh"
python3 "$ROOT/profile-diagnostics-v21.py" > "$ROOT/profile-diagnostics-v21.log" 2>&1
bash "$ROOT/full-native-v21.sh"
