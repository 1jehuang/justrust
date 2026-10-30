#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
while ! test -f "$ROOT/candidate-unit-v25-snapshot/orchestration-exit"; do sleep 3; done
test "$(cat "$ROOT/candidate-unit-v25-snapshot/orchestration-exit")" = 0
bash "$ROOT/run-suite-v25-snapshot.sh"
