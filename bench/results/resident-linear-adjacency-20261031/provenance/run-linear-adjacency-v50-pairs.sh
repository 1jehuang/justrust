#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/linear-adjacency-v50-pairs.exit"' EXIT
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v50-bin/jr-engine" --output "$ROOT/linear-adjacency-v50-candidate" --private /home/jeremy/z2 --cache-miss-stats 0
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v50-bin/jr-engine" --output "$ROOT/linear-adjacency-v50-candidate-repeat" --private /home/jeremy/z3 --cache-miss-stats 0
python3 "$ROOT/probe-cache-miss-attribution-v42.py" --engine "$ROOT/candidate-v43-bin/jr-engine" --output "$ROOT/linear-adjacency-v50-baseline-repeat" --private /home/jeremy/z4 --cache-miss-stats 0
