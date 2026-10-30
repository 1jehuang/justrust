#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
BUILD=$ROOT/candidate-build-v7
UNIT=$ROOT/candidate-unit-v7
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
mkdir -p "$BUILD" "$UNIT"
cd /home/jeremy/justrust/toolchain/resident
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$BUILD/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
sha256sum crates/ra-jit/src/main.rs crates/ra-jit/src/jr_engine.rs crates/ra-jit/Cargo.toml crates/ra-jit/build.rs crates/ra-jit/src/cargo/mod.rs crates/ra-jit/src/cargo/whole.rs crates/ra-jit/src/cargo/cli.rs crates/ra-jit/src/cargo/metadata.rs crates/blaze-metadata/src/lib.rs crates/blaze-metadata/tests/resident_typed_planning.rs > "$BUILD/source-before.sha256"
set +e
justrust check --release --locked -p blaze-metadata --tests -j 6 > "$BUILD/check.log" 2>&1
code=$?
printf '%s\n' "$code" > "$BUILD/check-exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$BUILD/exit"; exit "$code"; fi
justrust test --release --locked -p blaze-metadata --test resident_typed_planning -j 6 -- typed_programs --nocapture > "$UNIT/typed_programs.log" 2>&1
code=$?
printf '%s\n' "$code" > "$UNIT/typed_programs.exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$BUILD/exit"; printf '%s\n' "$code" > "$UNIT/exit"; exit "$code"; fi
justrust build --release --locked -p ra-jit --bin jr-engine -j 6 > "$BUILD/build.log" 2>&1
code=$?
set -e
printf '%s\n' "$code" > "$BUILD/compile-exit"
if test "$code" != 0; then printf '%s\n' "$code" > "$BUILD/exit"; exit "$code"; fi
sha256sum -c "$BUILD/source-before.sha256" > "$BUILD/source-after.log"
mkdir -p "$ROOT/candidate-v7-bin"
cp --reflink=auto "$CARGO_TARGET_DIR/release/jr-engine" "$ROOT/candidate-v7-bin/jr-engine"
"$ROOT/candidate-v7-bin/jr-engine" --version > "$BUILD/engine-version.txt"
sha256sum "$ROOT/candidate-v7-bin/jr-engine" > "$BUILD/engine-identity.sha256"
grep -Eq 'build resident-v1-[0-9a-f]{64}\)$' "$BUILD/engine-version.txt"
printf '0\n' > "$BUILD/exit"
export JUSTRUST_HOME="$UNIT/justrust-home"
for filter in resident_graph_reuse the_roots_of_a_listed_crate telemetry; do
 set +e
 justrust test --release --locked -p ra-jit --bin jr-engine -j 6 -- "$filter" --nocapture > "$UNIT/$filter.log" 2>&1
 code=$?
 set -e
 printf '%s\n' "$code" > "$UNIT/$filter.exit"
 if test "$code" != 0; then printf '%s\n' "$code" > "$UNIT/exit"; exit "$code"; fi
done
sha256sum -c "$BUILD/source-before.sha256" > "$UNIT/source-after.log"
printf '0\n' > "$UNIT/exit"
