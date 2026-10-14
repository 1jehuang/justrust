#!/usr/bin/env bash
# Train the PGO-instrumented compiler (./build-jr.sh pgo-gen) on what agents
# actually compile, then merge the profile for ./build-jr.sh pgo-use.
#
# Upstream trains on rustc-perf (many small crates, check/debug/opt). justrust
# trains on its own workloads: the Desktop UI crate (118k lines of gpui code,
# macro heavy) and its ~740 dependencies, cold and incremental, check and
# test, plus justrust itself. Sources are the pinned justrust bench snapshots
# (bench/suite.py prepare), so the profile is reproducible.
#
# Usage: ./pgo-train.sh   (JUSTRUST_REPO defaults to the monorepo root)
# Output: build/pgo/rustc.profdata
# JR_TRAIN_ONLY=1 JR_TRAIN_RUSTC=<rustc> runs only the workloads with another
# compiler (bolt.sh uses it to train a BOLT-instrumented compiler).
set -euo pipefail
cd "$(dirname "$0")"
root=$PWD
jr=${JUSTRUST_REPO:-$(cd ../.. && pwd)}
sfx=""; [ "${JR_STAGE:-1}" = 1 ] || sfx=-stage$JR_STAGE
rustc=${JR_TRAIN_RUSTC:-$root/build/jr/pgo-gen$sfx/bin/rustc}
raw=$root/build/pgo/raw
work=$root/build/pgo/work
profdata=$root/build/host/ci-llvm/bin/llvm-profdata
[ -x "$rustc" ] || { echo "no instrumented compiler at $rustc: run ./build-jr.sh pgo-gen" >&2; exit 2; }
[ -x "$profdata" ] || profdata=$(ls "$root"/build/host/stage0/lib/rustlib/*/bin/llvm-profdata | head -1)

# Real cargo of the pinned toolchain (not the justrust shim on PATH).
cargo=${JR_PGO_CARGO:-$HOME/.justrust/toolchains/nightly-2026-10-08/bin/cargo}
snap() {
  python3 - "$jr" "$1" <<'EOF'
import sys
sys.path.insert(0, sys.argv[1] + "/bench")
import suite
root = suite.prepare(sys.argv[2])
print(root / suite.SNAPSHOTS[sys.argv[2]]["cwd"])
EOF
}
# The snapshots are shared with justrust's bench scripts: hold their lock
# (bench/suite.py bench_lock) for the whole training run, and tell the
# python helpers we hold it so they do not wait on us. Re-exec under
# `flock` rather than `exec 9>` + `flock 9`: an inherited fd 9 would let a
# daemonized grandchild keep the lock after this script exits.
mkdir -p "$HOME/.justrust/bench/work"
if [ -z "${BENCH_LOCK_HELD:-}" ]; then
  BENCH_LOCK_HELD=1 exec flock -o "$HOME/.justrust/bench/work/.bench.lock" "$0" "$@"
fi
desktop=$(snap desktop)
justrust_src=$(snap justrust)
rel=crates/jcode-desktop-ui/src/fps_counter.rs
fps=$desktop/$rel
# bench/suite.py keeps the as-extracted file here. Do not run this while a
# justrust benchmark is using the same snapshot.
pristine=$(dirname "$desktop")/.pristine/jcode-desktop/$rel
[ -f "$pristine" ] || { mkdir -p "$(dirname "$pristine")"; cp "$fps" "$pristine"; }
trap 'cp "$pristine" "$fps"' EXIT

# Plain cargo with the instrumented rustc and no justrust caches, so every
# crate really goes through the compiler. Same flags as justrust's defaults
# matter less than covering the same compiler paths (frontend, incremental,
# LLVM codegen at opt-level 0, cranelift is a separate backend .so).
export RUSTC=$rustc
export CARGO_INCREMENTAL=1
export JUSTRUST_DEPCACHE=0 JUSTRUST_BUILD_SCRIPT_CACHE=0 JUSTRUST_PEER_CACHE=0 JUSTRUST_REMOTE_CACHE=0 JUSTRUST_REMOTE=0
export JUSTRUST_TOOLCHAIN=system
rm -rf "$work"
mkdir -p "$work" "$raw"
step() { echo "pgo-train: $*" >&2; }
edit() { # one-line body edit, unique per call
  python3 - "$fps" "$1" <<'EOF'
import sys
p, n = sys.argv[1], sys.argv[2]
s = open(p).read()
a = "            let current = snapshot();\n"
assert s.count(a) == 1
open(p, "w").write(s.replace(a, a + f"            std::hint::black_box({n}u64);\n", 1))
EOF
}

t0=$(date +%s)
cd "$desktop"
step "desktop: cold check"
CARGO_TARGET_DIR=$work/desktop "$cargo" check -p jcode-desktop-ui -q
step "desktop: cold test build"
CARGO_TARGET_DIR=$work/desktop "$cargo" test -p jcode-desktop-ui --lib --no-run -q
for i in 1 2 3; do
  cp "$pristine" "$fps"; edit $i
  step "desktop: incremental check $i"
  CARGO_TARGET_DIR=$work/desktop "$cargo" check -p jcode-desktop-ui -q
  step "desktop: incremental test build $i"
  CARGO_TARGET_DIR=$work/desktop "$cargo" test -p jcode-desktop-ui --lib --no-run -q
done
cp "$pristine" "$fps"
cd "$justrust_src"
step "justrust: cold test build"
CARGO_TARGET_DIR=$work/justrust "$cargo" test --no-run -q
step "justrust: cold release build"
CARGO_TARGET_DIR=$work/justrust "$cargo" build --release -q
echo "pgo-train: workloads took $(( $(date +%s) - t0 ))s" >&2
if [ -n "${JR_TRAIN_ONLY:-}" ]; then rm -rf "$work"; exit 0; fi
echo "pgo-train: $(ls "$raw" | wc -l) profraw files, $(du -sh "$raw" | cut -f1)" >&2
"$profdata" merge -o "$root/build/pgo/rustc.profdata" "$raw"
ls -la "$root/build/pgo/rustc.profdata"
rm -rf "$work"
