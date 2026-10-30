#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
BUILD=$ROOT/candidate-build-v3
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
mkdir -p "$BUILD"
cd /home/jeremy/justrust/toolchain/resident
export PATH="$TC/bin:$PATH"
export JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$BUILD/justrust-home"
export JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc"
export CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
sha256sum crates/ra-jit/src/main.rs crates/ra-jit/src/jr_engine.rs crates/ra-jit/Cargo.toml crates/ra-jit/build.rs crates/ra-jit/src/cargo/mod.rs crates/ra-jit/src/cargo/whole.rs > "$BUILD/source-before.sha256"
set +e
justrust build --release --locked -p ra-jit --bin jr-engine -j 6 > "$BUILD/build.log" 2>&1
status=$?
set -e
printf '%s\n' "$status" > "$BUILD/compile-exit"
if test "$status" != 0; then printf '%s\n' "$status" > "$BUILD/exit"; exit "$status"; fi
sha256sum -c "$BUILD/source-before.sha256" > "$BUILD/source-after.log"
mkdir -p "$ROOT/candidate-v3-bin"
cp --reflink=auto "$CARGO_TARGET_DIR/release/jr-engine" "$ROOT/candidate-v3-bin/jr-engine"
"$ROOT/candidate-v3-bin/jr-engine" --version > "$BUILD/engine-version.txt"
sha256sum "$ROOT/candidate-v3-bin/jr-engine" > "$BUILD/engine-identity.sha256"
grep -Eq 'build resident-v1-[0-9a-f]{64}\)$' "$BUILD/engine-version.txt"
printf '0\n' > "$BUILD/exit"
