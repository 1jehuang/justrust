#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
cd /home/jeremy/justrust
trap 'printf "%s\n" "$?" > "$ROOT/suite-v19-reverse.exit"' EXIT
test "$(cat "$ROOT/candidate-unit-v19/full-native.exit")" = 0
python3 "$ROOT/verify-native-unit-v19.py" > "$ROOT/native-unit-v19-reverse-admission.log" 2>&1
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
for pair in candidate:candidate-v19-bin:rsa16 baseline:candidate-v4-bin:rsb10; do
 IFS=: read -r label binary output <<< "$pair"
 python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/$binary/jr-engine" --toolchain "$TC" --source /home/jeremy/.jcode/scratch/ba30/source --output "/home/jeremy/$output" --samples 5 --timeout 360 > "$ROOT/suite-v19-reverse-$label.log" 2>&1
 printf '0\n' > "$ROOT/suite-v19-reverse-$label.exit"
done
python3 bench/suite.py compare /home/jeremy/rsb10/suite.json /home/jeremy/rsa16/suite.json --floor 10 > "$ROOT/suite-v19-reverse-compare.log" 2>&1
