#!/bin/bash
set -u
cd /home/jeremy/justrust
python3 bench/blaze_edit_compare.py \
 --source /home/jeremy/.jcode/scratch/blaze-comparison-20261030/source \
 --output /home/jeremy/.jcode/scratch/blaze-comparison-20261030/matched-check \
 --blaze /home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze \
 --justrust /home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client \
 --rustc /home/jeremy/.jcode/scratch/provider-profile-20261030/sysroot/bin/rustc \
 --cargo /home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo \
 --command check --samples 3 \
 > /home/jeremy/.jcode/scratch/blaze-comparison-20261030/matched-launch.stdout \
 2> /home/jeremy/.jcode/scratch/blaze-comparison-20261030/matched-launch.stderr
status=$?
printf '%s\n' "$status" > /home/jeremy/.jcode/scratch/blaze-comparison-20261030/matched-launch.exit
exit "$status"
