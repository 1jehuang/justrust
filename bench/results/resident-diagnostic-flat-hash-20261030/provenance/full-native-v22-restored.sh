#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
OUT="$ROOT/candidate-unit-v22-restored"
mkdir -p "$OUT"
cd /home/jeremy/justrust/toolchain/resident
test ! -e crates/ra-jit/src/diagnostic_hash.rs
sha256sum -c "$ROOT/candidate-build-v22/source-before.sha256" > "$OUT/source-before.log"
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$OUT/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
set +e
justrust test --release --locked -p ra-jit --bin jr-engine -j 6 > "$OUT/full-native.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$OUT/full-native.exit"
test ! -e crates/ra-jit/src/diagnostic_hash.rs
sha256sum -c "$ROOT/candidate-build-v22/source-before.sha256" > "$OUT/source-after.log"
exit "$code"
