#!/usr/bin/env bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
mkdir -p "$ROOT/baseline-build"
cd "$ROOT/source"
export PATH="$TC/bin:$PATH"
export JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_PROCMACRO_CACHE=0
export JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off
export JUSTRUST_HOME="$ROOT/baseline-build/justrust-home"
export JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc"
export RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/baseline-build/target"
export RA_JIT_GIT_SHA=5d4f78f7577cf7170993b74f735099dd03f5c6ca
# Controlled local baseline: this engine requires the pinned matching Rust toolchain.
# It must not inherit the shipping jr-rustc or a different hosted toolchain.
"$TC/bin/rustc" -vV > "$ROOT/baseline-build/rustc-version.txt"
sha256sum "$TC/bin/rustc" "$TC/bin/cargo" > "$ROOT/baseline-build/tool-identities.sha256"
set +e
justrust build --release --locked -p ra-jit --bin ra-jit -j 6 > "$ROOT/baseline-build/build.log" 2>&1
status=$?
printf '%s\n' "$status" > "$ROOT/baseline-build/exit"
exit "$status"
