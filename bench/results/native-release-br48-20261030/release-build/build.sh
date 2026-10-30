#!/usr/bin/env bash
set -uo pipefail
root=/home/jeremy/.jcode/scratch/native-release-frontier-20261030
trap 'rc=$?; printf "%s\n" "$rc" > "$root/build-freeze.status"' EXIT
while [ ! -f /home/jeremy/.jcode/scratch/ba30/profile47.exit ]; do sleep 10; done
test "$(cat /home/jeremy/.jcode/scratch/ba30/profile47.exit)" = 0 || exit 1
cd /home/jeremy/justrust/toolchain/rustc
sha256sum compiler/rustc_monomorphize/src/partitioning.rs compiler/rustc_session/src/options.rs > "$root/source-before.sha256"
./x --config "$root/bootstrap-release.toml" build --stage 1 --jobs 8 compiler/rustc library > "$root/build.log" 2>&1
rc=$?
printf '%s\n' "$rc" > "$root/build.status"
[ "$rc" = 0 ] || exit "$rc"
sha256sum -c "$root/source-before.sha256" > "$root/source-after.log" || exit 1
mkdir -m 700 "$root/sysroot"
rsync -aL --exclude=/lib/rustlib/src --exclude=/lib/rustlib/rustc-src build/host/stage1/ "$root/sysroot/" > "$root/freeze.log" 2>&1 || exit 1
sha256sum "$root/sysroot/bin/rustc" "$root/sysroot"/lib/librustc_driver-*.so > "$root/identity-before.sha256"
"$root/sysroot/bin/rustc" -vV > "$root/compiler-version.txt"
