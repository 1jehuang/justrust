#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
cd /home/jeremy/justrust
test "$(cat "$ROOT/suite-v28-candidate.exit")" = 0
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/suite-v28-repeat.exit"' EXIT
for pair in candidate:candidate-v28-bin:rsa25 baseline:candidate-v26-bin:rsb19; do
 IFS=: read -r label binary output <<< "$pair"
 set +e
 python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/$binary/jr-engine" --toolchain "$TC" --source /home/jeremy/.jcode/scratch/ba30/source --output "/home/jeremy/$output" --samples 5 --timeout 360 > "$ROOT/suite-v28-repeat-$label.log" 2>&1
 code=$?
 set -e
 printf '%s\n' "$code" > "$ROOT/suite-v28-repeat-$label.exit"
 if test "$code" != 0; then exit "$code"; fi
done
python3 bench/suite.py compare /home/jeremy/rsb19/suite.json /home/jeremy/rsa25/suite.json --floor 10 > "$ROOT/suite-v28-repeat-compare.log" 2>&1
cat "$ROOT/suite-v28-repeat-compare.log"
