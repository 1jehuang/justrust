#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
UNIT="$ROOT/candidate-unit-v24-snapshot"
mkdir -p "$UNIT"
trap 'code=$?; printf "%s\n" "$code" > "$UNIT/orchestration-exit"' EXIT
cd /home/jeremy/justrust/toolchain/resident
sha256sum -c "$ROOT/candidate-build-v24/source-before.sha256" > "$UNIT/source-before.log"
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$UNIT/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
set +e
justrust test --manifest-path "$UNIT/vendor/rust-analyzer/Cargo.toml" --release --offline -p ide-diagnostics --lib -j 6 -- resident_selective_traversal --nocapture > "$UNIT/resident_selective_traversal.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$UNIT/resident_selective_traversal.exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$UNIT/exit"; exit "$code"; fi
for filter in diagnostic_hash::tests diagnostics_timing_tests resident_graph_reuse the_roots_of_a_listed_crate telemetry; do
 set +e
 justrust test --release --locked -p ra-jit --bin jr-engine -j 6 -- "$filter" --nocapture > "$UNIT/$filter.log" 2>&1
 code=$?
 set -e
 printf '%s\n' "$code" > "$UNIT/$filter.exit"
 if test "$code" != 0; then printf '%s\n' "$code" > "$UNIT/exit"; exit "$code"; fi
done
sha256sum -c "$ROOT/candidate-build-v24/source-before.sha256" > "$UNIT/source-after.log"
printf '0\n' > "$UNIT/exit"
