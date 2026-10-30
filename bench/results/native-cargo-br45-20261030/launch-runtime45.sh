#!/bin/bash
set -u
cd /home/jeremy/justrust
python3 bench/blaze_runtime_compare.py \
 --source /home/jeremy/.jcode/scratch/ba30/source \
 --output /home/jeremy/.jcode/scratch/br45 \
 --blaze /home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze \
 --justrust /home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client \
 --rustc /home/jeremy/.jcode/scratch/demand-query-cache-20261030/sysroot/bin/rustc \
 --cargo /home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo \
 --native-cargo /home/jeremy/.jcode/scratch/native-cargo-frontier-20261030/jr-cargo \
 --blaze-llvm --blaze-seed-cache /home/jeremy/.jcode/scratch/br30/blaze/cache/blaze \
 --samples 5 --test-filter summary::tests:: \
 > /home/jeremy/.jcode/scratch/ba30/runtime45.stdout \
 2> /home/jeremy/.jcode/scratch/ba30/runtime45.stderr
status=$?
printf '%s\n' "$status" > /home/jeremy/.jcode/scratch/ba30/runtime45.exit
exit "$status"
