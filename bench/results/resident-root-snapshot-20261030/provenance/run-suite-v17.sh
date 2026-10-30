#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
cd /home/jeremy/justrust
if ! test -f "$ROOT/candidate-unit-v17/exit" || test "$(cat "$ROOT/candidate-unit-v17/exit")" != 0; then printf '2\n' > "$ROOT/suite-v17.exit"; exit 2; fi
python3 "$ROOT/verify-native-unit-v17.py" > "$ROOT/native-unit-v17-admission.log" 2>&1
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
for pair in baseline:candidate-v16-bin:rsb06 candidate:candidate-v17-bin:rsa12; do
 IFS=: read -r label binary output <<< "$pair"
 set +e
 python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/$binary/jr-engine" --toolchain "$TC" --source /home/jeremy/.jcode/scratch/ba30/source --output "/home/jeremy/$output" --samples 5 --timeout 360 > "$ROOT/suite-v17-$label.log" 2>&1
 code=$?
 set -e
 printf '%s\n' "$code" > "$ROOT/suite-v17-$label.exit"
 if test "$code" != 0; then printf '%s\n' "$code" > "$ROOT/suite-v17.exit"; exit "$code"; fi
done
python3 bench/suite.py compare /home/jeremy/rsb06/suite.json /home/jeremy/rsa12/suite.json --floor 10 > "$ROOT/suite-v17-compare.log" 2>&1
printf '0\n' > "$ROOT/suite-v17.exit"
