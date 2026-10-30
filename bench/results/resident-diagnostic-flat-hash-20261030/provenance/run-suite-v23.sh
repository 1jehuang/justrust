#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
cd /home/jeremy/justrust
if ! test -f "$ROOT/candidate-unit-v23/exit" || test "$(cat "$ROOT/candidate-unit-v23/exit")" != 0; then printf '2\n' > "$ROOT/suite-v23.exit"; exit 2; fi
python3 "$ROOT/verify-native-unit-v23.py" > "$ROOT/native-unit-v23-admission.log" 2>&1
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
for pair in baseline:candidate-v22-bin:rsb14 candidate:candidate-v23-bin:rsa20; do
 IFS=: read -r label binary output <<< "$pair"
 set +e
 python3 bench/suite.py resident --reference-engine "$ROOT/reference-bin/ra-jit" --engine "$ROOT/$binary/jr-engine" --toolchain "$TC" --source /home/jeremy/.jcode/scratch/ba30/source --output "/home/jeremy/$output" --samples 5 --timeout 360 > "$ROOT/suite-v23-$label.log" 2>&1
 code=$?
 set -e
 printf '%s\n' "$code" > "$ROOT/suite-v23-$label.exit"
 if test "$code" != 0; then printf '%s\n' "$code" > "$ROOT/suite-v23.exit"; exit "$code"; fi
done
python3 bench/suite.py compare /home/jeremy/rsb14/suite.json /home/jeremy/rsa20/suite.json --floor 10 > "$ROOT/suite-v23-compare.log" 2>&1
printf '0\n' > "$ROOT/suite-v23.exit"
