#!/bin/bash
set -u
root=/home/jeremy/.jcode/scratch/demand-query-cache-20261030
while [ ! -f "$root/acceptance.status" ]; do sleep 10; done
if [ "$(cat "$root/acceptance.status")" != 0 ]; then printf '1\n' > /home/jeremy/.jcode/scratch/ba30/runtime42.exit; exit 1; fi
cd /home/jeremy/justrust
python3 bench/blaze_runtime_compare.py \
 --source /home/jeremy/.jcode/scratch/ba30/source \
 --output /home/jeremy/.jcode/scratch/br42 \
 --blaze /home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze \
 --justrust /home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client \
 --rustc /home/jeremy/.jcode/scratch/demand-query-cache-20261030/sysroot/bin/rustc \
 --cargo /home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo \
 --blaze-llvm --demand-query-cache \
 --blaze-seed-cache /home/jeremy/.jcode/scratch/br30/blaze/cache/blaze \
 --samples 5 --test-filter summary::tests:: \
 > /home/jeremy/.jcode/scratch/ba30/runtime42.stdout \
 2> /home/jeremy/.jcode/scratch/ba30/runtime42.stderr
status=$?
printf '%s\n' "$status" > /home/jeremy/.jcode/scratch/ba30/runtime42.exit
exit "$status"
