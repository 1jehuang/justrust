#!/usr/bin/env bash
# BOLT-optimize librustc_driver of a PGO build, as upstream's opt-dist does
# for the official toolchain (instrumentation mode, so no `perf` needed).
#
#   ./build-jr.sh pgo-bolt-base   # PGO build linked with -Wl,-q
#   ./bolt.sh                     # instrument, train, optimize
#
# Needs llvm-bolt and merge-fdata (LLVM_BOLT_DIR, default build/bolt/bin).
# Build them once from the in-tree LLVM:
#   cmake -G Ninja src/llvm-project/llvm -DLLVM_ENABLE_PROJECTS=bolt \
#     -DLLVM_TARGETS_TO_BUILD=X86 -DCMAKE_BUILD_TYPE=Release
#   ninja llvm-bolt merge-fdata bolt_rt
# and copy bin/{llvm-bolt,merge-fdata} to build/bolt/bin and
# lib/libbolt_rt_*.a to build/bolt/lib (llvm-bolt finds its instrumentation
# runtime in ../lib).
# LLVM itself comes from Rust CI already BOLT-optimized, so only the driver
# is processed here.
# Output: build/jr/pgo-bolt (JR_STAGE=2: build/jr/pgo-bolt-stage2).
set -euo pipefail
cd "$(dirname "$0")"
root=$PWD
sfx=""; [ "${JR_STAGE:-1}" = 1 ] || sfx=-stage$JR_STAGE
bolt_dir=${LLVM_BOLT_DIR:-$root/build/bolt/bin}
base=$root/build/jr/pgo-bolt-base$sfx
out=$root/build/jr/pgo-bolt$sfx
prof=$root/build/pgo/bolt
[ -x "$bolt_dir/llvm-bolt" ] || { echo "no llvm-bolt in $bolt_dir" >&2; exit 2; }
[ -d "$base" ] || { echo "no $base: run ./build-jr.sh pgo-bolt-base" >&2; exit 2; }

rm -rf "$out" "$prof"
mkdir -p "$prof"
cp -a "$base/." "$out"
driver=$(ls "$out"/lib/librustc_driver-*.so)
orig=$prof/librustc_driver.orig.so
cp "$driver" "$orig"

echo "bolt: instrumenting $(basename "$driver")" >&2
"$bolt_dir/llvm-bolt" -instrument "$orig" \
  --instrumentation-file="$prof/prof" --instrumentation-file-append-pid \
  -o "$driver"

echo "bolt: training" >&2
JR_TRAIN_ONLY=1 JR_TRAIN_RUSTC=$out/bin/rustc ./pgo-train.sh
"$bolt_dir/merge-fdata" "$prof"/prof.*.fdata > "$prof/merged.fdata"
rm -f "$prof"/prof.*.fdata

echo "bolt: optimizing" >&2
"$bolt_dir/llvm-bolt" "$orig" -data "$prof/merged.fdata" -o "$driver" \
  -reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions \
  -split-strategy=cdsplit -split-all-cold -jump-tables=move -icf=all \
  -update-debug-sections -dyno-stats 2> "$prof/bolt.log"
rm -f "$orig"
echo "bolt: done -> $out/bin/rustc" >&2
