#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
cd /home/jeremy/justrust
trap 'printf "%s\n" "$?" > "$ROOT/suite-v17-cumulative.exit"' EXIT
test "$(cat "$ROOT/candidate-unit-v17/full-native.exit")" = 0
python3 "$ROOT/verify-native-unit-v17.py" > "$ROOT/native-unit-v17-cumulative-admission.log" 2>&1
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
for pair in baseline:candidate-v4-bin:rsb07 candidate:candidate-v17-bin:rsa13; do
 IFS=: read -r label binary output <<< "$pair"
 python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/$binary/jr-engine" --toolchain "$TC" --source /home/jeremy/.jcode/scratch/ba30/source --output "/home/jeremy/$output" --samples 5 --timeout 360 > "$ROOT/suite-v17-cumulative-$label.log" 2>&1
 printf '0\n' > "$ROOT/suite-v17-cumulative-$label.exit"
done
python3 bench/suite.py compare /home/jeremy/rsb07/suite.json /home/jeremy/rsa13/suite.json --floor 10 > "$ROOT/suite-v17-cumulative-compare.log" 2>&1
