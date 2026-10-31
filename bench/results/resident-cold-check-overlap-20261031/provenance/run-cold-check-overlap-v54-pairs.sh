#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
trap 'code=$?; printf "%s\n" "$code" > "$ROOT/cold-check-overlap-v54-pairs.exit"' EXIT
python3 "$ROOT/probe-cold-check-overlap-v54.py" --engine "$ROOT/candidate-v54-bin/jr-engine" --output "$ROOT/cold-check-overlap-v54-off" --private /home/jeremy/co1 --cache-miss-stats 0 --cold-check-overlap 0
python3 "$ROOT/probe-cold-check-overlap-v54.py" --engine "$ROOT/candidate-v54-bin/jr-engine" --output "$ROOT/cold-check-overlap-v54-on" --private /home/jeremy/co2 --cache-miss-stats 0 --cold-check-overlap 1
python3 "$ROOT/probe-cold-check-overlap-v54.py" --engine "$ROOT/candidate-v54-bin/jr-engine" --output "$ROOT/cold-check-overlap-v54-on-repeat" --private /home/jeremy/co3 --cache-miss-stats 0 --cold-check-overlap 1
python3 "$ROOT/probe-cold-check-overlap-v54.py" --engine "$ROOT/candidate-v54-bin/jr-engine" --output "$ROOT/cold-check-overlap-v54-off-repeat" --private /home/jeremy/co4 --cache-miss-stats 0 --cold-check-overlap 0
