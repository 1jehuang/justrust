#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
cd /home/jeremy/justrust
set +e
python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/candidate-v4-bin/jr-engine" --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu --source /home/jeremy/.jcode/scratch/ba30/source --output /home/jeremy/rsb03 --samples 5 --timeout 360 > "$ROOT/suite-v12-repeat-before.log" 2>&1
code=$?
printf '%s\n' "$code" > "$ROOT/suite-v12-repeat-before.exit"
if test "$code" != 0; then exit "$code"; fi
python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/candidate-v12-bin/jr-engine" --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu --source /home/jeremy/.jcode/scratch/ba30/source --output /home/jeremy/rsa09 --samples 5 --timeout 360 > "$ROOT/suite-v12-repeat-after.log" 2>&1
code=$?
printf '%s\n' "$code" > "$ROOT/suite-v12-repeat-after.exit"
if test "$code" != 0; then exit "$code"; fi
python3 bench/suite.py compare /home/jeremy/rsb03/suite.json /home/jeremy/rsa09/suite.json --floor 10 > "$ROOT/suite-v12-repeat-compare.log" 2>&1
code=$?
printf '%s\n' "$code" > "$ROOT/suite-v12-repeat.exit"
exit "$code"
