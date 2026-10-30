#!/usr/bin/env bash
# Execute ON the hosted machine. SSH/upload time is not measured.
# Usage: bash THIS_SCRIPT PRESERVED_JR_RUST PRIVATE_SCRATCH NEW_RESULT_PREFIX [STOCK_RUSTC]
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 bench/suite.py native-cache \
  --binary "${1:?preserved compiler binary required}" \
  --scratch "${2:?existing private scratch directory required}" \
  --out-prefix "${3:?new output prefix required}" \
  --stock "${4:-/usr/local/bin/rustc}" -n 5 --warmup 1
