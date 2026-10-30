#!/bin/bash
set -u
cd /home/jeremy/justrust
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/candidate-v2-bin/jr-engine" --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu --source /home/jeremy/.jcode/scratch/ba30/source --output /home/jeremy/rsb01 --samples 5 --timeout 360 > "$ROOT/suite-before-v3.log" 2>&1
code=$?
printf '%s\n' "$code" > "$ROOT/suite-before-v3.exit"
exit "$code"
