#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
BUILD=$ROOT/candidate-unit-v3
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
mkdir -p "$BUILD"
while ! test -f "$ROOT/candidate-build-v3/exit"; do sleep 2; done
if test "$(cat "$ROOT/candidate-build-v3/exit")" != 0; then printf '2\n' > "$BUILD/exit"; exit 2; fi
cd /home/jeremy/justrust/toolchain/resident
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$BUILD/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
set +e
justrust test --release --locked -p ra-jit --bin jr-engine -j 6 -- resident_graph_reuse --nocapture > "$BUILD/graph.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$BUILD/graph-exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$BUILD/exit"; exit "$code"; fi
set +e
justrust test --release --locked -p ra-jit --bin jr-engine -j 6 -- the_roots_of_a_listed_crate --nocapture > "$BUILD/listed.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$BUILD/listed-exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$BUILD/exit"; exit "$code"; fi
set +e
justrust test --release --locked -p ra-jit --bin jr-engine -j 6 -- telemetry_tests --nocapture > "$BUILD/telemetry.log" 2>&1
code=$?
printf '%s\n' "$code" > "$BUILD/exit"
exit "$code"
