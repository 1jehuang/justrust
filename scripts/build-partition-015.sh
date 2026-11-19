#!/usr/bin/env bash
# Isolated measurement only. Never installs or publishes a toolchain.
set -euo pipefail
if [[ $# != 3 ]]; then
  echo 'usage: build-partition-015.sh FROZEN_SOURCE.tar.zst PUBLISHED_CONTROL.tar.gz NEW_OUTPUT_DIRECTORY' >&2
  exit 2
fi
source_archive=$(realpath "$1")
control_archive=$(realpath "$2")
out=$(realpath -m "$3")
rev=78c729d823524f32ee460d89d6f6e114e7fc829d
check() { printf '%s  %s\n' "$1" "$2" | sha256sum --check --strict; }
# Reproduce: git archive "$rev" | zstd -T2 -3 > source.tar.zst
check 05df6dae251103b9df829ac1ff860c0a90232bf0bf4eb42f3284578fdb9bb94a "$source_archive"
check 72b06f83325a7305d31af4e4714d8bd9e15cf63eb980ca677288c9ea6c5bad3a "$control_archive"
mkdir "$out"
cd "$out"
exec > >(tee build.log) 2>&1
set -x
mkdir source control
tar --zstd -xf "$source_archive" -C source
tar -xzf "$control_archive" -C control
check 994419eb0d12f1a4152a23d9030dc4d05c9bfd7a5c7af923f24bb3738afad95b control/bin/cargo
cd source/toolchain/rustc
printf '%s\n' "$rev" "${rev:0:9}" 2026-11-19 > git-commit-info
printf '%s\n' "$rev" > justrust-release-archive
export JR_STAGE=1 JR_TOOLS=1 CI=1 GIT_TERMINAL_PROMPT=0
bash build-jr.sh pgo-gen
bash pgo-train.sh
bash build-jr.sh pgo-bolt-base
bash bolt.sh
prefix=$PWD/build/jr/pgo-bolt
cp --remove-destination "$out/control/bin/cargo" "$prefix/bin/cargo"
check 994419eb0d12f1a4152a23d9030dc4d05c9bfd7a5c7af923f24bb3738afad95b "$prefix/bin/cargo"
"$prefix/bin/rustc" -vV | tee "$out/rustc-version.txt"
grep -Fx "commit-hash: $rev" "$out/rustc-version.txt"
sha256sum "$prefix/bin/"{rustc,cargo} "$prefix/lib/"librustc_driver*.so > "$out/components.sha256"
