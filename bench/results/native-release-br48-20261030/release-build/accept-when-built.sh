#!/usr/bin/env bash
set -uo pipefail
root=/home/jeremy/.jcode/scratch/native-release-frontier-20261030
while [ ! -f "$root/build-freeze.status" ]; do sleep 10; done
if [ "$(cat "$root/build-freeze.status")" != 0 ]; then
 printf '1\n' > "$root/acceptance.status"
 exit 1
fi
cd /home/jeremy/justrust
python3 bench/reuse-cgu-inline-accept.py --rustc "$root/sysroot/bin/rustc" --output "$root/acceptance" --require-mechanism > "$root/acceptance.log" 2>&1
rc=$?
printf '%s\n' "$rc" > "$root/acceptance.status"
exit "$rc"
