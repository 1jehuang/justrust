#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
BUILD=$ROOT/candidate-build-v29b
UNIT=$ROOT/candidate-unit-v29b
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
mkdir -p "$BUILD" "$UNIT" "$BUILD/frozen-source"
cd /home/jeremy/justrust/toolchain/resident
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$BUILD/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
files=(Cargo.lock crates/hir-ty-codegen/patch.rs crates/hir-ty-codegen/Cargo.toml crates/ra-jit/src/outline.rs vendor/rust-analyzer/crates/hir/src/diagnostics.rs vendor/rust-analyzer/crates/hir/src/lib.rs vendor/rust-analyzer/crates/ide-diagnostics/src/lib.rs vendor/rust-analyzer/crates/ide-diagnostics/src/tests.rs vendor/rust-analyzer/crates/hir-def/src/signatures.rs crates/hir-ty-codegen/mod.rs crates/ra-jit/src/main.rs crates/ra-jit/src/diagnostic_hash.rs crates/ra-jit/src/jr_engine.rs crates/ra-jit/Cargo.toml crates/ra-jit/build.rs crates/ra-jit/src/cargo/mod.rs crates/ra-jit/src/cargo/whole.rs crates/ra-jit/src/cargo/cli.rs crates/ra-jit/src/cargo/metadata.rs crates/blaze-metadata/src/lib.rs)
sha256sum "${files[@]}" > "$BUILD/source-before.sha256"
find crates vendor -type f -print0 | sort -z | xargs -0 sha256sum > "$BUILD/all-source-before.sha256"
for file in "${files[@]}"; do mkdir -p "$BUILD/frozen-source/$(dirname "$file")"; cp "$file" "$BUILD/frozen-source/$file"; done
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
run_gate "$BUILD/check" justrust check --release --locked -p hir-ty-codegen -p ra-jit --tests -j 6
run_gate "$BUILD/compile" justrust build --release --locked -p ra-jit --bin jr-engine -j 6
sha256sum -c "$BUILD/source-before.sha256" > "$BUILD/source-after.log"
sha256sum -c "$BUILD/all-source-before.sha256" > "$BUILD/all-source-after.log"
mkdir -p "$ROOT/candidate-v29b-bin"
cp --reflink=auto "$CARGO_TARGET_DIR/release/jr-engine" "$ROOT/candidate-v29b-bin/jr-engine"
"$ROOT/candidate-v29b-bin/jr-engine" --version > "$BUILD/engine-version.txt"
sha256sum "$ROOT/candidate-v29b-bin/jr-engine" > "$BUILD/engine-identity.sha256"
grep -Eq 'build resident-v1-[0-9a-f]{64}\)$' "$BUILD/engine-version.txt"
printf '0\n' > "$BUILD/exit"
export JUSTRUST_HOME="$UNIT/justrust-home"
run_gate "$UNIT/mark" justrust test --release --locked -p hir-ty-codegen -j 6 -- nondebug_ --nocapture
run_gate "$UNIT/codegen-full" justrust test --release --locked -p hir-ty-codegen -j 6
run_gate "$UNIT/full-native" justrust test --release --locked -p ra-jit --bin jr-engine -j 6
sha256sum -c "$BUILD/source-before.sha256" > "$UNIT/source-after.log"
sha256sum -c "$BUILD/all-source-before.sha256" > "$UNIT/all-source-after.log"
printf '0\n' > "$UNIT/exit"
