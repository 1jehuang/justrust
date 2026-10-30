#!/usr/bin/env bash
# Existing incremental lint fixtures, unchanged, through a frozen matching sysroot.
set -euo pipefail
root=$(cd "$(dirname "$0")/../toolchain/rustc" && pwd)
sysroot=$(realpath "${1:?usage: $0 MATCHING_SYSROOT NEW_OUTPUT_DIRECTORY}")
out=$(realpath -m "${2:?usage: $0 MATCHING_SYSROOT NEW_OUTPUT_DIRECTORY}")
mkdir "$out"
compiletest="$root/build/host/stage1-tools-bin/compiletest"
sha256sum "$compiletest" "$sysroot/bin/rustc" "$sysroot"/lib/librustc_driver*.so > "$out/identity-before.sha256"
"$sysroot/bin/rustc" -vV > "$out/compiler-version.txt"
for mode in no yes; do
  "$compiletest" \
    --compile-lib-path "$sysroot/lib" \
    --run-lib-path "$sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib" \
    --rustc-path "$sysroot/bin/rustc" \
    --src-root "$root" --src-test-suite-root "$root/tests/incremental" \
    --build-root "$out" --build-test-suite-root "$out/$mode" \
    --sysroot-base "$sysroot" --stage 1 --stage-id stage1-x86_64-unknown-linux-gnu \
    --mode incremental --suite incremental \
    --host x86_64-unknown-linux-gnu --target x86_64-unknown-linux-gnu \
    --channel nightly --nightly-branch master --git-merge-commit-email rust-lang-team@rust-lang.org \
    --minicore-path "$root/tests/auxiliary/minicore.rs" \
    --llvm-components x86 --llvm-version 23.1.3 \
    --python python3 --cc cc --cxx c++ --cflags '' --cxxflags '' --jobs 3 \
    --target-rustcflags="-Zjr-empty-lint-level-fast-path=$mode" \
    --target-rustcflags=-Zjr-cache-small-query-values=no \
    --target-rustcflags=-Zjr-cache-associated-items=no \
    --target-rustcflags=-Zincremental-verify-ich \
    --target-rustcflags="-Lnative=$root/build/x86_64-unknown-linux-gnu/native/rust-test-helpers" \
    lint-unused-features hashes/delayed_lints cache-lint-expectation \
    2>&1 | tee "$out/$mode.log"
done
sha256sum "$compiletest" "$sysroot/bin/rustc" "$sysroot"/lib/librustc_driver*.so > "$out/identity-after.sha256"
cmp "$out/identity-before.sha256" "$out/identity-after.sha256"
