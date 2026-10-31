#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > /home/jeremy/.jcode/scratch/blaze-native-engine-20261030/block-scopes-v49-pairs.exit' EXIT
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v49-bin/jr-engine" --output "$ROOT/block-scopes-v49-candidate" --private /home/jeremy/q2 --cache-miss-stats 0
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v49-bin/jr-engine" --output "$ROOT/block-scopes-v49-candidate-repeat" --private /home/jeremy/q3 --cache-miss-stats 0
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v43-bin/jr-engine" --output "$ROOT/block-scopes-v49-baseline-repeat" --private /home/jeremy/q4 --cache-miss-stats 0
