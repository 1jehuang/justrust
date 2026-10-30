#!/bin/bash
set -u
cd /home/jeremy/justrust
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
python3 bench/resident_engine_compare.py --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/candidate-v2-bin/jr-engine" --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu --source /home/jeremy/.jcode/scratch/ba30/source --output /home/jeremy/re01 --samples 5 --timeout 360 > "$ROOT/runtime-v2.log" 2>&1
code=$?
printf '%s\n' "$code" > "$ROOT/runtime-v2.exit"
exit "$code"
