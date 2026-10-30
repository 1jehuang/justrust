#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
while ! test -f "$ROOT/candidate-unit-v5/exit"; do
 if test -f "$ROOT/candidate-build-v5/exit" && test "$(cat "$ROOT/candidate-build-v5/exit")" != 0; then printf '2\n' > "$ROOT/suite-after-v5.exit"; exit 2; fi
 sleep 3
done
if test "$(cat "$ROOT/candidate-unit-v5/exit")" != 0; then printf '2\n' > "$ROOT/suite-after-v5.exit"; exit 2; fi
cd /home/jeremy/justrust
set +e
python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/candidate-v5-bin/jr-engine" --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu --source /home/jeremy/.jcode/scratch/ba30/source --output /home/jeremy/rsa02 --samples 5 --timeout 360 > "$ROOT/suite-after-v5.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$ROOT/suite-after-v5.exit"
if test "$code" != 0; then exit "$code"; fi
python3 bench/suite.py compare /home/jeremy/rsa01/suite.json /home/jeremy/rsa02/suite.json --floor 10 > "$ROOT/suite-v4-v5-compare.log" 2>&1
