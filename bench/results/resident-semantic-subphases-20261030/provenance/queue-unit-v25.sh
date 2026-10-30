#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
while ! test -f "$ROOT/candidate-build-v25/orchestration-exit"; do sleep 3; done
test "$(cat "$ROOT/candidate-build-v25/orchestration-exit")" = 0
bash "$ROOT/native-unit-v25-snapshot.sh"
