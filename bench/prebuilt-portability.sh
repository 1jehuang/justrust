#!/usr/bin/env bash
# Cross-user key portability: build the same snapshot under two different
# cargo home paths (same registry), each with its own depcache, and compare
# the per-crate base keys. Untimed.
# usage: bench/prebuilt-portability.sh <snapshot cwd> <out dir> [cargo args...]
set -u
cwd=$1; out=$2; shift 2
args=("$@"); [ ${#args[@]} -eq 0 ] && args=(check)
mkdir -p "$out"
for h in A B; do
  mkdir -p "$out/home$h/cargo"
  ln -sfn "$HOME/.cargo/registry" "$out/home$h/cargo/registry"
  [ -d "$HOME/.cargo/git" ] && ln -sfn "$HOME/.cargo/git" "$out/home$h/cargo/git"
  ( cd "$cwd" && CARGO_HOME="$out/home$h/cargo" JUSTRUST_DEPCACHE_DIR="$out/dc$h" \
    JUSTRUST_DEPCACHE_EXPLAIN="$out/ex$h" CARGO_TARGET_DIR="$out/t$h" JUSTRUST_REMOTE=0 \
    JUSTRUST_QUIET=1 JUSTRUST_TOOLCHAIN="${TC:-nightly-2026-10-08}" \
    justrust "${args[@]}" --offline >"$out/log$h" 2>&1 )
  echo "home$h exit $?"
done
a=$(ls "$out/exA" | wc -l); b=$(ls "$out/exB" | wc -l)
same=$(comm -12 <(ls "$out/exA" | sort) <(ls "$out/exB" | sort) | wc -l)
echo "units A=$a B=$b equal base keys=$same"
