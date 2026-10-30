#!/usr/bin/env bash
# Run unchanged native compiler regression fixtures with an already built,
# matching sysroot. Avoid bootstrap rebuilding identities during concurrent work.
set -euo pipefail
root=$(cd "$(dirname "$0")/../toolchain/rustc" && pwd)
sysroot=$(realpath "${1:?usage: $0 MATCHING_SYSROOT NEW_OUTPUT_DIRECTORY}")
out=$(realpath -m "${2:?usage: $0 MATCHING_SYSROOT NEW_OUTPUT_DIRECTORY}")
mkdir "$out"
compiletest="$root/build/host/stage1-tools-bin/compiletest"
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
    --python python3 --cc cc --cxx c++ --cflags '' --cxxflags '' --jobs 4 \
    --target-rustcflags="-Zjr-cache-associated-items=$mode" \
    --target-rustcflags=-Zincremental-verify-ich \
    --target-rustcflags="-Lnative=$root/build/x86_64-unknown-linux-gnu/native/rust-test-helpers" \
    explicit_supertraits_containing_assoc_item issue-100521-change-struct-name-assocty \
    hashes/inherent_impls hashes/trait_impls hashes/trait_defs 2>&1 | tee "$out/$mode.log"
done
