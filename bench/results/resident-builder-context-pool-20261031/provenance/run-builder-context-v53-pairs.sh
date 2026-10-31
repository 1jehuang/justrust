#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/builder-context-v53-pairs.exit"' EXIT
python3 "$ROOT/probe-builder-context-v52.py" --engine "$ROOT/candidate-v53-bin/jr-engine" --output "$ROOT/builder-context-v53-off" --private /home/jeremy/bp1 --cache-miss-stats 0 --builder-pool 0
python3 "$ROOT/probe-builder-context-v52.py" --engine "$ROOT/candidate-v53-bin/jr-engine" --output "$ROOT/builder-context-v53-on" --private /home/jeremy/bp2 --cache-miss-stats 0 --builder-pool 1
python3 "$ROOT/probe-builder-context-v52.py" --engine "$ROOT/candidate-v53-bin/jr-engine" --output "$ROOT/builder-context-v53-on-repeat" --private /home/jeremy/bp3 --cache-miss-stats 0 --builder-pool 1
python3 "$ROOT/probe-builder-context-v52.py" --engine "$ROOT/candidate-v53-bin/jr-engine" --output "$ROOT/builder-context-v53-off-repeat" --private /home/jeremy/bp4 --cache-miss-stats 0 --builder-pool 0
