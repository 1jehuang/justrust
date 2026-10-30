#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
while ! test -f "$ROOT/suite-v21.exit"; do sleep 3; done
if test "$(cat "$ROOT/suite-v21.exit")" != 0; then printf '2\n' > "$ROOT/candidate-unit-v21/full-native.exit"; exit 2; fi
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
cd /home/jeremy/justrust/toolchain/resident
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$ROOT/candidate-unit-v21/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
set +e
justrust test --release --locked -p ra-jit --bin jr-engine -j 6 > "$ROOT/candidate-unit-v21/full-native.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$ROOT/candidate-unit-v21/full-native.exit"
sha256sum -c "$ROOT/candidate-build-v21/source-before.sha256" > "$ROOT/candidate-unit-v21/full-native-source-after.log"
exit "$code"
