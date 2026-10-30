#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
while ! test -f "$ROOT/candidate-unit-v17/exit"; do
 if test -f "$ROOT/candidate-build-v17/orchestration-exit" && test "$(cat "$ROOT/candidate-build-v17/orchestration-exit")" != 0; then printf '2\n' > "$ROOT/suite-v17.exit"; exit 2; fi
 sleep 3
done
exec bash "$ROOT/run-suite-v17.sh"
