#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'printf "%s\n" "$?" > "$ROOT/final-v20.exit"' EXIT
while ! test -f "$ROOT/suite-v20.exit"; do sleep 3; done
test "$(cat "$ROOT/suite-v20.exit")" = 0
python3 "$ROOT/interleaved-v20.py" > "$ROOT/interleaved-v20.log" 2>&1
bash "$ROOT/full-native-v20.sh"
