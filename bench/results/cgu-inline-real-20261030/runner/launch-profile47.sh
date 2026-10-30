#!/bin/bash
set -u
cd /home/jeremy/justrust
python3 /home/jeremy/.jcode/scratch/ba30/profile-cgu47.py > /home/jeremy/.jcode/scratch/ba30/profile47.stdout 2> /home/jeremy/.jcode/scratch/ba30/profile47.stderr
rc=$?
printf '%s\n' "$rc" > /home/jeremy/.jcode/scratch/ba30/profile47.exit
exit "$rc"
