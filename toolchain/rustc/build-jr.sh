#!/usr/bin/env bash
# Build justrust's rustc fork (stage 1 compiler + std) with prebuilt CI LLVM.
#
# Usage: ./build-jr.sh [dev|release|pgo-gen|pgo-use] [extra x.py args]
#   dev      incremental, 256 codegen units: fast rebuilds while changing the
#            compiler, but the compiler itself runs ~30% slower (measured).
#   release  the official dist settings except PGO/BOLT: thin LTO of
#            rustc_driver, 1 codegen unit, not incremental.
#   pgo-gen  release + PGO instrumentation (writes build/pgo/raw/*.profraw
#            when the compiler runs). Train it with ./pgo-train.sh.
#   pgo-use  release + PGO from build/pgo/rustc.profdata (JR_PGO_PROFILE to
#            override).
#   pgo-bolt-base  pgo-use linked for BOLT; ./bolt.sh turns it into
#            build/jr/pgo-bolt, what justrust benchmarks and ships.
# Output: build/host/stage1 (bin/rustc, lib/rustlib/...), also saved as
# build/jr/<mode> (a copy that later builds do not overwrite, so modes can be
# benchmarked against each other).
#
# LLVM comes from Rust CI and is already PGO+BOLT optimized by upstream. Only
# rustc_driver (the Rust half of the compiler) needs our own PGO.
set -euo pipefail
cd "$(dirname "$0")"
mode=${1:-release}
[ $# -gt 0 ] && shift
root=$PWD
release='incremental = false\nlto = "thin"\ncodegen-units = 1\n'
pgo=''
case $mode in
  dev) extra='incremental = true\nlto = "thin-local"\n' ;;
  release) extra=$release ;;
  pgo-gen)
    rm -rf build/pgo/raw
    mkdir -p build/pgo/raw
    extra=$release
    pgo="generate = \"$root/build/pgo/raw\"" ;;
  pgo-use|pgo-bolt-base)
    prof=${JR_PGO_PROFILE:-$root/build/pgo/rustc.profdata}
    [ -f "$prof" ] || { echo "no PGO profile at $prof: run ./build-jr.sh pgo-gen and ./pgo-train.sh" >&2; exit 2; }
    extra=$release
    pgo="use = \"$prof\""
    # pgo-bolt-base: same, linked with relocations kept (-Wl,-q) so
    # ./bolt.sh can rewrite librustc_driver.
    if [ "$mode" = pgo-bolt-base ]; then set -- --enable-bolt-settings "$@"; fi ;;
  *) echo "usage: $0 [dev|release|pgo-gen|pgo-use|pgo-bolt-base]" >&2; exit 2 ;;
esac
cp justrust.bootstrap.toml bootstrap.toml
# Mode settings go into the [rust] table of the base config.
python3 - "$extra" <<'EOF'
import sys
s = open("bootstrap.toml").read()
assert s.count("\n[rust]\n") == 1
open("bootstrap.toml", "w").write(s.replace("\n[rust]\n", "\n[rust]\n" + sys.argv[1].encode().decode("unicode_escape"), 1))
EOF
if [ -n "$pgo" ]; then printf '\n[pgo.rustc]\n%s\n' "$pgo" >> bootstrap.toml; fi
# JR_STAGE=2: the compiler is compiled by our own stage 1 (as upstream's
# dist), not by the stage 0 beta. Output dir gets a -stage2 suffix.
stage=${JR_STAGE:-1}
# JR_TOOLS=1 also builds rustdoc and clippy against this driver (what
# `justrust toolchain jr-install` packages; they must come from the same
# build as librustc_driver, which BOLT later rewrites in place).
tools=()
[ -n "${JR_TOOLS:-}" ] && tools=(src/tools/rustdoc src/tools/clippy)
./x build --stage "$stage" library compiler/rustc "${tools[@]}" "$@"
# Keep a copy per mode. stage1 is a sysroot of real files plus symlinks into
# the source tree (rust-src), so a copy that keeps symlinks is ~1 GB.
out=build/jr/$mode
[ "$stage" = 1 ] || out=$out-stage$stage
rm -rf "$out"
mkdir -p build/jr
cp -a "build/host/stage$stage/." "$out"
echo "build-jr: $mode -> $out/bin/rustc"
