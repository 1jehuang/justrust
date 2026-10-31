#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
BUILD=$ROOT/candidate-build-v44
UNIT=$ROOT/candidate-unit-v44
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
cd /home/jeremy/justrust
# Run only after coordinator verifies explicit source-ready hashes.
git diff --exit-code HEAD -- toolchain/resident/Cargo.lock toolchain/resident/crates/hir-ty-codegen/Cargo.toml toolchain/resident/crates/hir-ty-codegen/patch.rs
mkdir -p "$BUILD" "$UNIT"
cd toolchain/resident
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$BUILD/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
sha256sum Cargo.lock > "$BUILD/lock-before.sha256"
find crates vendor -type f -print0 | sort -z | xargs -0 sha256sum > "$BUILD/all-source-before.sha256"
trap 'code=$?; printf "%s\n" "$code" > "$BUILD/orchestration-exit"' EXIT
run_gate() {
 local name=$1
 shift
 set +e
 "$@" > "$name.log" 2>&1
 local code=$?
 set -e
 printf '%s\n' "$code" > "$name.exit"
 if [ "$code" != 0 ]; then exit "$code"; fi
}
run_gate "$BUILD/check" justrust check --release --locked -p ra-jit --tests -p hir-ty-codegen -j 6
run_gate "$BUILD/compile" justrust build --release --locked -p ra-jit --bin jr-engine -j 6
mkdir -p "$ROOT/candidate-v44-bin"
cp --reflink=auto "$CARGO_TARGET_DIR/release/jr-engine" "$ROOT/candidate-v44-bin/jr-engine"
"$ROOT/candidate-v44-bin/jr-engine" --version > "$BUILD/engine-version.txt"
sha256sum "$ROOT/candidate-v44-bin/jr-engine" > "$BUILD/engine-identity.sha256"
grep -Eq 'build resident-v1-[0-9a-f]{64}\)$' "$BUILD/engine-version.txt"
export JUSTRUST_HOME="$UNIT/justrust-home"
run_gate "$UNIT/codegen-native" justrust test --release --locked -p hir-ty-codegen -j 6
run_gate "$UNIT/full-native" justrust test --release --locked -p ra-jit --bin jr-engine -j 6
sha256sum -c "$BUILD/all-source-before.sha256" > "$UNIT/all-source-after.log"
sha256sum -c "$BUILD/lock-before.sha256" > "$UNIT/lock-after.log"
printf '0\n' > "$UNIT/exit"
